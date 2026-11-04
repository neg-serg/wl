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

/// Largest image side accepted for decoding (16K wallpapers still fit).
pub const MAX_DECODE_DIMENSION: u32 = 16_384;

/// Decode limits for image files.
///
/// The `image` crate defaults allow any dimensions as long as a single
/// allocation stays under 512 MiB, so a small PNG/GIF claiming 65535x65535 can
/// make the daemon try to allocate several gigabytes. Refuse pathological
/// dimensions and cap total decoder allocations instead.
#[allow(clippy::field_reassign_with_default)] // Limits is #[non_exhaustive]
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIMENSION);
    limits.max_image_height = Some(MAX_DECODE_DIMENSION);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    limits
}

/// Default viewport dimensions used when rasterizing SVGs.
const SVG_DEFAULT_WIDTH: u32 = 1920;
const SVG_DEFAULT_HEIGHT: u32 = 1080;

fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("svg") || extension.eq_ignore_ascii_case("svgz")
        })
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
    _mode: ResizeMode,
) -> (u32, u32) {
    if src_w == target_w && src_h == target_h {
        return (src_w, src_h);
    }
    // Every mode produces a texture at the target size: crop fills it, fit
    // letterboxes onto it, center/no paste native pixels onto it. Keeping that
    // invariant means the GPU draw is always 1:1 and the sampler never has to
    // rescale the wallpaper — which is where detail used to get lost.
    (target_w, target_h)
}

/// Dimensions of a `src_w`×`src_h` image scaled to fit inside `target_w`×`target_h`
/// without changing its aspect ratio (rounded, at least 1px).
fn fitted_dims(src_w: u32, src_h: u32, target_w: u32, target_h: u32) -> (u32, u32) {
    let scale = (target_w as f64 / src_w as f64).min(target_h as f64 / src_h as f64);
    (
        ((src_w as f64 * scale).round() as u32).max(1),
        ((src_h as f64 * scale).round() as u32).max(1),
    )
}

/// Resize `src` to `target_w`×`target_h` with a filter chosen by direction.
///
/// `image`'s `resize` scales its kernel with the ratio, so a single call is
/// already a correct resampler; the filter choice decides how sharp versus how
/// ring-free the result is. Lanczos3 keeps more detail when shrinking (that is
/// the common case: a 6K/8K source on a 4K output), while CatmullRom is the
/// gentler choice when enlarging a small image.
fn resample(src: &RgbaImage, target_w: u32, target_h: u32) -> RgbaImage {
    let target_w = target_w.max(1);
    let target_h = target_h.max(1);
    let shrinking = target_w < src.width() || target_h < src.height();
    let filter = if shrinking {
        FilterType::Lanczos3
    } else {
        FilterType::CatmullRom
    };
    image::imageops::resize(src, target_w, target_h, filter)
}

/// Resize a decoded image to the output's effective resolution.
///
/// Every mode returns a texture sized exactly `target_w`×`target_h`, so the
/// draw is 1:1 and nothing is rescaled on the GPU:
///
/// - **Crop**: center-crop the source to the target aspect ratio, then scale it
///   up/down to fill the canvas.
/// - **Fit**: scale to fit inside the target, centered on a black canvas
///   (letterbox/pillarbox).
/// - **Center**: paste the source at its native size, centered; overflow is
///   cropped and the rest stays black.
/// - **No**: same as **Center** — native pixels, no scaling.
///
/// If the source dimensions already match the target, the image is returned as-is
/// (zero-loss passthrough).
pub fn resize_for_output(
    img: DecodedImage,
    target_w: u32,
    target_h: u32,
    mode: ResizeMode,
) -> DecodedImage {
    if img.width == target_w && img.height == target_h {
        return img;
    }

    let src = RgbaImage::from_raw(img.width, img.height, img.data)
        .expect("DecodedImage data length must match width*height*4");

    match mode {
        // No scaling: native pixels centered, overflow cropped.
        ResizeMode::Center | ResizeMode::No => {
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

            let cropped =
                image::imageops::crop_imm(&src, crop_x, crop_y, crop_w, crop_h).to_image();
            let resized = resample(&cropped, target_w, target_h);
            DecodedImage {
                data: resized.into_raw(),
                width: target_w,
                height: target_h,
            }
        }
        ResizeMode::Fit => {
            let (fit_w, fit_h) = fitted_dims(img.width, img.height, target_w, target_h);
            let resized = resample(&src, fit_w, fit_h);
            let mut canvas = RgbaImage::from_pixel(target_w, target_h, image::Rgba([0, 0, 0, 255]));
            let dst_x = (target_w.saturating_sub(fit_w)) / 2;
            let dst_y = (target_h.saturating_sub(fit_h)) / 2;
            image::imageops::overlay(&mut canvas, &resized, dst_x as i64, dst_y as i64);
            DecodedImage {
                data: canvas.into_raw(),
                width: target_w,
                height: target_h,
            }
        }
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

    use image::ImageDecoder as _;

    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(decode_limits());

    // Phone cameras frequently store the photo unrotated and record the camera
    // orientation in EXIF; ignoring it showed every portrait wallpaper sideways.
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);

    let img = img.to_rgba8();
    let (width, height) = img.dimensions();
    let data = img.into_raw();

    Ok(DecodedImage {
        data,
        width,
        height,
    })
}

/// EXIF orientation of an image, without decoding its pixels.
///
/// Defaults to [`image::metadata::Orientation::NoTransforms`] for formats
/// without orientation metadata (PNG, GIF, …).
pub fn read_orientation(path: &Path) -> Result<image::metadata::Orientation, DecodeError> {
    use image::ImageDecoder as _;

    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    decoder.orientation().map_err(Into::into)
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

    use image::ImageDecoder as _;

    let mut decoder = image::codecs::gif::GifDecoder::new(reader)
        .map_err(|e| DecodeError::Image(e.to_string()))?;
    // A GIF's logical screen can claim up to 65535x65535 pixels; without this
    // the compositor tries to allocate the whole canvas.
    decoder
        .set_limits(decode_limits())
        .map_err(|e| DecodeError::Image(format!("GIF exceeds decode limits: {e}")))?;

    // `into_frames` hands back an iterator that owns the decoder, so the
    // stream is `'static` and can live independently of this function.
    let frames: image::Frames<'static> = decoder.into_frames();

    Ok(GifFrameStream { frames })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::jpeg_with_orientation;

    fn resize(src: (u32, u32), target: (u32, u32), mode: ResizeMode) -> (u32, u32) {
        resize_output_dims(src.0, src.1, target.0, target.1, mode)
    }

    #[test]
    fn resize_output_dims_passthrough_when_dims_match() {
        let src = (1920, 1080);
        assert_eq!(resize(src, (1920, 1080), ResizeMode::Crop), src);
        assert_eq!(resize(src, (1920, 1080), ResizeMode::Fit), src);
        assert_eq!(resize(src, (1920, 1080), ResizeMode::Center), src);
        assert_eq!(resize(src, (1920, 1080), ResizeMode::No), src);
    }

    #[test]
    fn resize_output_dims_every_mode_fills_target() {
        // Crop fills, fit letterboxes, center/no paste — all end up target-sized,
        // so the GPU draw stays 1:1.
        assert_eq!(
            resize((1920, 1080), (1280, 720), ResizeMode::Crop),
            (1280, 720)
        );
        assert_eq!(
            resize((1000, 500), (100, 200), ResizeMode::Crop),
            (100, 200)
        );
        assert_eq!(
            resize((1000, 500), (100, 200), ResizeMode::Center),
            (100, 200)
        );
        assert_eq!(resize((1000, 500), (100, 200), ResizeMode::No), (100, 200));
        assert_eq!(
            resize((1920, 1080), (1280, 720), ResizeMode::Fit),
            (1280, 720)
        );
        assert_eq!(resize((1000, 500), (100, 200), ResizeMode::Fit), (100, 200));
        assert_eq!(resize((3, 3), (100, 100), ResizeMode::Fit), (100, 100));
    }

    #[test]
    fn fitted_dims_preserves_aspect() {
        assert_eq!(fitted_dims(1920, 1080, 1280, 720), (1280, 720));
        assert_eq!(fitted_dims(1000, 500, 100, 200), (100, 50));
        assert_eq!(fitted_dims(500, 1000, 100, 200), (100, 200));
        assert_eq!(fitted_dims(3, 3, 100, 100), (100, 100));
    }

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> DecodedImage {
        DecodedImage {
            data: rgba
                .iter()
                .copied()
                .cycle()
                .take((width * height * 4) as usize)
                .collect(),
            width,
            height,
        }
    }

    fn pixel(img: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
        let off = ((y * img.width + x) * 4) as usize;
        [
            img.data[off],
            img.data[off + 1],
            img.data[off + 2],
            img.data[off + 3],
        ]
    }

    #[test]
    fn resize_no_and_center_paste_native_pixels() {
        for mode in [ResizeMode::No, ResizeMode::Center] {
            let img = solid(100, 50, [10, 20, 30, 255]);
            let out = resize_for_output(img, 200, 200, mode);
            assert_eq!((out.width, out.height), (200, 200));
            // Center of the canvas is the image itself…
            assert_eq!(pixel(&out, 100, 100), [10, 20, 30, 255]);
            // …and the untouched frame is black.
            assert_eq!(pixel(&out, 5, 5), [0, 0, 0, 255]);
        }
    }

    #[test]
    fn resize_center_crops_overflow_without_scaling() {
        let img = solid(400, 200, [1, 2, 3, 255]);
        let out = resize_for_output(img, 200, 100, ResizeMode::Center);
        assert_eq!((out.width, out.height), (200, 100));
        assert_eq!(pixel(&out, 0, 0), [1, 2, 3, 255]);
        assert_eq!(pixel(&out, 199, 99), [1, 2, 3, 255]);
    }

    #[test]
    fn resize_fit_letterboxes_onto_black_canvas() {
        // 100x50 into 100x200 → 100x50 content, 75px black bars above and below.
        let img = solid(100, 50, [9, 9, 9, 255]);
        let out = resize_for_output(img, 100, 200, ResizeMode::Fit);
        assert_eq!((out.width, out.height), (100, 200));
        assert_eq!(pixel(&out, 50, 10), [0, 0, 0, 255]);
        assert_eq!(pixel(&out, 50, 100), [9, 9, 9, 255]);
    }

    #[test]
    fn resample_averages_away_near_nyquist_detail_when_shrinking() {
        // A 1px checkerboard at 4x the target: a correct resampler averages it
        // into a flat mid grey instead of leaving aliasing behind.
        let (w, h) = (64u32, 64u32);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                data.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let src = RgbaImage::from_raw(w, h, data).unwrap();
        let out = resample(&src, 16, 16);
        assert_eq!(out.dimensions(), (16, 16));
        for p in out.pixels() {
            let v = p.0[0] as i32;
            assert!(
                (v - 128).abs() <= 8,
                "checkerboard should average to ~128, got {v}"
            );
        }
    }

    #[test]
    fn resample_keeps_average_and_range_when_enlarging() {
        // Enlarging a smooth two-tone test image must not blow up the range or
        // shift the average much (no ringing overflow past the source range).
        let (w, h) = (8u32, 8u32);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 40 } else { 200 };
                data.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let src = RgbaImage::from_raw(w, h, data).unwrap();
        let out = resample(&src, 32, 32);
        let mean = |img: &RgbaImage| {
            let sum: u32 = img.pixels().map(|p| p.0[0] as u32).sum();
            sum as f64 / img.pixels().len() as f64
        };
        assert!((mean(&out) - mean(&src)).abs() < 6.0);
        // CatmullRom sharpens the step slightly (small overshoot), so allow a
        // few units beyond the source range, but nothing close to clipping.
        let min = out.pixels().map(|p| p.0[0]).min().unwrap() as i32;
        let max = out.pixels().map(|p| p.0[0]).max().unwrap() as i32;
        assert!(
            (min - 40).abs() <= 12 && (max - 200).abs() <= 12,
            "range {min}..{max} drifted too far from the source range"
        );
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
    fn exif_orientation_is_applied_to_static_images() {
        let dir = std::env::temp_dir().join(format!("wl-exif-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let plain_path = jpeg_with_orientation(&dir, None);
        let plain = decode_to_rgba8(&plain_path).unwrap();
        assert_eq!((plain.width, plain.height), (8, 4), "plain JPEG resized?");

        // Orientation 6 = rotate 90 degrees; the 8x4 photo becomes 4x8.
        let oriented_path = jpeg_with_orientation(&dir, Some(6));
        let oriented = decode_to_rgba8(&oriented_path).unwrap();
        assert_eq!(
            (oriented.width, oriented.height),
            (4, 8),
            "EXIF orientation was not applied"
        );

        // Same pixels the image crate produces when applying the tag itself.
        let mut expected = image::load_from_memory(&fs::read(&oriented_path).unwrap()).unwrap();
        expected.apply_orientation(image::metadata::Orientation::Rotate90);
        assert_eq!(oriented.data, expected.to_rgba8().into_raw());

        fs::remove_dir_all(&dir).unwrap();
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
    fn svg_detection_is_case_insensitive() {
        for name in ["a.svg", "a.SVG", "a.Svg", "a.svgz", "a.SVGZ"] {
            assert!(is_svg(Path::new(name)), "{name} must be detected as SVG");
        }
        for name in ["a.png", "a.svg.png", "a.sv"] {
            assert!(!is_svg(Path::new(name)), "{name} must not be an SVG");
        }
    }

    #[test]
    fn oversized_gif_canvas_is_rejected_instead_of_allocated() {
        let dir = std::env::temp_dir().join(format!("wl-gif-limits-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("huge.gif");

        {
            let file = fs::File::create(&path).unwrap();
            // 20000x20000 logical screen: 1.6 GB of RGBA, far over the cap.
            let mut encoder = gif::Encoder::new(file, 20_000, 20_000, &[]).unwrap();
            // from_rgba_speed builds a valid indexed frame (palette + indices);
            // a hand-filled RGBA buffer produces an unusable file.
            let mut rgba = [0u8, 0, 0, 255];
            let frame = gif::Frame::from_rgba_speed(1, 1, &mut rgba, 10);
            encoder.write_frame(&frame).unwrap();
        }

        let error = gif_frame_stream(&path)
            .err()
            .expect("oversized GIF accepted");
        assert!(
            error.to_string().contains("limits"),
            "unexpected error: {error}"
        );
        fs::remove_dir_all(&dir).unwrap();
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
                    &[
                        10 + 30 * count as u8,
                        20 + 30 * count as u8,
                        30 + 30 * count as u8,
                        255
                    ]
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
