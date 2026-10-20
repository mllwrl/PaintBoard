//! A drawing as an SVG document.

use std::collections::HashMap;
use std::fmt::Write;

use vello::kurbo::{Affine, BezPath, Cap, Join, PathEl, Shape, Size};
use vello::peniko::Color;

use super::{Drawing, Op, Outlines, Rgba8, base64, has_segments, number};

/// The SVG document of `drawing`, `size` drawing units large (a unit is a CSS pixel),
/// over `background` if any.
pub fn svg(drawing: &Drawing, size: Size, background: Option<Color>) -> String {
    let mut writer = Writer {
        drawing,
        defs: String::new(),
        body: String::new(),
        outlines: Outlines::default(),
        glyphs: HashMap::new(),
        pictures: HashMap::new(),
        blurs: HashMap::new(),
        clips: 0,
    };
    if let Some(background) = background.map(Rgba8::of).filter(|c| c.a > 0) {
        let _ = writeln!(
            writer.body,
            "<rect width=\"100%\" height=\"100%\"{}/>",
            paint("fill", background)
        );
    }
    for op in &drawing.ops {
        writer.op(op);
    }
    let (w, h) = (number(size.width, 3), number(size.height, 3));
    let mut out = String::with_capacity(writer.defs.len() + writer.body.len() + 256);
    out += "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
         xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\" \
         width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
    );
    if !writer.defs.is_empty() {
        out += "<defs>\n";
        out += &writer.defs;
        out += "</defs>\n";
    }
    out += &writer.body;
    out += "</svg>\n";
    out
}

struct Writer<'a> {
    drawing: &'a Drawing,
    /// Glyphs, pictures, clips and blurs, referred to from the body by id.
    defs: String,
    body: String,
    outlines: Outlines,
    /// Ids of glyphs written to `defs`; `None` for glyphs without an outline.
    glyphs: HashMap<(u64, u32, u32), Option<usize>>,
    /// Ids of pictures written to `defs`, by index in the drawing; `None` if it could
    /// not be encoded.
    pictures: HashMap<usize, Option<usize>>,
    /// Ids of blur filters by their spread.
    blurs: HashMap<String, usize>,
    clips: usize,
}

impl Writer<'_> {
    fn op(&mut self, op: &Op) {
        match op {
            Op::Fill {
                transform,
                color,
                path,
            } => {
                if has_segments(path) {
                    let _ = writeln!(
                        self.body,
                        "<path{} d=\"{}\"{}/>",
                        matrix(*transform),
                        data(path, 3),
                        paint("fill", *color)
                    );
                }
            }
            Op::Stroke {
                transform,
                color,
                style,
                path,
            } => {
                if !has_segments(path) {
                    return;
                }
                let cap = match style.start_cap {
                    Cap::Butt => "butt",
                    Cap::Square => "square",
                    Cap::Round => "round",
                };
                let join = match style.join {
                    Join::Bevel => "bevel",
                    Join::Miter => "miter",
                    Join::Round => "round",
                };
                let _ = write!(
                    self.body,
                    "<path{} d=\"{}\" fill=\"none\"{} stroke-width=\"{}\" \
                     stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\"",
                    matrix(*transform),
                    data(path, 3),
                    paint("stroke", *color),
                    number(style.width, 3),
                );
                if style.join == Join::Miter {
                    let _ = write!(
                        self.body,
                        " stroke-miterlimit=\"{}\"",
                        number(style.miter_limit, 3)
                    );
                }
                if !style.dash_pattern.is_empty() {
                    let dashes: Vec<String> =
                        style.dash_pattern.iter().map(|d| number(*d, 3)).collect();
                    let _ = write!(
                        self.body,
                        " stroke-dasharray=\"{}\" stroke-dashoffset=\"{}\"",
                        dashes.join(" "),
                        number(style.dash_offset, 3)
                    );
                }
                self.body += "/>\n";
            }
            Op::Push {
                alpha,
                transform,
                clip,
            } => {
                let id = self.clip(*transform, clip);
                let _ = write!(self.body, "<g clip-path=\"url(#c{id})\"");
                if *alpha < 1.0 {
                    let _ = write!(self.body, " opacity=\"{}\"", number(f64::from(*alpha), 3));
                }
                self.body += ">\n";
            }
            Op::Pop => self.body += "</g>\n",
            Op::Shadow {
                transform,
                rect,
                color,
                std_dev,
            } => {
                let spread = number(*std_dev, 3);
                let count = self.blurs.len();
                let id = *self.blurs.entry(spread.clone()).or_insert_with(|| {
                    let _ = writeln!(
                        self.defs,
                        "<filter id=\"b{count}\" x=\"-50%\" y=\"-50%\" width=\"200%\" \
                         height=\"200%\"><feGaussianBlur stdDeviation=\"{spread}\"/></filter>"
                    );
                    count
                });
                let _ = writeln!(
                    self.body,
                    "<rect{} x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{} filter=\"url(#b{id})\"/>",
                    matrix(*transform),
                    number(rect.x0, 3),
                    number(rect.y0, 3),
                    number(rect.width(), 3),
                    number(rect.height(), 3),
                    paint("fill", *color)
                );
            }
            Op::Glyphs {
                transform,
                color,
                font,
                size,
                glyphs,
            } => {
                let units = self.outlines.units_per_em(font);
                let mut uses = String::new();
                for glyph in glyphs {
                    let key = (font.data.id(), font.index, glyph.id);
                    let count = self.glyphs.len();
                    let id = *self.glyphs.entry(key).or_insert_with(|| {
                        let outline = self.outlines.get(font, glyph.id)?;
                        let _ = writeln!(
                            self.defs,
                            "<path id=\"g{count}\" d=\"{}\"/>",
                            data(outline, 1)
                        );
                        Some(count)
                    });
                    if let Some(id) = id {
                        let place = Outlines::place(units, *size, glyph);
                        let _ = writeln!(uses, "<use xlink:href=\"#g{id}\"{}/>", matrix(place));
                    }
                }
                if !uses.is_empty() {
                    let _ = writeln!(
                        self.body,
                        "<g{}{}>\n{uses}</g>",
                        matrix(*transform),
                        paint("fill", *color)
                    );
                }
            }
            Op::Image {
                transform,
                bounds,
                picture,
                placed,
            } => {
                let Some(id) = self.picture(*picture) else {
                    return;
                };
                let mut area = BezPath::new();
                area.extend(bounds.path_elements(0.1));
                let clip = self.clip(*transform, &area);
                let _ = writeln!(
                    self.body,
                    "<g clip-path=\"url(#c{clip})\"><use xlink:href=\"#p{id}\"{}/></g>",
                    matrix(*transform * *placed)
                );
            }
        }
    }

    /// Writes a clip path of `path` placed by `transform`; its id.
    fn clip(&mut self, transform: Affine, path: &BezPath) -> usize {
        let id = self.clips;
        self.clips += 1;
        let _ = writeln!(
            self.defs,
            "<clipPath id=\"c{id}\"><path{} d=\"{}\"/></clipPath>",
            matrix(transform),
            data(path, 3)
        );
        id
    }

    /// Writes picture `index` once; its id.
    fn picture(&mut self, index: usize) -> Option<usize> {
        if let Some(&id) = self.pictures.get(&index) {
            return id;
        }
        let picture = &self.drawing.pictures[index];
        let encoded = match &picture.file {
            Some((file, pb_io::image::Stored::Png)) => Some(("png", file.to_vec())),
            Some((file, pb_io::image::Stored::Jpeg { .. })) => Some(("jpeg", file.to_vec())),
            None => pb_io::image::encode_png(&picture.pixels())
                .ok()
                .map(|png| ("png", png)),
        };
        let id = encoded.map(|(kind, bytes)| {
            let id = self.pictures.len();
            let _ = writeln!(
                self.defs,
                "<image id=\"p{id}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" \
                 xlink:href=\"data:image/{kind};base64,{}\"/>",
                picture.width,
                picture.height,
                base64(&bytes)
            );
            id
        });
        self.pictures.insert(index, id);
        id
    }
}

/// A `transform` attribute, or nothing for no transform.
fn matrix(t: Affine) -> String {
    if t == Affine::IDENTITY {
        return String::new();
    }
    let [a, b, c, d, e, f] = t.as_coeffs();
    if [a, b, c, d] == [1.0, 0.0, 0.0, 1.0] {
        return format!(
            " transform=\"translate({} {})\"",
            number(e, 3),
            number(f, 3)
        );
    }
    let m = [a, b, c, d].map(|v| number(v, 6));
    format!(
        " transform=\"matrix({} {} {} {} {} {})\"",
        m[0],
        m[1],
        m[2],
        m[3],
        number(e, 3),
        number(f, 3)
    )
}

/// Attributes that paint with `color`: `what` is "fill" or "stroke".
fn paint(what: &str, color: Rgba8) -> String {
    let mut out = format!(" {what}=\"#{:02x}{:02x}{:02x}\"", color.r, color.g, color.b);
    if color.a < 255 {
        let _ = write!(out, " {what}-opacity=\"{}\"", number(color.opacity(), 3));
    }
    out
}

/// SVG path data of `path`, coordinates with at most `digits` decimals.
fn data(path: &BezPath, digits: usize) -> String {
    let mut out = String::new();
    let p = |pt: vello::kurbo::Point| format!("{} {}", number(pt.x, digits), number(pt.y, digits));
    for el in path.elements() {
        if !out.is_empty() {
            out.push(' ');
        }
        match *el {
            PathEl::MoveTo(a) => out += &format!("M{}", p(a)),
            PathEl::LineTo(a) => out += &format!("L{}", p(a)),
            PathEl::QuadTo(a, b) => out += &format!("Q{} {}", p(a), p(b)),
            PathEl::CurveTo(a, b, c) => out += &format!("C{} {} {}", p(a), p(b), p(c)),
            PathEl::ClosePath => out.push('Z'),
        }
    }
    out
}
