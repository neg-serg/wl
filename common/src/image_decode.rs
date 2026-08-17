//! Image decoding module — converts image files to RGBA8 pixel data for GPU upload.
//!
//! Supports raster formats via the `image` crate and SVG via `resvg`.

use std::fmt;
use std::fs;
use std::io::{self, BufReader};
use std::path::Path;

use image::{AnimationDecoder, RgbaImage, imageops::FilterType};

use crate::ipc_types::ResizeMode;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Decoded raster image as raw RGBA8 pixel data.
pub struct DecodedImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A single GIF frame.
pub struct GifFrame {
    pub data: Vec<u8>,
    pub duration_ms: u32,
}

/// Header-level information about an animated GIF.
///
/// Obtained with [`gif_info`], which only reads the GIF header and frame
/// descriptors — it never decodes pixel data, so it is cheap even for very
/// long animations.
pub struct GifInfo {
    /// Logical screen width in pixels (all frames share this size).
    pub width: u32,
    /// Logical screen height in pixels.
    pub height: u32,
    /// Total number of frames in the animation.
    pub frame_count: usize,
    /// Display duration of every frame, in milliseconds, in frame order.
    pub durations_ms: Vec<u32>,
}

/// A lazy, frame-at-a-time GIF decoder.
///
/// Only one decoded frame is held in memory at a time; the previous frame's
/// buffer is dropped as soon as the next one is decoded. This keeps peak RAM
/// usage proportional to a *single* frame instead of the whole animation.
pub struct GifFrameStream {
    frames: image::Frames<'static>,
}

impl GifFrameStream {
    /// Decode and return the next frame, advancing the stream by one.
    ///
    /// Returns `None` once the animation has been fully read. The returned
    /// frame is RGBA8 at the GIF's logical screen size.
    pub fn next_frame(&mut self) -> Option<Result<GifFrame, DecodeError>> {
        let frame = self.frames.next()?;
        let frame = match frame {
            Ok(f) => f,
            Err(e) => return Some(Err(DecodeError::Image(e.to_string()))),
        };
        let (numer, denom) = frame.delay().numer_denom_ms();
        let duration_ms = numer.checked_div(denom).unwrap_or(0);
        let data = frame.into_buffer().into_raw();
        Some(Ok(GifFrame { data, duration_ms }))
    }
}

/// Errors that can occur during image decoding.
#[derive(Debug)]
pub enum DecodeError {
    Io(io::Error),
    Image(String),
    Svg(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Io(err) => write!(f, "I/O error: {err}"),
            DecodeError::Image(msg) => write!(f, "image decode error: {msg}"),
            DecodeError::Svg(msg) => write!(f, "SVG decode error: {msg}"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecodeError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for DecodeError {
    fn from(err: io::Error) -> Self {
        DecodeError::Io(err)
    }
}

impl From<image::ImageError> for DecodeError {
    fn from(err: image::ImageError) -> Self {
        DecodeError::Image(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// SVG helpers
// ---------------------------------------------------------------------------

/// Default viewport dimensions used when rasterizing SVGs.
const SVG_DEFAULT_WIDTH: u32 = 1920;
const SVG_DEFAULT_HEIGHT: u32 = 1080;

fn is_svg(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("svg" | "SVG" | "svgz" | "SVGZ")
    )
}

/// Rasterize an SVG file to RGBA8, scaling to fit inside the default viewport
/// while preserving aspect ratio.
fn decode_svg(path: &Path) -> Result<DecodedImage, DecodeError> {
    let data = fs::read(path)?;

    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .map_err(|e| DecodeError::Svg(e.to_string()))?;

    let svg_size = tree.size();
    let svg_w = svg_size.width();
    let svg_h = svg_size.height();

    // Compute scale factor so the SVG fits inside the default viewport.
    let scale = (SVG_DEFAULT_WIDTH as f32 / svg_w).min(SVG_DEFAULT_HEIGHT as f32 / svg_h);
    let px_w = (svg_w * scale).round() as u32;
    let px_h = (svg_h * scale).round() as u32;

    if px_w == 0 || px_h == 0 {
        return Err(DecodeError::Svg("SVG has zero-size dimensions".into()));
    }

    let mut pixmap = resvg::tiny_skia::Pixmap::new(px_w, px_h)
        .ok_or_else(|| DecodeError::Svg("failed to create pixmap".into()))?;

    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny_skia stores pixels as premultiplied RGBA; we need straight RGBA8.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|px| {
            let r = px.red();
            let g = px.green();
            let b = px.blue();
            let a = px.alpha();
            if a == 0 || a == 255 {
                [r, g, b, a]
            } else {
                // Un-premultiply.
                let af = a as f32 / 255.0;
                [
                    (r as f32 / af).round().min(255.0) as u8,
                    (g as f32 / af).round().min(255.0) as u8,
                    (b as f32 / af).round().min(255.0) as u8,
                    a,
                ]
            }
        })
        .collect::<Vec<u8>>();

    Ok(DecodedImage {
        data: rgba,
        width: px_w,
        height: px_h,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the dimensions an image of `src_w`×`src_h` would have after
/// [`resize_for_output`] with the given mode and target resolution.
///
/// This lets callers know the resized frame size *before* decoding any pixel
/// data (e.g. to budget GPU memory for a GIF atlas upfront).
pub fn resize_output_dims(
    src_w: u32,
    src_h: u32,
    target_w: u32,
    target_h: u32,
    mode: ResizeMode,
) -> (u32, u32) {
    if mode == ResizeMode::No || (src_w == target_w && src_h == target_h) {
        return (src_w, src_h);
    }
    match mode {
        ResizeMode::Center | ResizeMode::Crop => (target_w, target_h),
        ResizeMode::Fit => {
            let scale_x = target_w as f64 / src_w as f64;
            let scale_y = target_h as f64 / src_h as f64;
            let scale = scale_x.min(scale_y);
            (
                ((src_w as f64 * scale).round() as u32).max(1),
                ((src_h as f64 * scale).round() as u32).max(1),
            )
        }
        ResizeMode::No => (src_w, src_h),
    }
}

/// Resize a decoded image to match the output's effective resolution.
///
/// - **Crop**: Center-crop the source to fill the target aspect ratio, then resize
///   to target dimensions.
/// - **Fit**: Scale to fit within target dimensions while preserving aspect ratio.
/// - **No**: Return the image unchanged.
///
/// If the source dimensions already match the target, the image is returned as-is
/// (zero-loss passthrough).
pub fn resize_for_output(
    img: DecodedImage,
    target_w: u32,
    target_h: u32,
    mode: ResizeMode,
) -> DecodedImage {
    if mode == ResizeMode::No || (img.width == target_w && img.height == target_h) {
        return img;
    }

    let src = RgbaImage::from_raw(img.width, img.height, img.data)
        .expect("DecodedImage data length must match width*height*4");

    match mode {
        ResizeMode::Center => {
            let mut canvas = RgbaImage::from_pixel(target_w, target_h, image::Rgba([0, 0, 0, 255]));
            let paste_w = img.width.min(target_w);
            let paste_h = img.height.min(target_h);
            let dst_x = (target_w.saturating_sub(paste_w)) / 2;
            let dst_y = (target_h.saturating_sub(paste_h)) / 2;
            let src_x = (img.width.saturating_sub(target_w)) / 2;
            let src_y = (img.height.saturating_sub(target_h)) / 2;
            let cropped = image::imageops::crop_imm(&src, src_x, src_y, paste_w, paste_h);
            image::imageops::overlay(&mut canvas, &*cropped, dst_x as i64, dst_y as i64);
            DecodedImage {
                data: canvas.into_raw(),
                width: target_w,
                height: target_h,
            }
        }
        ResizeMode::Crop => {
            let src_aspect = img.width as f64 / img.height as f64;
            let tgt_aspect = target_w as f64 / target_h as f64;

            let (crop_w, crop_h) = if src_aspect > tgt_aspect {
                // Source is wider — crop horizontally
                let w = (img.height as f64 * tgt_aspect).round() as u32;
                (w.min(img.width), img.height)
            } else {
                // Source is taller — crop vertically
                let h = (img.width as f64 / tgt_aspect).round() as u32;
                (img.width, h.min(img.height))
            };

            let crop_x = (img.width.saturating_sub(crop_w)) / 2;
            let crop_y = (img.height.saturating_sub(crop_h)) / 2;

            let cropped = image::imageops::crop_imm(&src, crop_x, crop_y, crop_w, crop_h);
            let resized =
                image::imageops::resize(&*cropped, target_w, target_h, FilterType::CatmullRom);
            let (w, h) = resized.dimensions();
            DecodedImage {
                data: resized.into_raw(),
                width: w,
                height: h,
            }
        }
        ResizeMode::Fit => {
            let (fit_w, fit_h) = resize_output_dims(img.width, img.height, target_w, target_h, mode);
            let resized =
                image::imageops::resize(&src, fit_w.max(1), fit_h.max(1), FilterType::CatmullRom);
            let (w, h) = resized.dimensions();
            DecodedImage {
                data: resized.into_raw(),
                width: w,
                height: h,
            }
        }
        ResizeMode::No => unreachable!(),
    }
}

/// Decode an image file at `path` to raw RGBA8 pixel data.
///
/// SVG/SVGZ files are rasterized via `resvg` into a 1920x1080 viewport (scaled
/// to fit while preserving aspect ratio). All other formats are decoded with the
/// `image` crate.
pub fn decode_to_rgba8(path: &Path) -> Result<DecodedImage, DecodeError> {
    if is_svg(path) {
        return decode_svg(path);
    }

    let img = image::open(path)?.to_rgba8();
    let (width, height) = img.dimensions();
    let data = img.into_raw();

    Ok(DecodedImage {
        data,
        width,
        height,
    })
}

/// Read header-level information about an animated GIF at `path`.
///
/// This only decodes frame *metadata* (count and display delays) — no pixel
/// data is decompressed — so it is cheap even for long animations and can be
/// used to decide how many frames fit a memory budget before decoding.
pub fn gif_info(path: &Path) -> Result<GifInfo, DecodeError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);

    let mut decoder = gif::DecodeOptions::new()
        .read_info(reader)
        .map_err(|e| DecodeError::Image(e.to_string()))?;

    let width = u32::from(decoder.width());
    let height = u32::from(decoder.height());

    let mut frame_count = 0usize;
    let mut durations_ms = Vec::new();
    while let Some(frame) = decoder
        .next_frame_info()
        .map_err(|e| DecodeError::Image(e.to_string()))?
    {
        frame_count += 1;
        // GIF delays are in units of 10 ms (image crate converts identically:
        // delay * 10 ms), so this matches the durations reported per decoded frame.
        durations_ms.push(u32::from(frame.delay) * 10);
    }

    if frame_count == 0 {
        return Err(DecodeError::Image("GIF contains no frames".into()));
    }

    Ok(GifInfo {
        width,
        height,
        frame_count,
        durations_ms,
    })
}

/// Open an animated GIF at `path` for frame-at-a-time decoding.
///
/// Decode frames with [`GifFrameStream::next_frame`]; the stream holds at most
/// one decoded frame in memory at a time, so memory usage stays proportional to
/// a single frame regardless of how long the animation is.
pub fn gif_frame_stream(path: &Path) -> Result<GifFrameStream, DecodeError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);

    let decoder = image::codecs::gif::GifDecoder::new(reader)
        .map_err(|e| DecodeError::Image(e.to_string()))?;

    // `into_frames` hands back an iterator that owns the decoder, so the
    // stream is `'static` and can live independently of this function.
    let frames: image::Frames<'static> = decoder.into_frames();

    Ok(GifFrameStream { frames })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resize(src: (u32, u32), target: (u32, u32), mode: ResizeMode) -> (u32, u32) {
        resize_output_dims(src.0, src.1, target.0, target.1, mode)
    }

    #[test]
    fn resize_output_dims_no_mode_passthrough() {
        let src = (1920, 1080);
        assert_eq!(resize(src, (1280, 720), ResizeMode::No), src);
        // Passthrough also when dims already match the target.
        assert_eq!(resize(src, (1920, 1080), ResizeMode::Crop), src);
        assert_eq!(resize(src, (1920, 1080), ResizeMode::Fit), src);
        assert_eq!(resize(src, (1920, 1080), ResizeMode::Center), src);
    }

    #[test]
    fn resize_output_dims_crop_and_center_fill_target() {
        assert_eq!(resize((1920, 1080), (1280, 720), ResizeMode::Crop), (1280, 720));
        assert_eq!(resize((1000, 500), (100, 200), ResizeMode::Crop), (100, 200));
        assert_eq!(resize((1000, 500), (100, 200), ResizeMode::Center), (100, 200));
    }

    #[test]
    fn resize_output_dims_fit_preserves_aspect() {
        // Same aspect: exact fit.
        assert_eq!(resize((1920, 1080), (1280, 720), ResizeMode::Fit), (1280, 720));
        // Wider than target: fit by width.
        assert_eq!(resize((1000, 500), (100, 200), ResizeMode::Fit), (100, 50));
        // Taller than target: fit by height.
        assert_eq!(resize((500, 1000), (100, 200), ResizeMode::Fit), (100, 200));
        // Rounding keeps at least 1px.
        assert_eq!(resize((3, 3), (100, 100), ResizeMode::Fit), (100, 100));
    }

    /// Encode a small animated GIF with `frame_count` frames of `width`x`height`,
    /// each a solid color, with the given per-frame delays (in centiseconds).
    fn write_test_gif(
        dir: &Path,
        name: &str,
        width: u16,
        height: u16,
        frame_colors: &[(u8, u8, u8)],
        delays_cs: &[u16],
    ) -> std::path::PathBuf {
        let path = dir.join(name);
        let file = fs::File::create(&path).unwrap();
        let mut encoder = gif::Encoder::new(file, width, height, &[]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for (i, &(r, g, b)) in frame_colors.iter().enumerate() {
            let rgba: Vec<u8> = (0..(width as usize * height as usize))
                .flat_map(|_| [r, g, b, 255])
                .collect();
            let mut frame = gif::Frame::from_rgba_speed(
                width,
                height,
                &mut rgba.clone(),
                10, // default quantization speed
            );
            frame.delay = delays_cs[i];
            encoder.write_frame(&frame).unwrap();
        }
        path
    }

    #[test]
    fn gif_info_reads_header_without_pixels() {
        let dir = std::env::temp_dir().join(format!("wl-gif-info-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = write_test_gif(
            &dir,
            "info.gif",
            6,
            4,
            &[(10, 0, 0), (20, 0, 0), (30, 0, 0), (40, 0, 0)],
            &[3, 5, 7, 11], // centiseconds -> 30, 50, 70, 110 ms
        );

        let info = gif_info(&path).unwrap();
        assert_eq!(info.width, 6);
        assert_eq!(info.height, 4);
        assert_eq!(info.frame_count, 4);
        assert_eq!(info.durations_ms, vec![30, 50, 70, 110]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gif_frame_stream_decodes_one_frame_at_a_time() {
        let dir = std::env::temp_dir().join(format!("wl-gif-stream-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = write_test_gif(
            &dir,
            "stream.gif",
            6,
            4,
            &[(10, 20, 30), (40, 50, 60), (70, 80, 90), (100, 110, 120)],
            &[3, 5, 7, 11],
        );

        let mut stream = gif_frame_stream(&path).unwrap();
        let mut count = 0;
        let mut durations = Vec::new();
        while let Some(frame) = stream.next_frame() {
            let frame = frame.unwrap();
            assert_eq!(frame.data.len(), 6 * 4 * 4);
            // Every pixel is the frame's solid color (opaque).
            for px in frame.data.chunks_exact(4) {
                assert_eq!(
                    px,
                    &[10 + 30 * count as u8, 20 + 30 * count as u8, 30 + 30 * count as u8, 255]
                );
            }
            durations.push(frame.duration_ms);
            count += 1;
        }
        assert_eq!(count, 4);
        assert_eq!(durations, vec![30, 50, 70, 110]);

        // Stream durations agree with the header pass.
        let info = gif_info(&path).unwrap();
        assert_eq!(info.durations_ms, durations);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gif_info_rejects_file_without_frames() {
        let dir = std::env::temp_dir().join(format!("wl-gif-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.gif");
        fs::write(&path, b"not a real gif").unwrap();
        assert!(gif_info(&path).is_err());
        fs::remove_dir_all(&dir).ok();
    }
}
