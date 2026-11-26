//! Convert one compositor drop into a room draft. Only the document portal
//! interprets a transfer key; it is never logged or treated as a path.
use crate::{
    Msg,
    media::{self, LocalFile, NativeSource},
};
use smithay_clipboard::{DropMime, FileDrag, FileDrop};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncReadExt;

pub type Result = std::result::Result<Vec<LocalFile>, &'static str>;
const INVALID: &str = "The dropped file offer was invalid. Try dropping the files again.";

pub fn listen() -> iced::Task<Msg> {
    let (tx, rx) = async_channel::unbounded();
    smithay_clipboard::on_file_drag(move |event| {
        let msg = match event {
            FileDrag::Enter => Msg::HoverFiles(true),
            FileDrag::Leave => Msg::HoverFiles(false),
            FileDrag::Dropped(drop) => {
                // Iced messages are Clone; the FD/offer must be consumed once.
                let (one, offer) = async_channel::bounded(1);
                let _ = one.try_send(drop);
                Msg::WaylandDrop(offer)
            }
        };
        let _ = tx.try_send(msg);
    });
    iced::Task::run(rx, |m| m)
}

fn portal_key(bytes: &[u8]) -> std::result::Result<&str, &'static str> {
    // Some toolkit strings include their terminating NUL.
    let key =
        std::str::from_utf8(bytes.strip_suffix(&[0]).unwrap_or(bytes)).map_err(|_| INVALID)?;
    if key.is_empty() || key.chars().any(char::is_control) {
        Err(INVALID)
    } else {
        Ok(key)
    }
}

fn pin(path: &Path) -> Option<LocalFile> {
    use std::os::unix::fs::OpenOptionsExt;
    if !path.is_absolute() {
        return None;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(LocalFile {
        name: path.file_name()?.to_string_lossy().into_owned(),
        size: meta.len(),
        origin: NativeSource::Portal(file.into()),
    })
}

pub async fn resolve(drop: FileDrop) -> Result {
    let FileDrop {
        mime,
        pipe,
        completion,
    } = drop;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let max = match mime {
            DropMime::UriList => 256 * 1024,
            DropMime::Portal => 4096,
        };
        let mut bytes = Vec::new();
        tokio::net::unix::pipe::Receiver::from_owned_fd(pipe)
            .map_err(|_| INVALID)?
            .take(max + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| INVALID)?;
        if bytes.len() as u64 > max {
            return Err(INVALID);
        }
        let paths = match mime {
            DropMime::UriList => {
                media::drop_paths(std::str::from_utf8(&bytes).map_err(|_| INVALID)?)
            }
            DropMime::Portal => {
                let key = portal_key(&bytes)?;
                crate::app_id::portal_ready().await;
                let portal = ashpd::documents::FileTransfer::new()
                    .await
                    .map_err(|_| "The desktop file-transfer portal is unavailable.")?;
                portal
                    .retrieve_files(key)
                    .await
                    .map_err(
                        |_| "The desktop portal could not retrieve these files. Try a fresh drop.",
                    )?
                    .into_iter()
                    .map(PathBuf::from)
                    .collect()
            }
        };
        tokio::task::spawn_blocking(move || {
            let files: Vec<_> = paths
                .into_iter()
                .take(media::MAX_FILES + 1)
                .filter_map(|path| match mime {
                    DropMime::UriList => LocalFile::from_path(path),
                    DropMime::Portal => pin(&path),
                })
                .collect();
            if files.is_empty() {
                Err("The drop contained no readable regular files.")
            } else {
                Ok(files)
            }
        })
        .await
        .map_err(|_| INVALID)?
    })
    .await
    .map_err(|_| "The file drop timed out. Try dropping the files again.")?;
    if result.is_ok() {
        completion.finish();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portal_key_is_opaque_utf8_not_a_uri_list() {
        assert_eq!(portal_key(b"an opaque token\0").unwrap(), "an opaque token");
        for bad in [&b"\0"[..], b"a\0b", b"a\nb", b"\xff"] {
            assert!(portal_key(bad).is_err());
        }
    }
    #[test]
    fn pinned_portal_file_has_independent_cursors_and_survives_name_replacement() {
        use std::io::Read;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("original");
        std::fs::write(&path, b"original").unwrap();
        let staged = pin(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"replaced").unwrap();
        let p = staged.origin.read_path().unwrap();
        let mut a = std::fs::File::open(&p).unwrap();
        let mut b = std::fs::File::open(&p).unwrap();
        let mut buf = [0; 4];
        a.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"orig");
        b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"orig");
        assert!(pin(tmp.path()).is_none());
    }
}
