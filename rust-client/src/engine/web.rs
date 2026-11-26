//! The browser side of the engine's profile: the session JSON the page keeps
//! in `localStorage`, and the lock that gives one tab the IndexedDB stores.
use wasm_bindgen::{JsValue, closure::Closure, prelude::wasm_bindgen};

/// The page's copy of `matrix-sdk/session.json`.
const SESSION: &str = "untethered.matrix-sdk.session";
/// The engine this page last chose with `?engine=`.
const ENGINE: &str = "untethered.engine";
/// Not gomuks's lock name: the two engines keep separate stores.
const LOCK: &str = "untethered:matrix-sdk-profile";

#[wasm_bindgen(module = "/web/engine.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn engine_lock(name: &str, waiting: &JsValue) -> Result<JsValue, JsValue>;
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn selected() -> bool {
    let asked = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|query| {
            url::form_urlencoded::parse(query.trim_start_matches('?').as_bytes())
                .find(|(key, _)| key == "engine")
                .map(|(_, value)| value.into_owned())
        })
        .filter(|choice| matches!(choice.as_str(), "matrix-sdk" | "gomuks"));
    let storage = storage();
    if let Some(choice) = asked {
        if let Some(storage) = &storage {
            let _ = storage.set_item(ENGINE, &choice);
        }
        return choice == "matrix-sdk";
    }
    !storage
        .and_then(|s| s.get_item(ENGINE).ok().flatten())
        .is_some_and(|choice| choice == "gomuks")
}

pub fn saved_session() -> Option<String> {
    storage()?.get_item(SESSION).ok()?
}

pub fn persist_session(session: Option<&str>) {
    let Some(storage) = storage() else {
        return;
    };
    let _ = match session {
        Some(value) => storage.set_item(SESSION, value),
        None => storage.remove_item(SESSION),
    };
}

/// Wait until this tab owns the stores; `waiting` runs if another tab does.
/// The lock is held until the page closes.
pub async fn hold_profile(waiting: impl FnMut() + 'static) -> Result<(), String> {
    let waiting = Closure::<dyn FnMut()>::new(waiting);
    let result = engine_lock(LOCK, waiting.as_ref()).await;
    // The callback runs at most once, before the lock is granted.
    drop(waiting);
    result.map(drop).map_err(|_| {
        "This browser can’t lock its storage for one tab. Use a current browser on HTTPS or localhost."
            .to_owned()
    })
}
