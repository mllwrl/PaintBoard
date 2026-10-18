//! SVG files drawn into pictures, to place on the board.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

/// A drawn SVG: PNG bytes, the size in pixels it was drawn at, and the size it asks to be
/// shown at (its own width and height).
pub struct Drawing {
    pub png: Vec<u8>,
    pub pixels: (u32, u32),
    pub size: (f64, f64),
}

/// Drawings are this many times their own size, so they stay sharp up close...
const SCALE: f32 = 2.0;
/// ...but at least this many pixels on the longer side (icons are tiny)...
const MIN_SIDE: f32 = 1024.0;
/// ...and at most this many.
const MAX_SIDE: f32 = 4096.0;

/// The system's fonts for text in SVG files, loaded once.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Starts loading the system's fonts in the background, so the first SVG with text does
/// not wait for them. Does nothing where there are no threads (the web).
pub fn warm_up() {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(|| {
        fonts();
    });
}

/// Draws the SVG (or gzipped SVGZ) file in `bytes`. Pictures it links to are looked up
/// next to `dir`, the file's folder.
pub fn draw(bytes: &[u8], dir: Option<&Path>) -> Result<Drawing, String> {
    let options = usvg::Options {
        resources_dir: dir.map(Path::to_path_buf),
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    let longer = w.max(h);
    let scale = (SCALE * longer).clamp(MIN_SIDE, MAX_SIDE) / longer;
    let pixels = (
        ((w * scale).ceil() as u32).max(1),
        ((h * scale).ceil() as u32).max(1),
    );
    let mut pixmap =
        tiny_skia::Pixmap::new(pixels.0, pixels.1).ok_or("the drawing is too large")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    Ok(Drawing {
        png,
        pixels,
        size: (f64::from(w), f64::from(h)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_shapes_at_a_sharp_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50">
            <rect x="0" y="0" width="50" height="50" fill="#ff0000"/>
        </svg>"##;
        let drawing = draw(svg, None).unwrap();
        assert_eq!(drawing.size, (100.0, 50.0));
        // Small drawings are drawn at least MIN_SIDE pixels wide.
        assert_eq!(drawing.pixels, (1024, 512));
        let pixels = crate::image::decode(&drawing.png, 4096).unwrap();
        let at = |x: u32, y: u32| {
            let i = ((y * pixels.width + x) * 4) as usize;
            pixels.rgba[i..i + 4].to_vec()
        };
        assert_eq!(at(100, 100), [255, 0, 0, 255]);
        assert_eq!(at(900, 100)[3], 0);
    }

    #[test]
    fn rejects_other_files() {
        assert!(draw(b"not an svg", None).is_err());
    }
}
