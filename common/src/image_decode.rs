//! Image decoding module — converts image files to RGBA8 pixel data for GPU upload.
//!
//! Supports raster formats via the `image` crate and SVG via `resvg`.

use std::fmt;
use std::fs;
use std::io::{self, BufReader};
use std::path::Path;

use image::AnimationDecoder;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Decoded raster image as raw RGBA8 pixel data.
pub struct DecodedImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// All frames of an animated GIF, each with RGBA8 data and duration.
pub struct GifFrames {
    pub frames: Vec<GifFrame>,
    pub width: u32,
    pub height: u32,
}

/// A single GIF frame.
pub struct GifFrame {
    pub data: Vec<u8>,
    pub duration_ms: u32,
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

/// Decode all frames of an animated GIF at `path`.
///
/// Each frame is returned as raw RGBA8 data together with its display duration
/// in milliseconds.
pub fn decode_gif_frames(path: &Path) -> Result<GifFrames, DecodeError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);

    let decoder = image::codecs::gif::GifDecoder::new(reader)
        .map_err(|e| DecodeError::Image(e.to_string()))?;

    let raw_frames = decoder
        .into_frames()
        .collect_frames()
        .map_err(|e| DecodeError::Image(e.to_string()))?;

    if raw_frames.is_empty() {
        return Err(DecodeError::Image("GIF contains no frames".into()));
    }

    // All frames share the same dimensions (the logical screen size).
    let first = raw_frames[0].buffer();
    let (width, height) = first.dimensions();

    let frames = raw_frames
        .into_iter()
        .map(|frame| {
            let (numer, denom) = frame.delay().numer_denom_ms();
            let duration_ms = if denom == 0 { 0 } else { numer / denom };
            let data = frame.into_buffer().into_raw();
            GifFrame { data, duration_ms }
        })
        .collect();

    Ok(GifFrames {
        frames,
        width,
        height,
    })
}
