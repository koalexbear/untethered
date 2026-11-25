use serde_json::Value;
use std::collections::BTreeMap;

// The browser worker buffers an entire file. Enforce the same initial limit on
// both clients; apply a smaller limit when advertised by the homeserver.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 10;

#[derive(Clone)]
pub struct LocalFile {
    pub name: String,
    pub size: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pub origin: NativeSource,
    #[cfg(target_arch = "wasm32")]
    pub file: web_sys::File,
}
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub enum NativeSource {
    Path(std::path::PathBuf),
    #[cfg(target_os = "linux")]
    Portal(std::sync::Arc<std::fs::File>),
    // Paste stays in process memory until Send or removal; clones share bytes.
    Clipboard(bytes::Bytes),
}
#[cfg(not(target_arch = "wasm32"))]
impl NativeSource {
    // Reopen pinned regular files for independent read offsets: thumbnail
    // decoding and upload may run concurrently; dup/try_clone shares a cursor.
    pub fn read_path(&self) -> Option<std::borrow::Cow<'_, std::path::Path>> {
        match self {
            Self::Path(path) => Some(path.as_path().into()),
            #[cfg(target_os = "linux")]
            Self::Portal(fd) => {
                use std::os::fd::AsRawFd;
                Some(std::path::PathBuf::from(format!("/proc/self/fd/{}", fd.as_raw_fd())).into())
            }
            Self::Clipboard(_) => None,
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl LocalFile {
    pub fn from_path(path: std::path::PathBuf) -> Option<Self> {
        let meta = path.metadata().ok()?;
        if !meta.is_file() {
            return None;
        }
        Some(Self {
            name: path.file_name()?.to_string_lossy().into_owned(),
            size: meta.len(),
            origin: NativeSource::Path(path),
        })
    }
}

#[derive(Default)]
pub struct Outbox {
    next: u64,
    pub rooms: BTreeMap<String, Vec<Attachment>>,
    pub skipped: BTreeMap<String, String>,
}
pub struct Attachment {
    pub id: u64,
    pub source: LocalFile,
    pub thumbnail: Option<iced::widget::image::Handle>,
    pub descriptor: Option<Value>,
    pub txn: Option<String>,
    pub busy: bool,
    pub error: Option<String>,
    pub upload: Option<UploadAttempt>,
    pub cancelled: bool,
}
// Dropping this lease stops only its own transport attempt (including on
// logout, room leave and shutdown). Retries always allocate a fresh RPC ID.
pub struct UploadAttempt {
    pub request: i64,
    pub percent: Option<u8>,
    pub _lease: async_channel::Sender<()>,
}
impl Attachment {
    pub fn status_label(&self) -> String {
        if let Some(upload) = &self.upload {
            return match upload.percent {
                None => "Preparing…".into(),
                Some(100) => "Finalizing…".into(),
                Some(n) => format!("Upload {n}%"),
            };
        }
        if self.cancelled {
            "Cancelled · retry"
        } else if self.error.is_some() {
            "Failed · retry"
        } else if self.busy {
            "Sending…"
        } else {
            "Ready"
        }
        .into()
    }
}
impl Outbox {
    fn too_large(file: &LocalFile, limit: u64) -> String {
        let name: String = file
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(100)
            .collect();
        format!(
            "{name} was not attached: {} bytes exceeds the {limit}-byte limit.",
            file.size
        )
    }
    fn note(&mut self, room: &str, oversize: Option<String>, full: bool) {
        let mut note = oversize.unwrap_or_default();
        if full {
            if !note.is_empty() {
                note.push(' ')
            }
            note.push_str(&format!(
                "Extra files were skipped: up to {MAX_FILES} attachments per message batch."
            ));
        }
        if note.is_empty() {
            self.skipped.remove(room);
        } else {
            self.skipped.insert(room.into(), note);
        }
    }
    pub fn apply_limit(&mut self, limit: u64) {
        // A pick/drop can finish before get_media_config. Recheck staged
        // sources without discarding other files or their caption.
        self.rooms.retain(|room, q| {
            if let Some(a) = q
                .iter()
                .find(|a| !a.busy && a.descriptor.is_none() && a.source.size > limit)
            {
                self.skipped
                    .insert(room.clone(), Self::too_large(&a.source, limit));
                q.retain(|a| a.busy || a.descriptor.is_some() || a.source.size <= limit);
            }
            !q.is_empty()
        });
    }
    pub fn stage(
        &mut self,
        room: &str,
        files: Vec<LocalFile>,
        limit: u64,
    ) -> Vec<(u64, LocalFile)> {
        let queue = self.rooms.entry(room.to_owned()).or_default();
        let mut oversized = None;
        let mut full = false;
        let mut previews = vec![];
        for file in files {
            if file.size > limit {
                oversized.get_or_insert_with(|| Self::too_large(&file, limit));
                continue;
            }
            if queue.len() >= MAX_FILES {
                full = true;
                continue;
            }
            self.next += 1;
            if crate::preview::candidate(&file) {
                previews.push((self.next, file.clone()));
            }
            queue.push(Attachment {
                id: self.next,
                source: file,
                thumbnail: None,
                descriptor: None,
                txn: None,
                busy: false,
                error: None,
                upload: None,
                cancelled: false,
            });
        }
        if queue.is_empty() {
            self.rooms.remove(room);
        }
        self.note(room, oversized, full);
        previews
    }
    pub fn get_mut(&mut self, room: &str, id: u64) -> Option<&mut Attachment> {
        self.rooms.get_mut(room)?.iter_mut().find(|a| a.id == id)
    }
    pub fn remove(&mut self, room: &str, id: u64) {
        if let Some(q) = self.rooms.get_mut(room) {
            q.retain(|a| a.id != id);
            if q.is_empty() {
                self.rooms.remove(room);
            }
        }
    }
}
pub fn size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1 << 20) as f64)
    } else if n >= 1024 {
        format!("{:.1} KiB", n as f64 / 1024.)
    } else {
        format!("{n} B")
    }
}

#[derive(Clone)]
pub struct Media {
    pub name: String,
    pub size: Option<u64>,
    pub mxc: String,
    pub encrypted: bool,
    pub preview: Option<PreviewRef>,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PreviewRef {
    pub mxc: String,
    pub encrypted: bool,
}
// Cap the bytes read, not just the size claimed in the Matrix event.
pub const INLINE_BYTES: usize = 4 * 1024 * 1024;
impl Media {
    pub fn parse(content: &Value) -> Option<Self> {
        if !matches!(
            content["msgtype"].as_str(),
            Some("m.file" | "m.image" | "m.audio" | "m.video")
        ) {
            return None;
        }
        let encrypted = content["file"].is_object();
        let mxc = if encrypted {
            &content["file"]["url"]
        } else {
            &content["url"]
        }
        .as_str()?;
        media_path(mxc)?;
        let info = &content["info"];
        let size = info["size"].as_u64();
        let preview = (content["msgtype"] == "m.image")
            .then(|| {
                let thumbnail = info["thumbnail_file"]["url"]
                    .as_str()
                    .map(|s| (s, true))
                    .or_else(|| {
                        if encrypted {
                            None
                        } else {
                            info["thumbnail_url"].as_str().map(|s| (s, false))
                        }
                    });
                let picked = thumbnail
                    .filter(|(s, _)| {
                        media_path(s).is_some()
                            && info["thumbnail_info"]["size"]
                                .as_u64()
                                .is_none_or(|n| n <= INLINE_BYTES as u64)
                    })
                    .or_else(|| {
                        size.filter(|n| *n <= INLINE_BYTES as u64)
                            .map(|_| (mxc, encrypted))
                    })?;
                Some(PreviewRef {
                    mxc: picked.0.to_owned(),
                    encrypted: picked.1,
                })
            })
            .flatten();
        // Never use a sender-supplied path as a save destination.
        let name = content["filename"]
            .as_str()
            .or(content["body"].as_str())
            .unwrap_or("attachment")
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty() && *s != "." && *s != "..")
            .unwrap_or("attachment")
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>();
        Some(Self {
            name,
            size,
            preview,
            mxc: mxc.into(),
            encrypted,
        })
    }
}
pub fn media_path(mxc: &str) -> Option<String> {
    let (server, id) = mxc.strip_prefix("mxc://")?.split_once('/')?;
    if server.is_empty()
        || matches!(server, "." | "..")
        || id.is_empty()
        || !server
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return None;
    }
    Some(format!("/_gomuks/media/{server}/{id}"))
}

#[cfg(target_os = "linux")]
pub fn drop_paths(uris: &str) -> Vec<std::path::PathBuf> {
    uris.lines()
        .filter_map(|s| {
            if s.starts_with('#') {
                return None;
            }
            let u = url::Url::parse(s).ok()?;
            if u.scheme() != "file"
                || u.host_str().is_some_and(|s| s != "localhost")
                || u.query().is_some()
                || u.fragment().is_some()
            {
                return None;
            }
            u.to_file_path().ok()
        })
        .take(MAX_FILES + 1)
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn multi_file_drafts_obey_per_room_count_and_server_size_limit() {
        let mut out = super::Outbox::default();
        let files = (0..14)
            .map(|i| super::LocalFile {
                name: format!("file-{i}.txt"),
                size: if i == 1 { 11 } else { 10 },
                origin: super::NativeSource::Path("unused".into()),
            })
            .collect();
        out.stage("original", files, 10);
        assert!(
            out.skipped["original"].contains("file-1.txt")
                && out.skipped["original"].contains("Extra files")
        );
        let q = &out.rooms["original"];
        assert_eq!(q.len(), super::MAX_FILES);
        assert_eq!(q.first().unwrap().source.name, "file-0.txt");
        assert_eq!(q.last().unwrap().source.name, "file-10.txt");
        assert!(q.iter().all(|a| a.source.size <= 10));
        let extra = q[0].source.clone();
        out.stage("other", vec![extra], 10);
        assert!(!out.skipped.contains_key("other"));
        assert_eq!(out.rooms["original"].len(), super::MAX_FILES);
    }
    #[test]
    fn uri_lists_handle_multiple_files_and_reject_network_paths() {
        let u = "# ignored\r\nfile:///tmp/a%20b.txt\r\nhttps://example.org/file\nfile://remote/share\nfile:///tmp/c.pdf\r\n";
        assert_eq!(
            super::drop_paths(u),
            [
                std::path::PathBuf::from("/tmp/a b.txt"),
                "/tmp/c.pdf".into()
            ]
        );
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn encrypted_image_prefers_encrypted_thumbnail_and_never_uses_unencrypted_sidecar() {
        let mut data = json!({"msgtype":"m.image","file":{"url":"mxc://s/original"},"info":{
            "size":12345,"thumbnail_file":{"url":"mxc://s/small"},"thumbnail_url":"mxc://s/plain"}});
        let preview = Media::parse(&data).unwrap().preview.unwrap();
        assert_eq!(
            (preview.mxc.as_str(), preview.encrypted),
            ("mxc://s/small", true)
        );
        data["info"]
            .as_object_mut()
            .unwrap()
            .remove("thumbnail_file");
        assert_eq!(
            Media::parse(&data).unwrap().preview.unwrap().mxc,
            "mxc://s/original"
        );
        data["info"]["size"] = json!(INLINE_BYTES + 1);
        assert!(Media::parse(&data).unwrap().preview.is_none());
    }
}
