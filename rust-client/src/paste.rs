//! User-initiated paste into the native message composer (never the login form).
use crate::media::{LocalFile, NativeSource};
use image::ImageEncoder;
use std::io::Write;

#[derive(Clone)]
pub enum Content {
    Image(LocalFile),
    Text(String),
    Empty,
}

pub type Result = std::result::Result<Content, &'static str>;
const TOO_LARGE: &str = "The clipboard image is too large to attach.";
const UNAVAILABLE: &str = "Could not read the clipboard. Try copying the screenshot again.";
const MAX_PIXELS: usize = 24_000_000;
const MAX_TEXT: usize = 4 << 20;

fn dimensions(w: usize, h: usize) -> bool {
    w > 0
        && h > 0
        && w <= 16_384
        && h <= 16_384
        && w.checked_mul(h).is_some_and(|n| n <= MAX_PIXELS)
}

fn screenshot(png: Vec<u8>, limit: usize) -> Result {
    if png.len() > limit {
        return Err(TOO_LARGE);
    }
    Ok(Content::Image(LocalFile {
        name: format!(
            "Screenshot-{}.png",
            chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
        ),
        size: png.len() as u64,
        origin: NativeSource::Clipboard(png.into()),
    }))
}

fn text(s: String) -> Result {
    if s.len() > MAX_TEXT {
        return Err("Clipboard text is too large to paste.");
    }
    Ok(Content::Text(s.replace("\r\n", "\n").replace('\r', "\n")))
}

struct PngBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for PngBuffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > self.limit - self.bytes.len() {
            return Err(std::io::Error::other(TOO_LARGE));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn portable(limit: usize) -> Result {
    let mut clip = arboard::Clipboard::new().map_err(|_| UNAVAILABLE)?;
    match clip.get_image() {
        Ok(img) => {
            if !dimensions(img.width, img.height) || img.bytes.len() != img.width * img.height * 4 {
                return Err(TOO_LARGE);
            }
            let mut png = PngBuffer {
                bytes: Vec::new(),
                limit,
            };
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(
                    &img.bytes,
                    img.width as u32,
                    img.height as u32,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|_| "Could not encode the screenshot within the upload limit.")?;
            screenshot(png.bytes, limit)
        }
        // X11 owners such as xclip may return "incorrect type" rather than
        // "content not available" when only text was offered. Check text too.
        Err(image_error) => match clip.get_text() {
            Ok(s) => text(s),
            Err(arboard::Error::ContentNotAvailable)
                if matches!(image_error, arboard::Error::ContentNotAvailable) =>
            {
                Ok(Content::Empty)
            }
            Err(_) => Err(UNAVAILABLE),
        },
    }
}

pub async fn read(limit: u64) -> Result {
    let limit = limit.min(crate::media::MAX_BYTES) as usize;
    #[cfg(target_os = "linux")]
    if let Some(offer) = tokio::task::spawn_blocking(smithay_clipboard::composer_offer)
        .await
        .map_err(|_| UNAVAILABLE)?
    {
        use smithay_clipboard::PasteOffer;
        use tokio::io::AsyncReadExt;
        let (fd, png) = match offer.map_err(|_| UNAVAILABLE)? {
            Some(PasteOffer::Png(fd)) => (fd, true),
            Some(PasteOffer::Text(fd)) => (fd, false),
            None => return Ok(Content::Empty),
        };
        let cap = if png { limit } else { MAX_TEXT };
        let mut data = Vec::new();
        let pipe = tokio::net::unix::pipe::Receiver::from_owned_fd(fd).map_err(|_| UNAVAILABLE)?;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            pipe.take(cap as u64 + 1).read_to_end(&mut data),
        )
        .await
        .map_err(|_| UNAVAILABLE)?
        .map_err(|_| UNAVAILABLE)?;
        if !png {
            return text(String::from_utf8(data).map_err(|_| UNAVAILABLE)?);
        }
        if data.len() > cap {
            return Err(TOO_LARGE);
        }
        // Already encoded PNG from the compositor: keep the original bytes.
        let (w, h) =
            image::ImageReader::with_format(std::io::Cursor::new(&data), image::ImageFormat::Png)
                .into_dimensions()
                .map_err(|_| "Clipboard did not contain a readable PNG.")?;
        if !dimensions(w as usize, h as usize) {
            return Err(TOO_LARGE);
        }
        return screenshot(data, limit);
    }
    tokio::task::spawn_blocking(move || portable(limit))
        .await
        .map_err(|_| UNAVAILABLE)?
}
