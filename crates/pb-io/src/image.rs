//! Image files: reading their size, decoding them for display and encoding pixels as PNG.

use std::fmt;
use std::io::Cursor;

use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};

/// File extensions of the formats that can be placed on a board, without the dot.
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

#[derive(Debug)]
pub struct ImageError(String);

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImageError {}

impl From<image::ImageError> for ImageError {
    fn from(err: image::ImageError) -> Self {
        Self(match err {
            image::ImageError::Unsupported(_) => "this image format is not supported".to_owned(),
            err => err.to_string(),
        })
    }
}

impl From<std::io::Error> for ImageError {
    fn from(err: std::io::Error) -> Self {
        Self(err.to_string())
    }
}

/// Decoded pixels: straight (not premultiplied) RGBA, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn decoder(bytes: &[u8]) -> Result<impl ImageDecoder + '_, ImageError> {
    Ok(ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .into_decoder()?)
}

/// Width and height as shown, with the camera's orientation (EXIF) applied.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), ImageError> {
    let mut decoder = decoder(bytes)?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 {
        return Err(ImageError("the image is empty".to_owned()));
    }
    let turned = matches!(
        decoder.orientation()?,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    Ok(if turned { (h, w) } else { (w, h) })
}

/// Decodes an image file, oriented as shown and scaled down to at most `max_side` pixels
/// on its longer side.
pub fn decode(bytes: &[u8], max_side: u32) -> Result<Pixels, ImageError> {
    let mut decoder = decoder(bytes)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    if image.width().max(image.height()) > max_side {
        image = image.resize(max_side, max_side, FilterType::Triangle);
    }
    let image = image.into_rgba8();
    Ok(Pixels {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

/// How an image file is stored, for files that can be copied into another document as
/// they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stored {
    Png,
    /// With this many color components: 1 gray, 3 color, 4 CMYK.
    Jpeg {
        components: u8,
    },
}

/// How `bytes` is stored, if it is a PNG or JPEG file that shows the way it is stored (no
/// camera orientation to apply); `None` for other files.
pub fn stored(bytes: &[u8]) -> Option<Stored> {
    let stored = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Stored::Png
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Stored::Jpeg {
            components: jpeg_components(bytes)?,
        }
    } else {
        return None;
    };
    let orientation = decoder(bytes).ok()?.orientation().ok()?;
    (orientation == Orientation::NoTransforms).then_some(stored)
}

/// The number of color components in a JPEG file, from its frame header.
fn jpeg_components(bytes: &[u8]) -> Option<u8> {
    let mut at = 2;
    loop {
        // Markers may be padded with any number of 0xFF bytes.
        while *bytes.get(at)? == 0xFF && *bytes.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(at + 2)?,
            *bytes.get(at + 3)?,
        ]));
        // Start of frame: every 0xC0..=0xCF but DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            // Length, precision, height and width come before the count.
            return bytes.get(at + 9).copied();
        }
        at += 2 + length;
    }
}

/// A PNG file of the pixels (for pictures pasted from the clipboard).
pub fn encode_png(pixels: &Pixels) -> Result<Vec<u8>, ImageError> {
    let image = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba.clone())
        .ok_or_else(|| ImageError("pixel data does not match the size".to_owned()))?;
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

/// A JPEG file of the pixels at `quality` (1–100); JPEG has no transparency, so alpha is
/// dropped.
pub fn encode_jpeg(pixels: &Pixels, quality: u8) -> Result<Vec<u8>, ImageError> {
    let rgb: Vec<u8> = pixels
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, _]| [r, g, b])
        .collect();
    let mut out = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    image::ImageEncoder::write_image(
        encoder,
        &rgb,
        pixels.width,
        pixels.height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_round_trips_the_size() {
        let bytes = encode_jpeg(&checker(30, 20), 90).unwrap();
        assert_eq!(dimensions(&bytes).unwrap(), (30, 20));
    }

    fn checker(width: u32, height: u32) -> Pixels {
        let rgba = (0..width * height)
            .flat_map(|i| {
                if (i % width + i / width).is_multiple_of(2) {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 128]
                }
            })
            .collect();
        Pixels {
            width,
            height,
            rgba,
        }
    }

    #[test]
    fn png_roundtrip() {
        let pixels = checker(5, 3);
        let png = encode_png(&pixels).unwrap();
        assert_eq!(dimensions(&png).unwrap(), (5, 3));
        assert_eq!(decode(&png, 100).unwrap(), pixels);
    }

    #[test]
    fn large_images_are_scaled_down() {
        let png = encode_png(&checker(400, 100)).unwrap();
        let small = decode(&png, 200).unwrap();
        assert_eq!((small.width, small.height), (200, 50));
        assert_eq!(small.rgba.len(), 200 * 50 * 4);
    }

    #[test]
    fn rejects_other_files() {
        assert!(dimensions(b"not an image").is_err());
        assert!(decode(b"", 10).is_err());
    }
}
