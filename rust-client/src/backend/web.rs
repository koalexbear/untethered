use super::{Commands, Events};
use futures_util::{FutureExt, select};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

// Report a category, not arbitrary exception text (which can contain URLs
// and homeserver data). Go exit codes are from StartClientWithoutExit.
fn worker_failure(reason: &str, phase: &str) -> String {
    let s = reason.to_ascii_lowercase();
    let hint = if s.contains("nomodificationallowederror")
        || s.contains("another open access handle")
    {
        "The browser database is in use. Close older Untethered tabs or windows, then reload this one."
    } else if s.contains("quotaexceedederror") {
        "The browser ran out of storage. Free disk space, then reload."
    } else if s.contains("required opfs")
        || s.contains("too old for opfs")
        || s.contains("web locks api unavailable")
    {
        "This browser lacks the local file storage or locking features gomuks requires. Use a current browser on HTTPS or localhost."
    } else if s.contains("securityerror")
        || s.contains("notallowederror")
        || s.contains("access is denied")
    {
        "Browser storage access was denied. Allow site storage for this origin, then reload."
    } else if s.contains("fetch")
        || s.contains("mime")
        || s.contains("networkerror")
        || s.contains("compileerror")
    {
        "A backend file could not be loaded. Hard-refresh this tab to fetch the current web build."
    } else if s.contains("go process exited (10)") {
        "gomuks could not open the local database (exit 10)."
    } else if s.contains("go process exited (11)") {
        "gomuks could not read the stored account (exit 11)."
    } else if s.contains("go process exited (12)") {
        "gomuks could not start the saved Matrix session (exit 12). Check that the homeserver is reachable from this browser, then reload."
    } else if s.contains("runtimeerror")
        || s.contains("out of memory")
        || s.contains("go process exited (2)")
    {
        "The WebAssembly backend crashed. Reload this tab to retry."
    } else {
        "The browser backend exited unexpectedly. Reload this tab to retry."
    };
    format!("{hint} [web: {phase}]")
}

pub async fn run(commands: Commands, events: Events) -> Result<(), String> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options("./backend/gomuks-worker.js", &options)
        .map_err(|_| "Cannot start the embedded gomuks worker.")?;
    let (in_tx, incoming) = async_channel::unbounded();
    let errors = in_tx.clone();
    let receive = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let result = js_sys::JSON::stringify(&event.data())
            .ok()
            .and_then(|v| v.as_string())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .ok_or("Invalid worker frame.");
        let _ = in_tx.try_send(result);
    });
    let error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        let _ = errors.try_send(Ok(
            json!({"command":"wasm-connection","data":{"connected":false,"error":event.message()}}),
        ));
    });
    worker.set_onmessage(Some(receive.as_ref().unchecked_ref()));
    worker.set_onerror(Some(error.as_ref().unchecked_ref()));
    let result = async {
        let mut phase = "worker startup";
        loop {
            select! {
                message = incoming.recv().fuse() => {
                    let mut frame = message.map_err(|_| "Worker disconnected.")??;
                    if let Some(data) = frame["data"].as_str() {
                        frame["data"] = serde_json::from_str(data).map_err(|_| "Invalid worker data.")?;
                    }
                    match frame["command"].as_str() {
                        Some("profile-waiting") => phase = "waiting for another tab",
                        Some("profile-opening") => phase = "opening browser storage",
                        Some("wasm-connection") if frame["data"]["connected"] == false => {
                            return Err(worker_failure(frame["data"]["error"].as_str().unwrap_or(""),phase));
                        }
                        Some("wasm-connection") if frame["data"]["connected"] == true => phase = "loading Matrix session",
                        Some("client_state") => phase = "running",
                        _ => {}
                    }
                    if events.send(frame).await.is_err() { return Ok(()); }
                }
                command = commands.recv().fuse() => {
                    let Ok(command) = command else { return Ok(()) };
                    let command=match command {
                        super::Command::Rpc(value)=>value,
                        super::Command::Inline{source,result}=>{
                            let worker=worker.clone();
                            wasm_bindgen_futures::spawn_local(async move{
                                let image=async{
                                    let path=crate::media::media_path(&source.mxc)?;
                                    let blob=crate::web_files::inline_blob(&worker,&path,source.encrypted,crate::media::INLINE_BYTES as u32).await.ok()?;
                                    crate::preview::blob_thumbnail(&blob,crate::preview::INLINE_WIDTH,crate::preview::INLINE_HEIGHT).await
                                }.await;
                                let _=result.send(image).await;
                            });continue;
                        }
                        super::Command::Upload{id,file,encrypt,limit,cancel}=>{
                            let worker=worker.clone();let events=events.clone();
                            wasm_bindgen_futures::spawn_local(async move{
                                let stop=cancel.recv().fuse();futures_util::pin_mut!(stop);
                                let prepared=async {
                                    file.size<=limit && crate::web_files::post_upload(&worker,id as f64,&file.file,encrypt).await.is_ok()
                                }.fuse();futures_util::pin_mut!(prepared);
                                let ok=futures_util::select_biased!{
                                    _=stop=>{crate::web_files::cancel_upload(&worker,id as f64);return},
                                    ok=prepared=>ok
                                };
                                if !ok {
                                    let _=events.send(json!({"command":"error","request_id":id,"data":"Upload failed."})).await;
                                } else {
                                    // Worker responses use the normal RPC channel. On
                                    // completion the lease closes too; expired IDs are no-ops.
                                    let _=stop.await;
                                    crate::web_files::cancel_upload(&worker,id as f64);
                                }
                            });continue;
                        }
                        super::Command::Download{id,media}=>{
                            let worker=worker.clone();let events=events.clone();
                            wasm_bindgen_futures::spawn_local(async move{
                                let ok=if let Some(path)=crate::media::media_path(&media.mxc) {crate::web_files::save_media(&worker,&path,media.encrypted,&media.name).await.is_ok()}else{false};
                                let _=events.send(json!({"command":if ok {"response"}else{"error"},"request_id":id,"data":null})).await;
                            });continue;
                        }
                    };
                    let wire = json!({"command": command["command"], "request_id": command["request_id"], "data": command["data"].to_string()});
                    let message = js_sys::JSON::parse(&wire.to_string()).map_err(|_| "Invalid command.")?;
                    worker.post_message(&message).map_err(|_| "Cannot send to worker.")?;
                }
            }
        }
    }.await;
    worker.set_onmessage(None);
    worker.set_onerror(None);
    worker.terminate();
    result
}
