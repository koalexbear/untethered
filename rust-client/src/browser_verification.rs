use super::{App, Msg};
use iced::Task;
use wasm_bindgen::{closure::Closure, prelude::*};

#[wasm_bindgen(module = "/web/verification.js")]
extern "C" {
    fn install(submit: JsValue);
    fn render(show: bool, user: &str, enabled: bool, busy: bool, status: &str);
}

pub fn listen() -> Task<Msg> {
    let (tx, rx) = async_channel::bounded(1);
    let submit = Closure::<dyn FnMut(String)>::new(move |s| {
        let _ = tx.try_send(s);
    });
    install(submit.into_js_value());
    Task::run(rx, Msg::SubmitWebRecovery)
}
pub fn sync(app: &App) {
    render(
        app.logged_in && !app.verified,
        &app.matrix_id,
        app.connected && !app.recovery_unavailable,
        app.verifying(),
        app.verification_feedback(),
    );
}
