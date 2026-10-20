//! Objects as vector drawing, for SVG and PDF files: the same drawing code as on screen
//! (see [`crate::draw_vector`]) records shapes, text and pictures here instead of handing
//! them to the GPU.
//!
//! Text is kept as the outlines of its glyphs, placed where the app's text layout puts
//! them: the file looks the same everywhere, with the same line breaks, whether or not
//! the Inter font is installed, though the text cannot be selected.

mod pdf;
mod svg;

pub use pdf::{Page, pdf};
pub use svg::svg;

use std::collections::HashMap;
use std::sync::Arc;

use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use vello::kurbo::{Affine, BezPath, PathEl, Rect, Shape, Stroke};
use vello::peniko::{Color, FontData, ImageBrush};

use crate::canvas::Canvas;

/// Curves are flattened to this accuracy where a shape has to be turned into a path, in
/// drawing units.
const TOLERANCE: f64 = 0.01;

/// A recorded drawing: what [`Canvas`] was asked to draw, in order.
#[derive(Default)]
pub struct Drawing {
    ops: Vec<Op>,
    pictures: Vec<Picture>,
}

/// A straight (not premultiplied) RGBA color.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgba8 {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl Rgba8 {
    fn of(color: Color) -> Self {
        let c = color.to_rgba8();
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    }

    fn opacity(self) -> f64 {
        f64::from(self.a) / 255.0
    }
}

enum Op {
    Fill {
        transform: Affine,
        color: Rgba8,
        path: BezPath,
    },
    Stroke {
        transform: Affine,
        color: Rgba8,
        style: Stroke,
        path: BezPath,
    },
    /// Starts a group, ended by [`Op::Pop`].
    Push {
        alpha: f32,
        transform: Affine,
        clip: BezPath,
    },
    Pop,
    Shadow {
        transform: Affine,
        rect: Rect,
        color: Rgba8,
        std_dev: f64,
    },
    Glyphs {
        transform: Affine,
        color: Rgba8,
        font: FontData,
        size: f32,
        glyphs: Vec<pb_text::Glyph>,
    },
    Image {
        transform: Affine,
        bounds: Rect,
        /// Into [`Drawing::pictures`].
        picture: usize,
        /// From the picture's pixels to the space of `transform`.
        placed: Affine,
    },
}

/// A picture drawn somewhere in the drawing.
struct Picture {
    /// The decoded image's blob, which tells pictures apart.
    key: u64,
    width: u32,
    height: u32,
    /// Straight RGBA.
    rgba: vello::peniko::Blob<u8>,
    /// The file it was decoded from, when that can be copied into the output as it is.
    file: Option<(Arc<[u8]>, pb_io::image::Stored)>,
}

impl Picture {
    fn pixels(&self) -> pb_io::image::Pixels {
        pb_io::image::Pixels {
            width: self.width,
            height: self.height,
            rgba: self.rgba.data().to_vec(),
        }
    }
}

fn path_of(shape: &impl Shape) -> BezPath {
    shape.path_elements(TOLERANCE).collect()
}

impl Drawing {
    /// Nothing was drawn.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

impl Canvas for Drawing {
    fn fill(&mut self, transform: Affine, brush: Color, shape: &impl Shape) {
        let color = Rgba8::of(brush);
        if color.a > 0 {
            let path = path_of(shape);
            self.ops.push(Op::Fill {
                transform,
                color,
                path,
            });
        }
    }

    fn stroke(&mut self, style: &Stroke, transform: Affine, brush: Color, shape: &impl Shape) {
        let color = Rgba8::of(brush);
        if color.a > 0 && style.width > 0.0 {
            self.ops.push(Op::Stroke {
                transform,
                color,
                style: style.clone(),
                path: path_of(shape),
            });
        }
    }

    fn push_layer(&mut self, alpha: f32, transform: Affine, clip: &impl Shape) {
        self.ops.push(Op::Push {
            alpha,
            transform,
            clip: path_of(clip),
        });
    }

    fn push_clip(&mut self, transform: Affine, clip: &impl Shape) {
        self.push_layer(1.0, transform, clip);
    }

    fn pop_layer(&mut self) {
        self.ops.push(Op::Pop);
    }

    fn shadow(&mut self, transform: Affine, rect: Rect, brush: Color, std_dev: f64) {
        self.ops.push(Op::Shadow {
            transform,
            rect,
            color: Rgba8::of(brush),
            std_dev,
        });
    }

    fn glyphs(&mut self, transform: Affine, run: &pb_text::GlyphRun, brush: Color) {
        let color = Rgba8::of(brush);
        if color.a > 0 && !run.glyphs.is_empty() {
            self.ops.push(Op::Glyphs {
                transform,
                color,
                font: run.font.clone(),
                size: run.size,
                glyphs: run.glyphs.clone(),
            });
        }
    }

    fn image(
        &mut self,
        transform: Affine,
        bounds: Rect,
        image: &ImageBrush,
        image_transform: Affine,
        file: Option<&[u8]>,
    ) {
        let key = image.image.data.id();
        let picture = match self.pictures.iter().position(|p| p.key == key) {
            Some(index) => index,
            None => {
                let file = file.and_then(|f| Some((Arc::from(f), pb_io::image::stored(f)?)));
                self.pictures.push(Picture {
                    key,
                    width: image.image.width,
                    height: image.image.height,
                    rgba: image.image.data.clone(),
                    file,
                });
                self.pictures.len() - 1
            }
        };
        self.ops.push(Op::Image {
            transform,
            bounds,
            picture,
            placed: image_transform,
        });
    }
}

/// Glyph outlines by font and glyph, read from the fonts once.
#[derive(Default)]
struct Outlines {
    /// In font units, y up; `None` for glyphs with nothing to draw (spaces).
    glyphs: HashMap<(u64, u32, u32), Option<BezPath>>,
    /// Font units per em, by font.
    units: HashMap<(u64, u32), f64>,
}

impl Outlines {
    /// Font units per em of `font`.
    fn units_per_em(&mut self, font: &FontData) -> f64 {
        *self
            .units
            .entry((font.data.id(), font.index))
            .or_insert_with(|| {
                skrifa::FontRef::from_index(font.data.data(), font.index)
                    .map(|f| f.metrics(Size::unscaled(), LocationRef::default()))
                    .map_or(1000.0, |m| f64::from(m.units_per_em.max(1)))
            })
    }

    /// The outline of glyph `id` of `font` in font units, y up.
    fn get(&mut self, font: &FontData, id: u32) -> Option<&BezPath> {
        self.glyphs
            .entry((font.data.id(), font.index, id))
            .or_insert_with(|| {
                let face = skrifa::FontRef::from_index(font.data.data(), font.index).ok()?;
                let glyph = face.outline_glyphs().get(skrifa::GlyphId::new(id))?;
                let mut pen = Pen(BezPath::new());
                let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
                glyph.draw(settings, &mut pen).ok()?;
                (!pen.0.elements().is_empty()).then_some(pen.0)
            })
            .as_ref()
    }

    /// Where glyph outlines of a run at font `size` go: from font units to the run's
    /// space, the glyph at `(x, y)` on its baseline.
    fn place(units_per_em: f64, size: f32, glyph: &pb_text::Glyph) -> Affine {
        let s = f64::from(size) / units_per_em;
        Affine::new([s, 0.0, 0.0, -s, f64::from(glyph.x), f64::from(glyph.y)])
    }
}

struct Pen(BezPath);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(cx1), f64::from(cy1)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}

/// `value` written briefly: at most `digits` decimals, without trailing zeros.
fn number(value: f64, digits: usize) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    let mut s = format!("{value:.digits$}");
    if s.contains('.') {
        s.truncate(s.trim_end_matches('0').trim_end_matches('.').len());
    }
    if s == "-0" { "0".to_owned() } else { s }
}

/// The path's elements with `transform` applied.
fn transformed(transform: Affine, path: &BezPath) -> BezPath {
    let mut path = path.clone();
    path.apply_affine(transform);
    path
}

/// Base64 (standard alphabet, padded), for pictures inside SVG files.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Whether the path has anything to draw.
fn has_segments(path: &BezPath) -> bool {
    path.elements()
        .iter()
        .any(|el| !matches!(el, PathEl::MoveTo(_) | PathEl::ClosePath))
}

#[cfg(test)]
mod tests;
