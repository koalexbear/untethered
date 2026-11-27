//! What the App hands the engine besides commands: attachment transfers and
//! desktop calls, which need channels and local files.
use async_channel::{Receiver, Sender};

pub enum Command {
    #[cfg(target_os = "linux")]
    Voice {
        room: String,
        target: crate::desktop_call::Target,
        controls: crate::desktop_call::Controls,
    },
    Inline {
        source: crate::media::PreviewRef,
        result: Sender<Option<crate::preview::Picture>>,
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
    /// The browser: saved as a file, or with `play`, played in the viewer
    /// (ROADMAP P4).
    #[cfg(target_arch = "wasm32")]
    Download {
        id: i64,
        media: crate::media::Media,
        play: bool,
    },
    /// Save an attachment (decrypted) for the media viewer to play.
    #[cfg(target_os = "linux")]
    Fetch {
        media: crate::media::Media,
        destination: std::path::PathBuf,
        result: Sender<bool>,
    },
}
