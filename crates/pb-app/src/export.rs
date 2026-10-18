//! Pictures of the board, a frame or the selection, saved as PNG, JPEG or PDF files.

use std::collections::{HashMap, HashSet};

use pb_core::{Document, ObjectId, ObjectKind};
use pb_geom::Camera;
use pb_geom::kurbo::{Rect, Size, Vec2};
use pb_io::image::Pixels;
use pb_render::vello::Scene;
use pb_render::vello::peniko::Color;
use pb_render::{Gpu, GridStyle, ImageCache, Overlay, TextRenderer, Theme, View, build_scene};

/// What goes into the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scope {
    Board,
    Selection,
    Frame(ObjectId),
    /// Every frame, one PDF page each.
    Frames,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Png,
    Jpeg,
    Pdf,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Pdf => "pdf",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "PNG image",
            Format::Jpeg => "JPEG image",
            Format::Pdf => "PDF document",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub scope: Scope,
    pub format: Format,
    /// Pixels per world unit.
    pub scale: f64,
    /// PNG only: no background behind the objects.
    pub transparent: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scope: Scope::Board,
            format: Format::Png,
            scale: 2.0,
            transparent: false,
        }
    }
}

/// The picture's longest side, in pixels; larger pictures are drawn at a smaller scale.
const MAX_SIDE: f64 = 16384.0;

/// The most pixels a picture may have (256 MB of RGBA).
const MAX_PIXELS: f64 = 64.0e6;

/// Room around the objects, in pixels, when the picture is not of a frame.
const MARGIN: f64 = 32.0;

/// Tiles are drawn one at a time, at most this many pixels on a side.
const TILE: u32 = 4096;

/// The part of the world to draw and which objects in it (`None`: all).
pub struct Target {
    pub area: Rect,
    pub only: Option<HashSet<ObjectId>>,
    /// Room is left around the area; frames are cut at their edges instead.
    pub margin: bool,
}

/// The pictures `scope` asks for: empty when there is nothing to draw.
pub fn targets(doc: &Document, scope: Scope, selection: &[ObjectId]) -> Vec<Target> {
    if scope == Scope::Frames {
        let frames = doc.objects().iter().filter(|o| o.data.kind.is_frame());
        return frames
            .filter(|o| doc.is_shown(o.id) && !doc.is_hidden(o.id))
            .filter_map(|o| target(doc, Scope::Frame(o.id), selection))
            .collect();
    }
    target(doc, scope, selection).into_iter().collect()
}

fn target(doc: &Document, scope: Scope, selection: &[ObjectId]) -> Option<Target> {
    match scope {
        Scope::Frame(id) => {
            let frame = doc.get(id).filter(|o| o.data.kind.is_frame())?;
            Some(Target {
                area: frame.data.bounds,
                only: None,
                margin: false,
            })
        }
        Scope::Frames => None,
        Scope::Board => {
            let ids = doc
                .objects()
                .iter()
                .map(|o| o.id)
                .filter(|&id| doc.is_shown(id) && !doc.is_hidden(id));
            let area = area_of(doc, ids)?;
            Some(Target {
                area,
                only: None,
                margin: true,
            })
        }
        Scope::Selection => {
            let only: HashSet<ObjectId> = doc
                .with_frame_members(selection)
                .into_iter()
                .flat_map(|unit| doc.leaves(unit))
                .map(|o| o.id)
                .filter(|&id| doc.is_shown(id) && !doc.is_hidden(id))
                .collect();
            let area = area_of(doc, only.iter().copied())?;
            Some(Target {
                area,
                only: Some(only),
                margin: true,
            })
        }
    }
}

/// Everything objects `ids` paint, in world units.
fn area_of(doc: &Document, ids: impl Iterator<Item = ObjectId>) -> Option<Rect> {
    ids.filter_map(|id| {
        let object = doc.get(id)?;
        match &object.data.kind {
            ObjectKind::Connector(_) => {
                let width = object.data.style.stroke_width;
                let path = doc.connector_path(id)?.bounds().inflate(width, width);
                Some(doc.connector_label(id).map_or(path, |l| path.union(l)))
            }
            _ => Some(pb_render::painted_bounds(&object.data)),
        }
    })
    .reduce(|a, b| a.union(b))
}

/// Pixels per world unit and the picture size for `area` drawn at about `scale`, kept
/// within [`MAX_SIDE`] and [`MAX_PIXELS`].
pub fn fit(area: Rect, scale: f64, margin: bool) -> (f64, u32, u32) {
    let pad = if margin { MARGIN } else { 0.0 };
    let (w, h) = (area.width().max(1.0), area.height().max(1.0));
    let side = (MAX_SIDE - 2.0 * pad) / w.max(h);
    let mut scale = scale.min(side);
    let pixels = (w * scale + 2.0 * pad) * (h * scale + 2.0 * pad);
    if pixels > MAX_PIXELS {
        scale *= (MAX_PIXELS / pixels).sqrt();
    }
    let scale = scale.max(Camera::MIN_ZOOM);
    let width = (w * scale + 2.0 * pad).ceil() as u32;
    let height = (h * scale + 2.0 * pad).ceil() as u32;
    (scale, width, height)
}

/// Things the picture is drawn with.
pub struct Painters<'a> {
    pub gpu: &'a mut Gpu,
    pub text: &'a mut TextRenderer,
    pub images: &'a mut ImageCache,
    pub theme: &'a Theme,
    pub me: &'a str,
}

/// The file `settings` ask for, of `targets` (one picture, or a PDF page each).
pub fn file(
    doc: &Document,
    targets: &[Target],
    settings: &Settings,
    mut painters: Painters<'_>,
) -> Result<Vec<u8>, String> {
    if settings.format != Format::Pdf {
        let target = targets.first().ok_or("there is nothing to export")?;
        let (pixels, _) = render(doc, target, settings, &mut painters)?;
        return encode(&pixels, settings.format);
    }
    let mut pictures = Vec::new();
    for target in targets {
        let (pixels, scale) = render(doc, target, settings, &mut painters)?;
        let jpeg = encode(&pixels, Format::Jpeg)?;
        pictures.push((jpeg, pixels.width, pixels.height, scale));
    }
    let pages: Vec<_> = pictures
        .iter()
        .map(|(jpeg, w, h, scale)| {
            // 96 board units at 100% to the inch, 72 points.
            let points = |px: u32| f64::from(px) / scale * 0.75;
            pb_io::pdf::PicturePage {
                jpeg,
                pixels: (*w, *h),
                size: (points(*w), points(*h)),
            }
        })
        .collect();
    Ok(pb_io::pdf::write_pictures(&pages))
}

/// Draws `target` as `settings` ask, tile by tile; also returns the pixels per world unit
/// it was drawn at.
fn render(
    doc: &Document,
    target: &Target,
    settings: &Settings,
    painters: &mut Painters<'_>,
) -> Result<(Pixels, f64), String> {
    let margin = target.margin;
    let (scale, width, height) = fit(target.area, settings.scale, margin);
    let pad = if margin { MARGIN / scale } else { 0.0 };
    let origin = target.area.origin() - Vec2::new(pad, pad);
    let background = if settings.transparent && settings.format == Format::Png {
        Color::TRANSPARENT
    } else {
        Color::WHITE
    };
    let tile = painters.gpu.max_image_side().min(TILE);
    let filter = pb_core::Filter::default();
    let votes = HashMap::new();
    let overlay = Overlay::default();
    let mut scene = Scene::new();
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for top in (0..height).step_by(tile as usize) {
        for left in (0..width).step_by(tile as usize) {
            let (w, h) = (tile.min(width - left), tile.min(height - top));
            let corner = origin + Vec2::new(f64::from(left), f64::from(top)) / scale;
            let camera = Camera::new(corner, scale);
            let view = View {
                camera: &camera,
                viewport: Size::new(f64::from(w), f64::from(h)),
                scale: 1.0,
                grid: GridStyle::None,
                theme: painters.theme,
                document: doc,
                assets: doc.assets(),
                overrides: &[],
                hidden: &[],
                previews: &[],
                overlay: &overlay,
                filter: &filter,
                me: painters.me,
                votes: &votes,
                plain: true,
                only: target.only.as_ref(),
            };
            build_scene(
                &mut scene,
                &view,
                &mut *painters.text,
                &mut *painters.images,
            );
            let pixels = painters.gpu.render_image(&scene, w, h, background)?;
            let row = w as usize * 4;
            for (y, line) in pixels.chunks_exact(row).enumerate() {
                let start = ((top as usize + y) * width as usize + left as usize) * 4;
                rgba[start..start + row].copy_from_slice(line);
            }
        }
    }
    let pixels = Pixels {
        width,
        height,
        rgba,
    };
    Ok((pixels, scale))
}

/// A picture file's bytes in `format` (PNG or JPEG).
fn encode(pixels: &Pixels, format: Format) -> Result<Vec<u8>, String> {
    let bytes = match format {
        Format::Png => pb_io::image::encode_png(pixels),
        Format::Jpeg | Format::Pdf => pb_io::image::encode_jpeg(pixels, 92),
    };
    bytes.map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_scale_when_it_fits() {
        let area = Rect::new(0.0, 0.0, 400.0, 300.0);
        assert_eq!(fit(area, 2.0, false), (2.0, 800, 600));
        assert_eq!(fit(area, 1.0, true), (1.0, 464, 364));
    }

    #[test]
    fn fit_shrinks_large_pictures() {
        let area = Rect::new(0.0, 0.0, 100_000.0, 1000.0);
        let (scale, width, _) = fit(area, 2.0, false);
        assert!(scale < 2.0);
        assert!(f64::from(width) <= MAX_SIDE);
        let area = Rect::new(0.0, 0.0, 10_000.0, 10_000.0);
        let (_, width, height) = fit(area, 4.0, false);
        assert!(f64::from(width) * f64::from(height) <= MAX_PIXELS * 1.001);
    }
}
