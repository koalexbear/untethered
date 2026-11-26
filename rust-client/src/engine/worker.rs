//! The matrix-sdk engine in a dedicated Web Worker, so sync, decryption and
//! IndexedDB work stay off the page's thread, where Iced draws (gomuks ran in
//! a worker too).
//!
//! The worker runs the page's own wasm module: the page posts it the compiled
//! module and the URL of the wasm-bindgen glue, `web/engine-worker.js` loads
//! them, and [`untethered_engine_worker`] starts the engine. From then on
//! every message is an object `{json, bytes?, file?}`: `json` is a
//! [`ToWorker`] or [`FromWorker`] in JSON, `bytes` an attachment's bytes, and
//! `file` a picked `File`.
//!
//! The page keeps what a worker can't: the session JSON in `localStorage`,
//! the one-tab lock, image decoding and download links.
use super::{Command, Event, Failure, Transfer, rt};
use crate::media::{LocalFile, Media, PreviewRef};
use async_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent, Worker, WorkerOptions, WorkerType};

const WAITING: &str = "Another Untethered tab is using this Matrix device. Use that tab, or close it and this one will connect automatically.";
const START: &str =
    "The matrix-sdk engine couldn’t start in a browser worker. Reload this tab to retry.";
const CRASHED: &str = "The matrix-sdk engine’s browser worker stopped. Reload this tab to retry.";

/// The page to the worker.
#[derive(Serialize, Deserialize)]
enum ToWorker {
    Start {
        session: Option<String>,
    },
    Command(Command),
    Inline {
        request: u64,
        source: PreviewRef,
    },
    /// With the picked `File` in `file`.
    Upload {
        id: i64,
        name: String,
        size: u64,
        encrypt: bool,
        limit: u64,
    },
    /// The App dropped the upload's lease: it was cancelled, or it's done.
    Release {
        id: i64,
    },
    Download {
        id: i64,
        media: Media,
    },
}

/// The worker to the page.
#[derive(Serialize, Deserialize)]
enum FromWorker {
    /// The worker listens; nothing the page sent before this arrived.
    Ready,
    Event(Event),
    /// Save (or with `None`, remove) the session JSON.
    Session(Option<String>),
    /// An inline image's bytes in `bytes`, or no image.
    Inline {
        request: u64,
    },
    /// A download's bytes in `bytes`.
    Download {
        id: i64,
        name: String,
    },
    Stopped(String),
    /// Debug builds: an entry of the QA event log (`qa.rs`). The page ignores
    /// it; browser fixtures watch the worker's messages for it.
    #[cfg(debug_assertions)]
    Qa(serde_json::Value),
}

#[wasm_bindgen]
extern "C" {
    /// The URL of this wasm-bindgen glue module, for the worker to load.
    #[wasm_bindgen(thread_local_v2, js_namespace = ["import", "meta"], js_name = url)]
    static GLUE: String;
}

fn message(json: &impl Serialize, extra: Option<(&str, &JsValue)>) -> Option<js_sys::Object> {
    let object = js_sys::Object::new();
    let text = serde_json::to_string(json).ok()?;
    js_sys::Reflect::set(&object, &"json".into(), &text.into()).ok()?;
    if let Some((key, value)) = extra {
        js_sys::Reflect::set(&object, &key.into(), value).ok()?;
    }
    Some(object)
}

fn field(data: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(data, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

fn read<T: for<'de> Deserialize<'de>>(data: &JsValue) -> Option<T> {
    serde_json::from_str(&field(data, "json").as_string()?).ok()
}

fn bytes(data: &JsValue) -> Option<Vec<u8>> {
    field(data, "bytes")
        .dyn_ref::<js_sys::Uint8Array>()
        .map(js_sys::Uint8Array::to_vec)
}

fn post_to(
    worker: &Worker,
    json: &ToWorker,
    extra: Option<(&str, &JsValue)>,
) -> Result<(), String> {
    let message = message(json, extra).ok_or(CRASHED)?;
    worker
        .post_message(&message)
        .map_err(|_| CRASHED.to_owned())
}

/// The page side: hold the stores for this tab, start the worker, and relay.
pub async fn run(
    commands: Receiver<Command>,
    backend: Receiver<crate::backend::Command>,
    events: Sender<Event>,
) -> Result<(), String> {
    let waiting = events.clone();
    super::web::hold_profile(move || {
        let _ = waiting.try_send(Event::Status(WAITING.into()));
    })
    .await?;
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker =
        Worker::new_with_options("./engine-worker.js", &options).map_err(|_| START.to_owned())?;
    let (inbox, incoming) = async_channel::unbounded::<Option<JsValue>>();
    let errors = inbox.clone();
    let receive = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let _ = inbox.try_send(Some(e.data()));
    });
    let failed = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |_| {
        let _ = errors.try_send(None);
    });
    worker.set_onmessage(Some(receive.as_ref().unchecked_ref()));
    worker.set_onerror(Some(failed.as_ref().unchecked_ref()));
    let boot = js_sys::Object::new();
    let glue = GLUE.with(String::clone);
    let started = js_sys::Reflect::set(&boot, &"glue".into(), &glue.into())
        .and_then(|_| js_sys::Reflect::set(&boot, &"module".into(), &wasm_bindgen::module()))
        .and_then(|_| worker.post_message(&boot));
    let result = match started {
        Ok(()) => relay(&worker, commands, backend, events, incoming).await,
        Err(_) => Err(START.to_owned()),
    };
    worker.set_onmessage(None);
    worker.set_onerror(None);
    worker.terminate();
    result
}

async fn relay(
    worker: &Worker,
    commands: Receiver<Command>,
    backend: Receiver<crate::backend::Command>,
    events: Sender<Event>,
    incoming: Receiver<Option<JsValue>>,
) -> Result<(), String> {
    // Commands wait in their channels until the worker listens.
    loop {
        match incoming.recv().await {
            Ok(Some(data)) => match read::<FromWorker>(&data) {
                Some(FromWorker::Ready) => break,
                Some(FromWorker::Stopped(why)) => return Err(why),
                _ => {}
            },
            Ok(None) => return Err(START.to_owned()),
            Err(_) => return Ok(()),
        }
    }
    let session = super::web::saved_session();
    post_to(worker, &ToWorker::Start { session }, None)?;
    let mut previews: HashMap<u64, Sender<Option<iced::widget::image::Handle>>> = HashMap::new();
    let mut next = 0;
    loop {
        tokio::select! {
            message = incoming.recv() => {
                let data = match message {
                    Ok(Some(data)) => data,
                    Ok(None) => return Err(CRASHED.to_owned()),
                    Err(_) => return Ok(()),
                };
                match read::<FromWorker>(&data) {
                    Some(FromWorker::Event(event)) => {
                        if events.send(event).await.is_err() {
                            return Ok(());
                        }
                    }
                    Some(FromWorker::Session(session)) => {
                        super::web::persist_session(session.as_deref());
                    }
                    Some(FromWorker::Inline { request }) => {
                        if let Some(result) = previews.remove(&request) {
                            let image = bytes(&data);
                            rt::spawn(async move {
                                let handle = match image {
                                    Some(image) => crate::preview::inline(image).await,
                                    None => None,
                                };
                                let _ = result.send(handle).await;
                            });
                        }
                    }
                    Some(FromWorker::Download { id, name }) => {
                        let result = match bytes(&data) {
                            Some(file) => {
                                crate::web_files::save_bytes(&file, &name);
                                Ok(())
                            }
                            None => Err(Failure::Failed),
                        };
                        let _ = events.send(Event::Done { id, result }).await;
                    }
                    Some(FromWorker::Stopped(why)) => return Err(why),
                    #[cfg(debug_assertions)]
                    Some(FromWorker::Qa(_)) => {}
                    Some(FromWorker::Ready) | None => {}
                }
            }
            command = commands.recv() => match command {
                Ok(command) => post_to(worker, &ToWorker::Command(command), None)?,
                Err(_) => return Ok(()),
            },
            transfer = backend.recv() => match transfer {
                Ok(crate::backend::Command::Inline { source, result }) => {
                    next += 1;
                    previews.insert(next, result);
                    post_to(worker, &ToWorker::Inline { request: next, source }, None)?;
                }
                Ok(crate::backend::Command::Upload { id, cancel, file, encrypt, limit }) => {
                    let upload = ToWorker::Upload { id, name: file.name, size: file.size, encrypt, limit };
                    post_to(worker, &upload, Some(("file", &file.file)))?;
                    let worker = worker.clone();
                    rt::spawn(async move {
                        let _ = cancel.recv().await;
                        let _ = post_to(&worker, &ToWorker::Release { id }, None);
                    });
                }
                Ok(crate::backend::Command::Download { id, media }) => {
                    post_to(worker, &ToWorker::Download { id, media }, None)?;
                }
                Err(_) => return Ok(()),
            },
        }
    }
}

fn post(json: &FromWorker, bytes: Option<Vec<u8>>) {
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let array = bytes.map(|b| js_sys::Uint8Array::from(b.as_slice()));
    let Some(message) = message(json, array.as_ref().map(|a| ("bytes", a.as_ref()))) else {
        return;
    };
    // Hand attachment bytes over instead of copying them.
    let _ = match &array {
        Some(array) => {
            scope.post_message_with_transfer(&message, &js_sys::Array::of1(&array.buffer()))
        }
        None => scope.post_message(&message),
    };
}

/// Post a QA event log entry to the page (`qa.rs`).
#[cfg(debug_assertions)]
pub(super) fn qa(entry: serde_json::Value) {
    post(&FromWorker::Qa(entry), None);
}

fn persist(session: Option<&str>) {
    post(&FromWorker::Session(session.map(str::to_owned)), None);
}

/// The worker side, called by `web/engine-worker.js` once the module loaded.
#[wasm_bindgen]
pub fn untethered_engine_worker() {
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let (to_engine, commands) = async_channel::unbounded();
    let (to_transfers, transfers) = async_channel::unbounded();
    let (events, from_engine) = async_channel::unbounded::<Event>();
    let engine = RefCell::new(Some((commands, transfers, events)));
    let leases: Rc<RefCell<HashMap<i64, Sender<()>>>> = Rc::default();
    let receive = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let data = e.data();
        match read::<ToWorker>(&data) {
            Some(ToWorker::Start { session }) => {
                let Some((commands, transfers, events)) = engine.borrow_mut().take() else {
                    return;
                };
                rt::spawn(async move {
                    let profile = super::session::Profile::web(session, persist);
                    let opener = super::browser::system();
                    let result =
                        super::run_with(profile, opener, commands, transfers, events).await;
                    let why = result
                        .err()
                        .unwrap_or_else(|| "The matrix-sdk engine stopped.".into());
                    post(&FromWorker::Stopped(why), None);
                });
            }
            Some(ToWorker::Command(command)) => {
                let _ = to_engine.try_send(command);
            }
            Some(ToWorker::Inline { request, source }) => {
                let (result, answer) = async_channel::bounded(1);
                let _ = to_transfers.try_send(Transfer::Inline { source, result });
                rt::spawn(async move {
                    let image = answer.recv().await.ok().flatten();
                    post(&FromWorker::Inline { request }, image);
                });
            }
            Some(ToWorker::Upload {
                id,
                name,
                size,
                encrypt,
                limit,
            }) => {
                let Ok(file) = field(&data, "file").dyn_into::<web_sys::File>() else {
                    let failed = Event::Uploaded {
                        id,
                        result: Err(Failure::Failed),
                    };
                    return post(&FromWorker::Event(failed), None);
                };
                let (lease, cancel) = async_channel::bounded(1);
                leases.borrow_mut().insert(id, lease);
                let file = LocalFile { name, size, file };
                let upload = Transfer::Upload {
                    id,
                    cancel,
                    file,
                    encrypt,
                    limit,
                };
                let _ = to_transfers.try_send(upload);
            }
            Some(ToWorker::Release { id }) => {
                // Dropping the lease cancels the upload if it's still running.
                leases.borrow_mut().remove(&id);
            }
            Some(ToWorker::Download { id, media }) => {
                let (result, answer) = async_channel::bounded(1);
                let name = media.name.clone();
                let _ = to_transfers.try_send(Transfer::Download { media, result });
                rt::spawn(async move {
                    match answer.recv().await {
                        Ok(Ok(file)) => post(&FromWorker::Download { id, name }, Some(file)),
                        Ok(Err(failure)) => post(
                            &FromWorker::Event(Event::Done {
                                id,
                                result: Err(failure),
                            }),
                            None,
                        ),
                        Err(_) => post(
                            &FromWorker::Event(Event::Done {
                                id,
                                result: Err(Failure::Failed),
                            }),
                            None,
                        ),
                    }
                });
            }
            None => {}
        }
    });
    scope.set_onmessage(Some(receive.as_ref().unchecked_ref()));
    // The worker lives as long as the page keeps it.
    receive.forget();
    rt::spawn(async move {
        while let Ok(event) = from_engine.recv().await {
            post(&FromWorker::Event(event), None);
        }
    });
    post(&FromWorker::Ready, None);
}
