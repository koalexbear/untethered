use async_channel::{Receiver, Sender};
use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::run;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::run;

pub enum Command {
    #[cfg(target_os = "linux")]
    Voice {
        room: String,
        target: crate::desktop_call::Target,
        controls: crate::desktop_call::Controls,
    },
    Rpc(Value),
    Inline {
        source: crate::media::PreviewRef,
        result: Sender<Option<iced::widget::image::Handle>>,
    },
    Upload {
        id: i64,
        cancel: Receiver<()>,
        file: crate::media::LocalFile,
        encrypt: bool,
        limit: u64,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Download {
        id: i64,
        media: crate::media::Media,
        destination: std::path::PathBuf,
    },
    #[cfg(target_arch = "wasm32")]
    Download {
        id: i64,
        media: crate::media::Media,
    },
}
#[cfg(test)]
impl Command {
    pub fn into_rpc(self) -> Value {
        if let Self::Rpc(v) = self {
            v
        } else {
            panic!("expected RPC")
        }
    }
}
type Commands = Receiver<Command>;
type Events = Sender<Value>;
