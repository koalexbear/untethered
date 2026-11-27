//! Smithay Clipboard
//!
//! Provides access to the Wayland clipboard for gui applications. The user
//! should have surface around.

#![deny(clippy::all, clippy::if_not_else, clippy::enum_glob_use)]
use std::ffi::c_void;
use std::io::Result;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc,Mutex,Weak};
use std::os::fd::OwnedFd;

use sctk::reexports::calloop::channel::{self, Sender};
use sctk::reexports::client::Connection;
use sctk::reexports::client::backend::Backend;

mod mime;
mod state;
mod worker;

pub enum PasteOffer {
    Png(OwnedFd),
    Text(OwnedFd),
}
static ACTIVE: Mutex<Weak<Sender<worker::Command>>> = Mutex::new(Weak::new());

/// `None` means Iced is not using a Wayland clipboard. Otherwise request the
/// focused seat's regular selection on its event queue. Never watches changes.
pub fn composer_offer() -> Option<Result<Option<PasteOffer>>> {
    let sender=ACTIVE.lock().ok()?.upgrade()?;
    let (tx,rx)=mpsc::channel();
    Some(sender.send(worker::Command::ComposerOffer(tx))
        .map_err(|_|std::io::Error::other("clipboard worker stopped"))
        .and_then(|()|rx.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_|std::io::Error::other("clipboard worker did not reply"))?))
}

/// `None` means Iced is not using a Wayland clipboard. Otherwise offer a PNG
/// on the focused seat's clipboard, and say whether that worked.
pub fn store_png(png: Vec<u8>) -> Option<bool> {
    let sender=ACTIVE.lock().ok()?.upgrade()?;
    let (tx,rx)=mpsc::channel();
    sender.send(worker::Command::StorePng(png,tx)).ok()?;
    Some(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or(false))
}

/// File drag events from the existing data device. No clipboard selections
/// are inspected. Untethered has one top-level window.
pub enum FileDrag {
    Enter,
    Leave,
    Dropped(FileDrop),
}

#[derive(Clone, Copy)]
pub enum DropMime {
    UriList,
    Portal,
}

impl DropMime {
    fn offered(types: &[String]) -> Option<Self> {
        // Keep the existing direct path for ordinary host-to-host drops.
        [Self::UriList, Self::Portal].into_iter()
            .find(|mime| types.iter().any(|s| s == mime.name()))
    }

    fn name(self) -> &'static str {
        match self {
            Self::UriList => "text/uri-list",
            Self::Portal => "application/vnd.portal.filetransfer",
        }
    }
}

pub struct FileDrop {
    pub mime: DropMime,
    pub pipe: OwnedFd,
    pub completion: DropCompletion,
}

/// Acknowledge only after consuming the payload. For portal drops this means
/// retrieving the files, not merely reading the transfer key.
pub struct DropCompletion(Sender<()>);
impl DropCompletion {
    pub fn finish(self) { let _ = self.0.send(()); }
}

type FileDragHandler = Box<dyn Fn(FileDrag) + Send + Sync>;
static FILE_DRAG: std::sync::OnceLock<FileDragHandler> = std::sync::OnceLock::new();

pub fn on_file_drag(handler: impl Fn(FileDrag) + Send + Sync + 'static) {
    let _ = FILE_DRAG.set(Box::new(handler));
}

/// Access to a Wayland clipboard.
pub struct Clipboard {
    request_sender: Arc<Sender<worker::Command>>,
    request_receiver: Receiver<Result<String>>,
    clipboard_thread: Option<std::thread::JoinHandle<()>>,
}

impl Clipboard {
    /// Creates new clipboard which will be running on its own thread with its
    /// own event queue to handle clipboard requests.
    ///
    /// # Safety
    ///
    /// `display` must be a valid `*mut wl_display` pointer, and it must remain
    /// valid for as long as `Clipboard` object is alive.
    pub unsafe fn new(display: *mut c_void) -> Self {
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);

        // Create channel to send data to clipboard thread.
        let (request_sender, rx_chan) = channel::channel();
        // Create channel to get data from the clipboard thread.
        let (clipboard_reply_sender, request_receiver) = mpsc::channel();

        let name = String::from("smithay-clipboard");
        let clipboard_thread = worker::spawn(name, connection, rx_chan, clipboard_reply_sender);

        let request_sender=Arc::new(request_sender);
        *ACTIVE.lock().expect("clipboard sender lock")=Arc::downgrade(&request_sender);
        Self { request_receiver, request_sender, clipboard_thread }
    }

    /// Load clipboard data.
    ///
    /// Loads content from a clipboard on a last observed seat.
    pub fn load(&self) -> Result<String> {
        let _ = self.request_sender.send(worker::Command::Load);

        if let Ok(reply) = self.request_receiver.recv() {
            reply
        } else {
            // The clipboard thread is dead, however we shouldn't crash downstream, so
            // propogating an error.
            Err(std::io::Error::other("clipboard is dead."))
        }
    }

    /// Store to a clipboard.
    ///
    /// Stores to a clipboard on a last observed seat.
    pub fn store<T: Into<String>>(&self, text: T) {
        let request = worker::Command::Store(text.into());
        let _ = self.request_sender.send(request);
    }

    /// Load primary clipboard data.
    ///
    /// Loads content from a  primary clipboard on a last observed seat.
    pub fn load_primary(&self) -> Result<String> {
        let _ = self.request_sender.send(worker::Command::LoadPrimary);

        if let Ok(reply) = self.request_receiver.recv() {
            reply
        } else {
            // The clipboard thread is dead, however we shouldn't crash downstream, so
            // propogating an error.
            Err(std::io::Error::other("clipboard is dead."))
        }
    }

    /// Store to a primary clipboard.
    ///
    /// Stores to a primary clipboard on a last observed seat.
    pub fn store_primary<T: Into<String>>(&self, text: T) {
        let request = worker::Command::StorePrimary(text.into());
        let _ = self.request_sender.send(request);
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        // Shutdown smithay-clipboard.
        let _ = self.request_sender.send(worker::Command::Exit);
        if let Some(clipboard_thread) = self.clipboard_thread.take() {
            let _ = clipboard_thread.join();
        }
    }
}
