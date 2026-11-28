//! Small local-only thumbnails, and the decoding of fetched pictures: a still
//! image, or an animated GIF or WebP's frames (ROADMAP P1).
use crate::media::LocalFile;
use iced::widget::image::Handle;
use std::{sync::Arc, time::Duration};

/// A decoded picture: still, or animated.
#[derive(Clone, Debug)]
pub enum Picture {
    Still(Handle),
    /// Desktop only: the browser decodes pictures still.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Moving(Arc<Frames>),
}
impl Picture {
    /// The picture, or an animation's first frame.
    pub fn still(&self) -> &Handle {
        match self {
            Self::Still(h) => h,
            Self::Moving(f) => &f.frames[0].0,
        }
    }
}

/// An animation's frames, each with how long it shows; never empty.
#[derive(Debug)]
pub struct Frames {
    pub frames: Vec<(Handle, Duration)>,
}

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

/// Frames an animation keeps at most.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FRAMES: usize = 300;

/// The memory an animation's frames may take, by what they're for.
#[cfg(not(target_arch = "wasm32"))]
fn frame_budget(fit: crate::media::Fit) -> usize {
    use crate::media::Fit;
    match fit {
        Fit::Avatar | Fit::Emote => 4 << 20,
        Fit::Inline => 16 << 20,
        Fit::Full | Fit::Zoom => 64 << 20,
    }
}

/// An animated GIF or WebP's frames within `width`×`height`, each scaled down
/// as it's decoded, stopping early past `budget` bytes. None for a still
/// picture (or one frame).
#[cfg(not(target_arch = "wasm32"))]
pub fn animation(bytes: &[u8], width: u32, height: u32, budget: usize) -> Option<Frames> {
    use image::{AnimationDecoder, ImageFormat, codecs, imageops};
    use std::io::Cursor;
    let frames: image::Frames = match image::guess_format(bytes).ok()? {
        ImageFormat::Gif => codecs::gif::GifDecoder::new(Cursor::new(bytes))
            .ok()?
            .into_frames(),
        ImageFormat::WebP => {
            let decoder = codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            decoder.into_frames()
        }
        _ => return None,
    };
    let mut kept = Vec::new();
    let mut used = 0;
    for frame in frames.take(MAX_FRAMES) {
        let frame = frame.ok()?;
        let delay = Duration::from(frame.delay());
        let buffer = frame.into_buffer();
        let (w, h) = buffer.dimensions();
        if !valid_size(w, h) {
            return None;
        }
        let scale = (width as f32 / w as f32)
            .min(height as f32 / h as f32)
            .min(1.);
        let (tw, th) = (
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
        );
        let small = if (tw, th) == (w, h) {
            buffer
        } else {
            imageops::thumbnail(&buffer, tw, th)
        };
        used += small.as_raw().len();
        if used > budget && kept.len() > 1 {
            break;
        }
        // Like browsers, a delay too short to mean anything is a tenth of a second.
        let delay = if delay < Duration::from_millis(20) {
            Duration::from_millis(100)
        } else {
            delay
        };
        kept.push((Handle::from_rgba(tw, th, small.into_raw()), delay));
    }
    (kept.len() > 1).then_some(Frames { frames: kept })
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn inline(bytes: Vec<u8>, fit: crate::media::Fit) -> Option<Picture> {
    let _permit = DECODERS.acquire().await;
    let (width, height) = fit.bounds();
    tokio::task::spawn_blocking(move || {
        if let Some(frames) = animation(&bytes, width, height, frame_budget(fit)) {
            return Some(Picture::Moving(Arc::new(frames)));
        }
        decode(std::io::Cursor::new(bytes), width, height).map(Picture::Still)
    })
    .await
    .ok()
    .flatten()
}

/// An inline timeline image from fetched bytes, decoded by the browser.
#[cfg(target_arch = "wasm32")]
pub async fn inline(bytes: Vec<u8>, fit: crate::media::Fit) -> Option<Handle> {
    let _permit = DECODERS.acquire().await;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;
    let (width, height) = fit.bounds();
    blob_thumbnail(&blob, width, height).await
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
    let (w, h): (u32, u32) = (dims.width.try_into().ok()?, dims.height.try_into().ok()?);
    if !valid_size(w, h) {
        return None;
    }
    // The picture's own shape within the bounds, never enlarged, so the
    // canvas has no empty borders.
    let scale = (f64::from(width) / f64::from(w))
        .min(f64::from(height) / f64::from(h))
        .min(1.);
    let width = ((f64::from(w) * scale).round() as u32).max(1);
    let height = ((f64::from(h) * scale).round() as u32).max(1);
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod animation_tests {
    use super::*;

    fn gif(frames: usize, size: u32) -> Vec<u8> {
        use image::{Delay, Frame, RgbaImage, codecs::gif::GifEncoder};
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            for n in 0..frames {
                let shade = (n * 60 % 256) as u8;
                let image = RgbaImage::from_pixel(size, size, image::Rgba([shade, 0, 0, 255]));
                encoder
                    .encode_frame(Frame::from_parts(
                        image,
                        0,
                        0,
                        Delay::from_numer_denom_ms(80, 1),
                    ))
                    .unwrap();
            }
        }
        out
    }

    #[test]
    fn an_animated_gif_keeps_its_frames_and_delays_fitted() {
        let frames = animation(&gif(3, 200), 100, 100, 1 << 20).expect("animated");
        assert_eq!(frames.frames.len(), 3);
        assert!(
            frames
                .frames
                .iter()
                .all(|(_, d)| *d == Duration::from_millis(80))
        );
        let Handle::Rgba { width, height, .. } = &frames.frames[0].0 else {
            panic!("rgba frames")
        };
        assert_eq!((*width, *height), (100, 100));
    }

    #[test]
    fn a_budget_stops_an_animation_early_and_a_still_is_no_animation() {
        // 100×100 RGBA is 40 000 bytes a frame: the budget fits two.
        let frames = animation(&gif(5, 100), 100, 100, 80_000).expect("animated");
        assert_eq!(frames.frames.len(), 2);
        assert!(animation(&gif(1, 50), 100, 100, 1 << 20).is_none());
        let mut png = Vec::new();
        image::RgbaImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert!(animation(&png, 100, 100, 1 << 20).is_none());
    }
}
