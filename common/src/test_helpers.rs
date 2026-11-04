//! Shared helpers for tests (crafting files that the image crate cannot write).

use std::path::{Path, PathBuf};

/// Encode a small JPEG (8x4, red left half, blue right half) and, when
/// `orientation` is set, prepend an EXIF APP1 segment with that Orientation
/// tag. Used to test EXIF handling without an external tool.
pub fn jpeg_with_orientation(dir: &Path, orientation: Option<u8>) -> PathBuf {
    use image::{Rgba, RgbaImage};

    let mut img = RgbaImage::new(8, 4);
    for (x, _, pixel) in img.enumerate_pixels_mut() {
        *pixel = if x < 4 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        };
    }

    let mut jpeg = Vec::new();
    {
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95);
        encoder
            .encode_image(&image::DynamicImage::ImageRgba8(img))
            .unwrap();
    }

    let path = dir.join(match orientation {
        Some(value) => format!("oriented-{value}.jpg"),
        None => "plain.jpg".to_string(),
    });

    if let Some(value) = orientation {
        // SOI then APP1: "Exif\0\0" + a tiny little-endian TIFF IFD0 with a
        // single Orientation (0x0112) SHORT entry.
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II\x2a\x00\x08\x00\x00\x00"); // IFD at offset 8
        tiff.extend_from_slice(&1u16.to_le_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes()); // count
        tiff.extend_from_slice(&u16::from(value).to_le_bytes());
        tiff.extend_from_slice(&[0, 0]); // value padding
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no next IFD

        let mut payload = b"Exif\x00\x00".to_vec();
        payload.extend_from_slice(&tiff);
        let length = (payload.len() + 2) as u16;

        let mut with_exif = Vec::with_capacity(jpeg.len() + payload.len() + 4);
        with_exif.extend_from_slice(&jpeg[..2]); // SOI
        with_exif.extend_from_slice(&[0xFF, 0xE1]);
        with_exif.extend_from_slice(&length.to_be_bytes());
        with_exif.extend_from_slice(&payload);
        with_exif.extend_from_slice(&jpeg[2..]);
        jpeg = with_exif;
    }

    std::fs::write(&path, &jpeg).unwrap();
    path
}
