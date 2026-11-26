//! Attachments on the matrix-sdk engine: uploads with progress and
//! cancellation, inline image previews, downloads, and the upload size
//! limit.
//!
//! The App keeps its outbox (`transfers.rs`) and sends the same
//! `backend::Command`s as on gomuks; the engine answers them. The App's
//! `media::Media` carries only an mxc URI, and encrypted media also needs the
//! keys from its event, so timeline items register each attachment's
//! `MediaSource` here, keyed by URI.
use super::{Event, Failure, rt};
#[cfg(not(target_arch = "wasm32"))]
use crate::media::NativeSource;
use crate::media::{INLINE_BYTES, LocalFile, Media, PreviewRef};
use async_channel::{Receiver, Sender};
use matrix_sdk::{
    Client, TransmissionProgress,
    media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings},
    ruma::{
        MxcUri, OwnedMxcUri, UInt,
        events::room::{MediaSource, message::MessageType},
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, Mutex},
};

/// Where each attachment (and its thumbnail) seen in a timeline lives.
#[derive(Default)]
pub struct Sources(Mutex<HashMap<String, MediaSource>>);

impl Sources {
    pub fn remember(&self, msgtype: &MessageType) {
        let (file, thumbnail) = match msgtype {
            MessageType::Image(c) => (
                &c.source,
                c.info.as_ref().and_then(|i| i.thumbnail_source.as_ref()),
            ),
            MessageType::Video(c) => (
                &c.source,
                c.info.as_ref().and_then(|i| i.thumbnail_source.as_ref()),
            ),
            MessageType::File(c) => (
                &c.source,
                c.info.as_ref().and_then(|i| i.thumbnail_source.as_ref()),
            ),
            MessageType::Audio(c) => (&c.source, None),
            _ => return,
        };
        let mut known = self.0.lock().unwrap();
        for source in std::iter::once(file).chain(thumbnail) {
            known.insert(uri(source).to_string(), source.clone());
        }
    }

    /// The source for an mxc URI the App asks for. Plain media needs no
    /// keys; encrypted media must come from an event we've seen, and never
    /// downloads as plain.
    fn get(&self, mxc: &str, encrypted: bool) -> Option<MediaSource> {
        match self.0.lock().unwrap().get(mxc) {
            Some(source) => {
                (matches!(source, MediaSource::Encrypted(_)) == encrypted).then(|| source.clone())
            }
            None if !encrypted => Some(MediaSource::Plain(OwnedMxcUri::from(mxc))),
            None => None,
        }
    }
}

fn uri(source: &MediaSource) -> &MxcUri {
    match source {
        MediaSource::Plain(uri) => uri,
        MediaSource::Encrypted(file) => &file.url,
    }
}

/// A small image for the timeline, decoded.
#[cfg(not(target_arch = "wasm32"))]
pub async fn inline(
    client: Client,
    sources: Arc<Sources>,
    preview: PreviewRef,
) -> Option<iced::widget::image::Handle> {
    crate::preview::inline(inline_bytes(client, sources, preview).await?).await
}

/// The bytes of a small image for the timeline. Plain images use the
/// homeserver's thumbnail; encrypted ones can only be fetched whole.
pub async fn inline_bytes(
    client: Client,
    sources: Arc<Sources>,
    preview: PreviewRef,
) -> Option<Vec<u8>> {
    let source = sources.get(&preview.mxc, preview.encrypted)?;
    let media = client.media();
    let bytes = match source {
        MediaSource::Plain(_) => {
            let thumbnail = MediaRequestParameters {
                source: source.clone(),
                format: MediaFormat::Thumbnail(MediaThumbnailSettings::new(
                    UInt::from(800u32),
                    UInt::from(600u32),
                )),
            };
            match media.get_media_content(&thumbnail, true).await {
                Ok(bytes) => bytes,
                Err(_) => whole(&media, source).await?,
            }
        }
        MediaSource::Encrypted(_) => whole(&media, source).await?,
    };
    (bytes.len() <= INLINE_BYTES).then_some(bytes)
}

async fn whole(media: &matrix_sdk::Media, source: MediaSource) -> Option<Vec<u8>> {
    let request = MediaRequestParameters {
        source,
        format: MediaFormat::File,
    };
    media.get_media_content(&request, true).await.ok()
}

/// Save an attachment (decrypted) to a file the user picked.
#[cfg(not(target_arch = "wasm32"))]
pub async fn download(
    client: Client,
    sources: Arc<Sources>,
    media: Media,
    destination: std::path::PathBuf,
) -> Result<(), Failure> {
    let bytes = whole_file(client, sources, &media).await?;
    rt::blocking(move || {
        use std::io::Write;
        let dir = destination.parent().ok_or(Failure::Failed)?;
        let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|_| Failure::Failed)?;
        file.write_all(&bytes).map_err(|_| Failure::Failed)?;
        file.persist(&destination).map_err(|_| Failure::Failed)?;
        Ok(())
    })
    .await
    .ok_or(Failure::Failed)?
}

/// An attachment, decrypted.
pub async fn whole_file(
    client: Client,
    sources: Arc<Sources>,
    media: &Media,
) -> Result<Vec<u8>, Failure> {
    let source = sources
        .get(&media.mxc, media.encrypted)
        .ok_or(Failure::Unavailable)?;
    let request = MediaRequestParameters {
        source,
        format: MediaFormat::File,
    };
    // Downloads skip the media cache, which is for previews.
    client
        .media()
        .get_media_content(&request, false)
        .await
        .map_err(|_| Failure::Unreachable)
}

pub async fn upload_limit(client: &Client) -> Result<u64, Failure> {
    client
        .load_or_fetch_max_upload_size()
        .await
        .map(u64::from)
        .map_err(|_| Failure::Unreachable)
}

/// Thumbnails go with images larger than this, like other clients send.
const THUMBNAIL: (u32, u32) = (800, 600);
/// Don't decode bigger images for a thumbnail.
const THUMBNAIL_SOURCE_BYTES: usize = 24 << 20;

/// Upload a file (encrypted for encrypted rooms) and describe it as the
/// content of an attachment message, like gomuks's `/_gomuks/upload`.
/// Progress goes out as [`Event::UploadProgress`]; dropping the App's lease
/// (`cancel` closes) stops it.
pub async fn upload(
    client: Client,
    id: i64,
    file: LocalFile,
    encrypt: bool,
    limit: u64,
    cancel: Receiver<()>,
    events: Sender<Event>,
) -> Result<Value, Failure> {
    let work = async {
        if file.size > limit {
            return Err(Failure::Failed);
        }
        let bytes = read(&file).await?;
        let mime = mime_type(&file.name, &bytes);
        let image = mime.type_() == mime::IMAGE;
        let size = if image { dimensions(&bytes) } else { None };
        let thumbnail = match size {
            Some((w, h))
                if (w > THUMBNAIL.0 || h > THUMBNAIL.1)
                    && bytes.len() <= THUMBNAIL_SOURCE_BYTES
                    && mime != mime::IMAGE_GIF =>
            {
                let source = bytes.clone();
                rt::blocking(move || thumbnail(&source)).await.flatten()
            }
            _ => None,
        };
        let length = bytes.len();
        let source = send(&client, &mime, bytes, encrypt, Some((id, &events))).await?;
        let mut info = json!({ "mimetype": mime.essence_str(), "size": length });
        if let Some((w, h)) = size {
            info["w"] = json!(w);
            info["h"] = json!(h);
        }
        if let Some((data, w, h)) = thumbnail {
            let length = data.len();
            let source = send(&client, &mime::IMAGE_JPEG, data, encrypt, None).await?;
            info["thumbnail_info"] =
                json!({ "mimetype": "image/jpeg", "size": length, "w": w, "h": h });
            match source {
                MediaSource::Plain(uri) => info["thumbnail_url"] = json!(uri),
                MediaSource::Encrypted(file) => info["thumbnail_file"] = json!(file),
            }
        }
        let kind = mime.type_();
        let msgtype = if image && size.is_some() {
            "m.image"
        } else if kind == mime::VIDEO {
            "m.video"
        } else if kind == mime::AUDIO {
            "m.audio"
        } else {
            "m.file"
        };
        let mut content = json!({
            "msgtype": msgtype,
            "body": file.name,
            "filename": file.name,
            "info": info,
        });
        match source {
            MediaSource::Plain(uri) => content["url"] = json!(uri),
            MediaSource::Encrypted(file) => content["file"] = json!(file),
        }
        Ok(content)
    };
    tokio::select! {
        result = work => result,
        _ = cancel.recv() => Err(Failure::Cancelled),
    }
}

#[cfg(target_arch = "wasm32")]
async fn read(file: &LocalFile) -> Result<Vec<u8>, Failure> {
    let buffer = wasm_bindgen_futures::JsFuture::from(file.file.array_buffer())
        .await
        .map_err(|_| Failure::Failed)?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    // The file changed since it was attached.
    if bytes.len() as u64 != file.size {
        return Err(Failure::Failed);
    }
    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
async fn read(file: &LocalFile) -> Result<Vec<u8>, Failure> {
    match &file.origin {
        NativeSource::Clipboard(bytes) if bytes.len() as u64 == file.size => Ok(bytes.to_vec()),
        NativeSource::Clipboard(_) => Err(Failure::Failed),
        origin => {
            let path = origin.read_path().ok_or(Failure::Failed)?;
            let bytes = tokio::fs::read(path).await.map_err(|_| Failure::Failed)?;
            // The file changed since it was attached.
            if bytes.len() as u64 != file.size {
                return Err(Failure::Failed);
            }
            Ok(bytes)
        }
    }
}

/// Upload one blob, reporting whole percents of it when `progress` is set.
async fn send(
    client: &Client,
    mime: &mime::Mime,
    data: Vec<u8>,
    encrypt: bool,
    progress: Option<(i64, &Sender<Event>)>,
) -> Result<MediaSource, Failure> {
    if encrypt {
        let mut reader = Cursor::new(data);
        let upload = client.upload_encrypted_file(&mut reader);
        let watch = upload.subscribe_to_send_progress();
        tokio::select! {
            file = upload => file.map(|f| MediaSource::Encrypted(Box::new(f))).map_err(|_| Failure::Unreachable),
            () = report(watch, progress) => unreachable!(),
        }
    } else {
        let upload = client.media().upload(mime, data, None);
        let watch = upload.subscribe_to_send_progress();
        tokio::select! {
            response = upload => response.map(|r| MediaSource::Plain(r.content_uri)).map_err(|_| Failure::Unreachable),
            () = report(watch, progress) => unreachable!(),
        }
    }
}

/// Whole percents of an upload as [`Event::UploadProgress`]; never returns.
async fn report(
    mut watch: impl futures_util::Stream<Item = TransmissionProgress> + Unpin,
    progress: Option<(i64, &Sender<Event>)>,
) {
    use futures_util::StreamExt;
    let Some((id, events)) = progress else {
        return std::future::pending().await;
    };
    let mut last = None;
    while let Some(p) = watch.next().await {
        if p.total == 0 {
            continue;
        }
        let percent = (p.current * 100 / p.total).min(100);
        if last != Some(percent) {
            last = Some(percent);
            let fraction = p.current as f64 / p.total as f64;
            let _ = events
                .send(Event::UploadProgress {
                    id,
                    fraction: fraction.min(1.0),
                })
                .await;
        }
    }
    std::future::pending().await
}

/// The MIME type from the content for images, otherwise from the name.
fn mime_type(name: &str, bytes: &[u8]) -> mime::Mime {
    if let Ok(format) = image::guess_format(bytes) {
        let essence = match format {
            image::ImageFormat::Png => Some(mime::IMAGE_PNG),
            image::ImageFormat::Jpeg => Some(mime::IMAGE_JPEG),
            image::ImageFormat::Gif => Some(mime::IMAGE_GIF),
            image::ImageFormat::WebP => "image/webp".parse().ok(),
            _ => None,
        };
        if let Some(essence) = essence {
            return essence;
        }
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    let essence = match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "zip" => "application/zip",
        "json" => "application/json",
        _ => "application/octet-stream",
    };
    essence.parse().unwrap_or(mime::APPLICATION_OCTET_STREAM)
}

fn decoder(bytes: &[u8]) -> Option<impl image::ImageDecoder + '_> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(96 << 20);
    reader.limits(limits);
    reader.into_decoder().ok()
}

/// Width and height as displayed, after the EXIF orientation.
fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    use image::{ImageDecoder, metadata::Orientation};
    let mut decoder = decoder(bytes)?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    Some(match decoder.orientation() {
        Ok(
            Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH,
        ) => (h, w),
        _ => (w, h),
    })
}

/// A JPEG that fits in [`THUMBNAIL`], with its size.
fn thumbnail(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    use image::{DynamicImage, ImageDecoder};
    let mut decoder = decoder(bytes)?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > 24_000_000 || decoder.total_bytes() > 96 << 20 {
        return None;
    }
    let orientation = decoder.orientation().ok();
    let mut full = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        full.apply_orientation(o);
    }
    let small = full.thumbnail(THUMBNAIL.0, THUMBNAIL.1);
    let mut out = Vec::new();
    small
        .to_rgb8()
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .ok()?;
    Some((out, small.width(), small.height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_come_from_content_for_images_and_names_otherwise() {
        let mut png = Vec::new();
        image::RgbImage::new(2, 1)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(mime_type("photo.jpg", &png), mime::IMAGE_PNG);
        assert_eq!(dimensions(&png), Some((2, 1)));
        assert_eq!(mime_type("clip.MP4", b"x").essence_str(), "video/mp4");
        assert_eq!(
            mime_type("notes", b"plain text"),
            mime::APPLICATION_OCTET_STREAM
        );
    }

    #[test]
    fn thumbnails_fit_the_box_and_keep_the_aspect_ratio() {
        let mut png = Vec::new();
        image::RgbImage::new(1600, 400)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (jpeg, w, h) = thumbnail(&png).unwrap();
        assert_eq!((w, h), (800, 200));
        assert_eq!(
            image::guess_format(&jpeg).unwrap(),
            image::ImageFormat::Jpeg
        );
    }

    #[test]
    fn encrypted_media_never_downloads_as_plain() {
        let sources = Sources::default();
        assert!(sources.get("mxc://s/unknown", true).is_none());
        assert!(matches!(
            sources.get("mxc://s/plain", false),
            Some(MediaSource::Plain(_))
        ));
        let file: matrix_sdk::ruma::events::room::EncryptedFile = serde_json::from_value(json!({
            "url": "mxc://s/secret", "v": "v2", "iv": "AAAAAAAAAAAAAAAAAAAAAA",
            "hashes": {"sha256": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"},
            "key": {"kty": "oct", "key_ops": ["encrypt", "decrypt"], "alg": "A256CTR",
                    "k": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "ext": true}
        }))
        .unwrap();
        let content: MessageType = serde_json::from_value(json!({
            "msgtype": "m.file", "body": "secret.bin", "file": file
        }))
        .unwrap();
        sources.remember(&content);
        assert!(matches!(
            sources.get("mxc://s/secret", true),
            Some(MediaSource::Encrypted(_))
        ));
        assert!(sources.get("mxc://s/secret", false).is_none());
    }
}
