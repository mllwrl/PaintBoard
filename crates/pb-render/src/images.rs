//! Decoded board images, ready for the GPU.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use pb_core::{AssetId, Assets};
use vello::peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

/// Images are decoded at most this large on their longer side; vello keeps images in an
/// atlas, and bigger ones would not fit.
///
/// TODO: mipmaps or a lower resolution when zoomed out, and decoding off the main thread.
const MAX_SIDE: u32 = 4096;

/// Decoded images by asset, decoded on first use.
#[derive(Default)]
pub struct ImageCache {
    /// `None` for files that could not be decoded, so they are not retried every frame.
    decoded: HashMap<AssetId, Option<ImageBrush>>,
}

impl ImageCache {
    /// The image for `id`, or `None` if the file is missing or broken.
    pub fn get(&mut self, assets: &Assets, id: AssetId) -> Option<&ImageBrush> {
        let brush = match self.decoded.entry(id) {
            Entry::Occupied(entry) => entry.into_mut(),
            // A missing file may arrive later (pasted again, synced), so it is not cached.
            Entry::Vacant(entry) => entry.insert(decode(assets.get(id)?, id)),
        };
        brush.as_ref()
    }

    /// Drops all decoded images (another board was opened).
    pub fn clear(&mut self) {
        self.decoded.clear();
    }
}

fn decode(bytes: &[u8], id: AssetId) -> Option<ImageBrush> {
    match pb_io::image::decode(bytes, MAX_SIDE) {
        Ok(pixels) => Some(ImageBrush::new(ImageData {
            data: Blob::from(pixels.rgba),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: pixels.width,
            height: pixels.height,
        })),
        Err(err) => {
            log::warn!("cannot show image {id}: {err}");
            None
        }
    }
}
