//! Pictures coming onto the board: from files (dialog, drag and drop, copied in the file
//! manager) and from the clipboard.

use std::path::Path;

use pb_core::{Image, ObjectData, ObjectKind, Style};
use pb_geom::kurbo::{Point, Rect};
use pb_io::image;
use pb_platform::clipboard::ClipboardImage;
use pb_platform::fs;

/// New images together take at most this part of the visible board's width and height.
const MAX_VIEW_WIDTH: f64 = 0.8;
const MAX_VIEW_HEIGHT: f64 = 0.6;
/// Space between images placed together, in logical pixels.
const GAP: f64 = 20.0;

/// An image file ready to be placed.
pub struct Picture {
    pub bytes: Vec<u8>,
    /// Size as shown, in pixels.
    pub width: u32,
    pub height: u32,
}

impl Picture {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, image::ImageError> {
        let (width, height) = image::dimensions(&bytes)?;
        Ok(Self {
            bytes,
            width,
            height,
        })
    }

    /// Reads an image file; the error message names the file.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let bytes = fs::read(path).map_err(|err| format!("{name}: {err}"))?;
        Self::from_bytes(bytes).map_err(|err| format!("{name}: {err}"))
    }

    /// A clipboard picture, stored as PNG.
    pub fn from_clipboard(image: ClipboardImage) -> Result<Self, image::ImageError> {
        let pixels = image::Pixels {
            width: image.width,
            height: image.height,
            rgba: image.rgba,
        };
        let bytes = image::encode_png(&pixels)?;
        Ok(Self {
            bytes,
            width: pixels.width,
            height: pixels.height,
        })
    }
}

/// Whether `path` names a file of a supported image format.
pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| image::EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Bounds for images of the given pixel sizes, side by side in a row centered on `at`.
/// One image pixel covers one screen pixel (`pixels_per_unit` per world unit: zoom times
/// display scale), unless the row must shrink to fit a part of the `visible` board.
pub fn place(sizes: &[(u32, u32)], at: Point, pixels_per_unit: f64, visible: Rect) -> Vec<Rect> {
    let gap = GAP / pixels_per_unit;
    let world: Vec<_> = sizes
        .iter()
        .map(|&(w, h)| {
            (
                f64::from(w) / pixels_per_unit,
                f64::from(h) / pixels_per_unit,
            )
        })
        .collect();
    let row_width =
        world.iter().map(|(w, _)| w).sum::<f64>() + gap * world.len().saturating_sub(1) as f64;
    let row_height = world.iter().map(|&(_, h)| h).fold(0.0, f64::max);
    let scale = (visible.width() * MAX_VIEW_WIDTH / row_width)
        .min(visible.height() * MAX_VIEW_HEIGHT / row_height)
        .min(1.0);
    let mut x = at.x - row_width * scale / 2.0;
    world
        .into_iter()
        .map(|(w, h)| {
            let (w, h) = (w * scale, h * scale);
            let r = Rect::new(x, at.y - h / 2.0, x + w, at.y + h / 2.0);
            x += w + gap * scale;
            r
        })
        .collect()
}

/// An image object showing `asset` over `bounds`.
pub fn image_object(asset: pb_core::AssetId, bounds: Rect) -> ObjectData {
    ObjectData {
        kind: ObjectKind::Image(Image::new(asset)),
        bounds,
        rotation: 0.0,
        style: Style {
            stroke_width: 0.0,
            ..Style::default()
        },
        tags: Default::default(),
        link: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_in_a_row_at_pixel_size() {
        let visible = Rect::new(0.0, 0.0, 2000.0, 1000.0);
        let rects = place(
            &[(100, 50), (200, 100)],
            Point::new(500.0, 500.0),
            1.0,
            visible,
        );
        assert_eq!(rects[0], Rect::new(340.0, 475.0, 440.0, 525.0));
        assert_eq!(rects[1], Rect::new(460.0, 450.0, 660.0, 550.0));
    }

    #[test]
    fn rows_shrink_to_the_view_together() {
        let visible = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let rects = place(&[(780, 100), (800, 400)], Point::ZERO, 1.0, visible);
        // The row is 1600 wide: it shrinks by half to 800, both images alike.
        assert!((rects[1].x1 - rects[0].x0 - 800.0).abs() < 1e-9);
        assert!((rects[1].height() - 200.0).abs() < 1e-9);
        assert!((rects[0].height() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn pixels_follow_zoom_and_display_scale() {
        let visible = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let r = place(&[(100, 100)], Point::ZERO, 2.5, visible)[0];
        assert_eq!(r.width(), 40.0);
    }

    #[test]
    fn recognizes_image_files() {
        assert!(is_image_file(Path::new("C:/a/photo.JPG")));
        assert!(is_image_file(Path::new("x.webp")));
        assert!(!is_image_file(Path::new("notes.txt")));
        assert!(!is_image_file(Path::new("png")));
    }
}
