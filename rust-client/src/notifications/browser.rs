use crate::Msg;
use iced::Task;
use wasm_bindgen::{closure::Closure, prelude::*};

#[wasm_bindgen(module = "/web/notifications.js")]
extern "C" {
    pub fn permission() -> String;
    #[wasm_bindgen(catch)]
    fn request() -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch)]
    pub fn show(
        user: &str,
        room: &str,
        event: &str,
        title: &str,
        body: &str,
        silent: bool,
    ) -> Result<(), JsValue>;
    fn install_alerts(open: JsValue, failed: JsValue);
}
pub fn ask() -> Task<Msg> {
    // Initiate the permission request during the button action, not on startup
    // or after awaiting a worker/network response.
    let prompt = request();
    Task::perform(
        async move {
            if let Ok(p) = prompt {
                let _ = wasm_bindgen_futures::JsFuture::from(p).await;
            }
        },
        |()| Msg::BrowserPermissionDone,
    )
}
pub fn listen() -> Task<Msg> {
    let (tx, rx) = async_channel::bounded(4);
    let errors = tx.clone();
    let failed = Closure::<dyn FnMut()>::new(move || {
        let _ = errors.try_send(Msg::DesktopDelivery(false));
    });
    let open = Closure::<dyn FnMut(String, String)>::new(move |user, room| {
        let _ = tx.try_send(Msg::OpenBrowserAlert(user, room));
    });
    install_alerts(open.into_js_value(), failed.into_js_value());
    Task::run(rx, |m| m)
}
