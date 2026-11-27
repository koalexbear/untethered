use crate::{
    Msg,
    media::{LocalFile, MAX_FILES},
};
use wasm_bindgen::{closure::Closure, prelude::*};
use web_sys::FileList;

#[wasm_bindgen(module = "/web/files.js")]
extern "C" {
    fn install_files(files: JsValue, hover: JsValue);
    pub fn pick(room: &str);
    pub fn pick_avatar(request: &str);
    #[wasm_bindgen(catch)]
    pub async fn thumbnail(
        file: &web_sys::Blob,
        width: u32,
        height: u32,
    ) -> Result<js_sys::Uint8Array, JsValue>;
    /// Offer bytes the engine fetched as a download.
    pub fn save_bytes(bytes: &[u8], name: &str);
}
pub fn listen() -> iced::Task<Msg> {
    let (tx, rx) = async_channel::unbounded();
    let hovered = tx.clone();
    let files = Closure::<dyn FnMut(String, FileList)>::new(move |room, files: FileList| {
        let files = (0..files.length())
            .take(MAX_FILES + 1)
            .filter_map(|i| files.item(i))
            .map(|file| LocalFile {
                name: file.name(),
                size: file.size() as u64,
                file,
            })
            .collect();
        let _ = tx.try_send(Msg::Files(room, files));
    });
    let hover = Closure::<dyn FnMut(bool)>::new(move |v| {
        let _ = hovered.try_send(Msg::HoverFiles(v));
    });
    install_files(files.into_js_value(), hover.into_js_value());
    iced::Task::run(rx, |m| m)
}
