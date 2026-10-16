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

/// A PNG file of the pixels (for pictures pasted from the clipboard).
pub fn encode_png(pixels: &Pixels) -> Result<Vec<u8>, ImageError> {
    let image = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba.clone())
        .ok_or_else(|| ImageError("pixel data does not match the size".to_owned()))?;
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

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
