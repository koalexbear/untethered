//! Small local-only thumbnails. Full-size originals still go to gomuks on Send.
use crate::media::LocalFile;
use iced::widget::image::Handle;

const MAX_SOURCE_BYTES: u64 = 24 << 20;
const MAX_PIXELS: u64 = 24_000_000;
const WIDTH: u32 = 272;
const HEIGHT: u32 = 152;
static DECODERS: async_lock::Semaphore = async_lock::Semaphore::new(2);

pub fn candidate(file: &LocalFile) -> bool {
    let ext = file
        .name
        .rsplit_once('.')
        .map_or(String::new(), |(_, ext)| ext.to_ascii_lowercase());
    let image = matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif");
    #[cfg(target_arch = "wasm32")]
    let image = image
        || matches!(
            file.file.type_().as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        );
    image && file.size <= MAX_SOURCE_BYTES
}

fn valid_size(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= 16_384 && h <= 16_384 && u64::from(w) * u64::from(h) <= MAX_PIXELS
}

pub async fn load(file: LocalFile) -> Option<Handle> {
    let _permit = DECODERS.acquire().await;
    #[cfg(not(target_arch = "wasm32"))]
    return tokio::task::spawn_blocking(move || native_thumbnail(&file))
        .await
        .ok()
        .flatten();
    #[cfg(target_arch = "wasm32")]
    return browser_thumbnail(&file).await;
}

#[cfg(not(target_arch = "wasm32"))]
fn native_thumbnail(source: &LocalFile) -> Option<Handle> {
    if let crate::media::NativeSource::Clipboard(bytes) = &source.origin {
        return decode(std::io::Cursor::new(bytes.as_ref()), WIDTH, HEIGHT);
    }
    let f = std::fs::File::open(source.origin.read_path()?).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() || meta.len() != source.size || meta.len() > MAX_SOURCE_BYTES {
        return None;
    }
    decode(std::io::BufReader::new(f), WIDTH, HEIGHT)
}

#[cfg(not(target_arch = "wasm32"))]
fn decode(input: impl std::io::BufRead + std::io::Seek, width: u32, height: u32) -> Option<Handle> {
    use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
    let mut reader = ImageReader::new(input).with_guessed_format().ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(96 << 20);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let (w, h) = decoder.dimensions();
    if !valid_size(w, h) || decoder.total_bytes() > 96 << 20 {
        return None;
    }
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let small = if img.width() > width || img.height() > height {
        img.thumbnail(width, height)
    } else {
        img
    }
    .into_rgba8();
    Some(Handle::from_rgba(
        small.width(),
        small.height(),
        small.into_raw(),
    ))
}

pub const INLINE_WIDTH: u32 = 400;
pub const INLINE_HEIGHT: u32 = 260;

#[cfg(not(target_arch = "wasm32"))]
pub async fn inline(bytes: Vec<u8>) -> Option<Handle> {
    let _permit = DECODERS.acquire().await;
    tokio::task::spawn_blocking(move || {
        decode(std::io::Cursor::new(bytes), INLINE_WIDTH, INLINE_HEIGHT)
    })
    .await
    .ok()
    .flatten()
}

/// An inline timeline image from fetched bytes, decoded by the browser.
#[cfg(target_arch = "wasm32")]
pub async fn inline(bytes: Vec<u8>) -> Option<Handle> {
    let _permit = DECODERS.acquire().await;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;
    blob_thumbnail(&blob, INLINE_WIDTH, INLINE_HEIGHT).await
}

#[cfg(target_arch = "wasm32")]
async fn browser_thumbnail(source: &LocalFile) -> Option<Handle> {
    blob_thumbnail(&source.file, WIDTH, HEIGHT).await
}
#[cfg(target_arch = "wasm32")]
pub async fn blob_thumbnail(source: &web_sys::Blob, width: u32, height: u32) -> Option<Handle> {
    use wasm_bindgen_futures::JsFuture;
    // Read only the header before letting the browser decode the raster. In
    // particular a renamed SVG or a huge raster never enters the drawing path.
    let head = source.slice_with_f64_and_f64(0., 262144.).ok()?;
    let bytes = js_sys::Uint8Array::new(&JsFuture::from(head.array_buffer()).await.ok()?).to_vec();
    let dims = imagesize::blob_size(&bytes).ok()?;
    if !valid_size(dims.width.try_into().ok()?, dims.height.try_into().ok()?) {
        return None;
    }
    let pixels = crate::web_files::thumbnail(source, width, height)
        .await
        .ok()?;
    if pixels.length() != width * height * 4 {
        return None;
    }
    Some(Handle::from_rgba(width, height, pixels.to_vec()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn native_preview_resizes_real_pixels_without_modifying_the_original() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("preview.png");
        image::RgbaImage::from_pixel(600, 100, image::Rgba([23, 77, 199, 255]))
            .save(&path)
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let file = LocalFile::from_path(path.clone()).unwrap();
        let Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = native_thumbnail(&file).unwrap()
        else {
            panic!("not a bitmap")
        };
        assert_eq!(width, WIDTH);
        assert!((44..=46).contains(&height));
        assert_eq!(&pixels[..4], &[23, 77, 199, 255]);
        assert_eq!(original, std::fs::read(path).unwrap());
    }
}
