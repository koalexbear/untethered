//! Copy Image (part of ROADMAP P3), as on Discord: an image attachment's
//! original goes on the clipboard as a PNG, from a message's right-click menu
//! or the media viewer. The original is downloaded (decrypted) into the
//! private runtime directory, decoded, and removed once it's on the clipboard.
//! On Wayland the PNG is offered through Iced's own clipboard connection
//! (vendor/smithay-clipboard); elsewhere arboard holds it.
use crate::{
    App, Msg, Request,
    backend::Command,
    media::{Kind, Media},
};
use iced::Task;
use std::path::{Path, PathBuf};
use tempfile::TempPath;

pub const COPYING: &str = "Copying image…";
pub const COPIED: &str = "Image copied.";
const FAILED: &str = "Couldn't copy the image. Try again.";
const TOO_LARGE: &str = "That image is too large to copy.";
/// The largest picture copied, in pixels.
const MAX_PIXELS: u64 = 40_000_000;

impl App {
    pub fn copy_image(&mut self, media: Media) {
        if !self.logged_in || !self.connected || media.kind != Kind::Image || self.copying_image() {
            return;
        }
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let Ok(file) = tempfile::Builder::new()
            .prefix("untethered-copy-")
            .tempfile_in(dir)
        else {
            return self.image_copied(Err(FAILED));
        };
        let path = file.into_temp_path();
        self.seq += 1;
        if self
            .commands
            .try_send(Command::Download {
                id: self.seq,
                media,
                destination: path.to_path_buf(),
            })
            .is_ok()
        {
            self.pending.insert(self.seq, Request::CopyImage(path));
            self.copy_note = Some(COPYING);
            self.status = COPYING.into();
        } else {
            self.image_copied(Err(FAILED));
        }
    }

    pub fn copying_image(&self) -> bool {
        self.copy_ready.is_some()
            || self
                .pending
                .values()
                .any(|p| matches!(p, Request::CopyImage(_)))
    }

    /// The original arrived (or didn't): copy it once this update is done.
    pub fn image_fetched(&mut self, path: TempPath, ok: bool) {
        if ok {
            self.copy_ready = Some(path);
        } else {
            self.image_copied(Err(FAILED));
        }
    }

    /// Decode a fetched original and put it on the clipboard, off the UI
    /// thread. Runs after every update.
    pub fn copy_fetched(&mut self) -> Task<Msg> {
        let Some(path) = self.copy_ready.take() else {
            return Task::none();
        };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || to_clipboard(&path))
                    .await
                    .unwrap_or(Err(FAILED))
            },
            Msg::ImageCopied,
        )
    }

    pub fn image_copied(&mut self, result: Result<(), &'static str>) {
        let note = match result {
            Ok(()) => COPIED,
            Err(why) => why,
        };
        self.copy_note = Some(note);
        self.status = note.into();
    }
}

/// Small enough to decode and copy.
fn copyable(width: u32, height: u32) -> bool {
    u64::from(width) * u64::from(height) <= MAX_PIXELS
}

fn to_clipboard(path: &Path) -> Result<(), &'static str> {
    let reader = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|_| FAILED)?;
    let (w, h) = reader.into_dimensions().map_err(|_| FAILED)?;
    if !copyable(w, h) {
        return Err(TOO_LARGE);
    }
    let picture = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|_| FAILED)?
        .decode()
        .map_err(|_| FAILED)?
        .into_rgba8();
    #[cfg(target_os = "linux")]
    {
        let png = png(&picture)?;
        if let Some(stored) = smithay_clipboard::store_png(png) {
            return if stored { Ok(()) } else { Err(FAILED) };
        }
    }
    portable(picture)
}

#[cfg(target_os = "linux")]
fn png(picture: &image::RgbaImage) -> Result<Vec<u8>, &'static str> {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            picture.as_raw(),
            picture.width(),
            picture.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| FAILED)?;
    Ok(png)
}

/// X11 and Windows: arboard serves the picture for as long as its clipboard
/// lives, so it lives as long as the app.
fn portable(picture: image::RgbaImage) -> Result<(), &'static str> {
    static CLIPBOARD: std::sync::Mutex<Option<arboard::Clipboard>> = std::sync::Mutex::new(None);
    let mut held = CLIPBOARD.lock().map_err(|_| FAILED)?;
    if held.is_none() {
        *held = Some(arboard::Clipboard::new().map_err(|_| FAILED)?);
    }
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    held.as_mut()
        .ok_or(FAILED)?
        .set_image(arboard::ImageData {
            width,
            height,
            bytes: picture.into_raw().into(),
        })
        .map_err(|_| FAILED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_pictures_are_refused_and_unreadable_files_fail() {
        assert!(copyable(6000, 4000));
        assert!(!copyable(10_000, 10_000));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-an-image");
        std::fs::write(&path, b"plain text").unwrap();
        assert_eq!(to_clipboard(&path), Err(FAILED));
    }
}
