//! Decoded board images, ready for the GPU.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use pb_core::{AssetId, Assets};
use vello::peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

/// Images are decoded at most this large on their longer side; vello keeps images in an
/// atlas, and bigger ones would not fit.
///
/// TODO: decoding off the main thread.
const MAX_SIDE: u32 = 4096;

/// Smaller copies of an image stop halving at this longer side.
const MIN_SIDE: u32 = 64;

/// Decoded images by asset, decoded on first use.
///
/// vello copies every image drawn in a frame into one atlas of at most 8192×8192 pixels,
/// and an image that does not fit is silently not drawn. So an image is drawn from a copy
/// just big enough for its size on screen (halved from the decoded one), which keeps a
/// screen full of photos and PDF pages well inside the atlas.
#[derive(Default)]
pub struct ImageCache {
    /// `None` for files that could not be decoded, so they are not retried every frame.
    decoded: HashMap<AssetId, Option<Levels>>,
}

/// An image at full size, then each copy half as large as the one before; made on demand.
struct Levels {
    levels: Vec<ImageBrush>,
}

impl ImageCache {
    /// The image for `id` with a longer side of at least `side` pixels where it can, or
    /// `None` if the file is missing or broken.
    pub fn get(&mut self, assets: &Assets, id: AssetId, side: f64) -> Option<&ImageBrush> {
        let levels = match self.decoded.entry(id) {
            Entry::Occupied(entry) => entry.into_mut(),
            // A missing file may arrive later (pasted again, synced), so it is not cached.
            Entry::Vacant(entry) => entry.insert(decode(assets.get(id)?, id)),
        };
        levels.as_mut().map(|levels| levels.at_least(side))
    }

    /// Drops the decoded image of `id`, e.g. a book's page turned away from: decoded pages
    /// are large, and a book has many.
    pub fn forget(&mut self, id: AssetId) {
        self.decoded.remove(&id);
    }

    /// Drops all decoded images (another board was opened).
    pub fn clear(&mut self) {
        self.decoded.clear();
    }
}

impl Levels {
    fn at_least(&mut self, side: f64) -> &ImageBrush {
        let mut i = 0;
        loop {
            if i + 1 == self.levels.len() {
                let last = &self.levels[i].image;
                if last.width.max(last.height) / 2 < MIN_SIDE {
                    break;
                }
                let half = halve(last);
                self.levels.push(ImageBrush::new(half));
            }
            let next = &self.levels[i + 1].image;
            if f64::from(next.width.max(next.height)) < side {
                break;
            }
            i += 1;
        }
        &self.levels[i]
    }
}

fn decode(bytes: &[u8], id: AssetId) -> Option<Levels> {
    match pb_io::image::decode(bytes, MAX_SIDE) {
        Ok(pixels) => Some(Levels {
            levels: vec![ImageBrush::new(image_data(
                pixels.rgba,
                pixels.width,
                pixels.height,
            ))],
        }),
        Err(err) => {
            log::warn!("cannot show image {id}: {err}");
            None
        }
    }
}

fn image_data(rgba: Vec<u8>, width: u32, height: u32) -> ImageData {
    ImageData {
        data: Blob::from(rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    }
}

/// The image at half its size: each pixel averages a 2×2 block, colors weighted by alpha
/// so transparent pixels do not darken edges.
fn halve(image: &ImageData) -> ImageData {
    let (w, h) = (image.width as usize, image.height as usize);
    let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
    let src = image.data.data();
    let mut out = vec![0u8; hw * hh * 4];
    for y in 0..hh {
        for x in 0..hw {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1));
                let p = &src[(sy * w + sx) * 4..][..4];
                let a = u32::from(p[3]);
                for c in 0..3 {
                    sum[c] += u32::from(p[c]) * a;
                }
                sum[3] += a;
            }
            let o = &mut out[(y * hw + x) * 4..][..4];
            for c in 0..3 {
                o[c] = sum[c].checked_div(sum[3]).unwrap_or(0) as u8;
            }
            o[3] = (sum[3] / 4) as u8;
        }
    }
    image_data(out, hw as u32, hh as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(width: u32, height: u32) -> Levels {
        let rgba = vec![255; (width * height * 4) as usize];
        Levels {
            levels: vec![ImageBrush::new(image_data(rgba, width, height))],
        }
    }

    fn side(brush: &ImageBrush) -> u32 {
        brush.image.width.max(brush.image.height)
    }

    #[test]
    fn small_on_screen_images_use_small_copies() {
        let mut image = levels(2000, 1000);
        assert_eq!(side(image.at_least(5000.0)), 2000);
        assert_eq!(side(image.at_least(1500.0)), 2000);
        assert_eq!(side(image.at_least(1000.0)), 1000);
        assert_eq!(side(image.at_least(300.0)), 500);
        // Never smaller than MIN_SIDE.
        assert_eq!(side(image.at_least(1.0)), 125);
        assert_eq!(image.at_least(1.0).image.height, 62);
        // Copies are made once.
        let count = image.levels.len();
        image.at_least(1.0);
        assert_eq!(image.levels.len(), count);
    }

    #[test]
    fn halving_keeps_colors_under_transparency() {
        // Red opaque pixel next to transparent black ones.
        let mut rgba = vec![0u8; 4 * 4];
        rgba[..4].copy_from_slice(&[255, 0, 0, 255]);
        let half = halve(&image_data(rgba, 2, 2));
        assert_eq!((half.width, half.height), (1, 1));
        assert_eq!(half.data.data(), &[255, 0, 0, 63]);
    }
}
