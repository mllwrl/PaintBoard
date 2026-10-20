//! Drawings as the pages of a PDF file.

use std::collections::HashMap;
use std::fmt::Write;

use vello::kurbo::{Affine, BezPath, Cap, Join, PathEl, Point, RoundedRect, Shape, Size};

use super::{Drawing, Op, Outlines, Rgba8, has_segments, number, transformed};

/// A page: its drawing and its size in drawing units (pixels at 100%).
pub struct Page {
    pub drawing: Drawing,
    pub size: Size,
}

/// Points (1/72 inch) per drawing unit: 96 units to the inch, like the board at 100%.
const POINTS_PER_UNIT: f64 = 0.75;

/// A PDF file with a page per drawing, shapes and text as vector paths.
pub fn pdf(pages: &[Page]) -> Vec<u8> {
    let mut file = File::new();
    let catalog = file.reserve();
    let tree = file.reserve();
    let resources = file.reserve();
    let mut writer = Writer {
        file,
        resources,
        outlines: Outlines::default(),
        alphas: HashMap::new(),
        glyphs: HashMap::new(),
        pictures: HashMap::new(),
        forms: Vec::new(),
        drawing: None,
    };
    let mut kids = Vec::new();
    for page in pages {
        let (w, h) = (page.size.width.max(1.0), page.size.height.max(1.0));
        let scale = POINTS_PER_UNIT.min(pb_io::pdf::MAX_PAGE_SIDE / w.max(h));
        let (pw, ph) = (w * scale, h * scale);
        // Drawing units, y down, to points, y up.
        let mut content = format!(
            "{} 0 0 {} 0 {} cm\n",
            number(scale, 6),
            number(-scale, 6),
            number(ph, 3)
        );
        writer.drawing = Some(&page.drawing);
        let mut at = 0;
        writer.content(&page.drawing.ops, &mut at, &mut content);
        let contents = writer.file.reserve();
        writer.file.stream(contents, "", content.as_bytes());
        let id = writer.file.reserve();
        let body = format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] /Resources {resources} 0 R \
             /Contents {contents} 0 R >>",
            number(pw, 3),
            number(ph, 3)
        );
        writer.file.object(id, body.as_bytes());
        kids.push(format!("{id} 0 R"));
    }
    let mut dict = String::from("<< /ExtGState <<");
    let mut alphas: Vec<_> = writer.alphas.iter().collect();
    alphas.sort_by_key(|(_, id)| **id);
    for (alpha, id) in alphas {
        let a = number(f64::from(*alpha) / 1000.0, 3);
        let _ = write!(dict, " /a{id} << /ca {a} /CA {a} >>");
    }
    dict += " >> /XObject <<";
    for (name, id) in &writer.forms {
        let _ = write!(dict, " /{name} {id} 0 R");
    }
    dict += " >> >>";
    let mut file = writer.file;
    file.object(resources, dict.as_bytes());
    let body = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        kids.len()
    );
    file.object(tree, body.as_bytes());
    file.object(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R >>").as_bytes(),
    );
    file.finish(catalog)
}

/// A PDF file being written, objects in any order.
struct File {
    out: Vec<u8>,
    /// Where each object starts, by id less one.
    offsets: Vec<usize>,
}

impl File {
    fn new() -> Self {
        Self {
            out: b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            offsets: Vec::new(),
        }
    }

    /// An id for an object written later.
    fn reserve(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len()
    }

    fn object(&mut self, id: usize, body: &[u8]) {
        self.offsets[id - 1] = self.out.len();
        self.out.extend(format!("{id} 0 obj\n").bytes());
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// A stream object of `data`, compressed; `dict` holds entries besides its length and
    /// filter.
    fn stream(&mut self, id: usize, dict: &str, data: &[u8]) {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        self.raw_stream(id, &format!("{dict} /Filter /FlateDecode"), &packed);
    }

    /// A stream object of `data` as it is (already encoded as `dict` says).
    fn raw_stream(&mut self, id: usize, dict: &str, data: &[u8]) {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.object(id, &body);
    }

    fn finish(mut self, root: usize) -> Vec<u8> {
        let xref = self.out.len();
        let count = self.offsets.len() + 1;
        self.out
            .extend(format!("xref\n0 {count}\n0000000000 65535 f \n").bytes());
        for offset in &self.offsets {
            self.out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        self.out.extend(
            format!("trailer\n<< /Size {count} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .bytes(),
        );
        self.out
    }
}

struct Writer<'a> {
    file: File,
    /// The resource dictionary all pages and forms share.
    resources: usize,
    outlines: Outlines,
    /// Graphics states by opacity in thousandths, numbered.
    alphas: HashMap<u16, usize>,
    /// Form names of glyph outlines; `None` for glyphs with nothing to draw.
    glyphs: HashMap<(u64, u32, u32), Option<String>>,
    /// Image names by picture, per drawing (by its address); `None` if it cannot be
    /// written.
    pictures: HashMap<(usize, usize), Option<String>>,
    /// Every form and image by name, and its object.
    forms: Vec<(String, usize)>,
    /// The drawing whose pictures images refer to.
    drawing: Option<&'a Drawing>,
}

impl Writer<'_> {
    /// Writes `ops` from `at` to the end of the group they are in (or of all).
    fn content(&mut self, ops: &[Op], at: &mut usize, out: &mut String) {
        while let Some(op) = ops.get(*at) {
            *at += 1;
            match op {
                Op::Fill {
                    transform,
                    color,
                    path,
                } => {
                    if has_segments(path) {
                        *out += "q ";
                        cm(out, *transform);
                        self.color(out, *color, "rg");
                        path_ops(out, path, 3);
                        *out += "f Q\n";
                    }
                }
                Op::Stroke {
                    transform,
                    color,
                    style,
                    path,
                } => {
                    if !has_segments(path) {
                        continue;
                    }
                    *out += "q ";
                    cm(out, *transform);
                    self.color(out, *color, "RG");
                    let cap = match style.start_cap {
                        Cap::Butt => 0,
                        Cap::Round => 1,
                        Cap::Square => 2,
                    };
                    let join = match style.join {
                        Join::Miter => 0,
                        Join::Round => 1,
                        Join::Bevel => 2,
                    };
                    let _ = write!(
                        out,
                        "{} w {cap} J {join} j {} M ",
                        number(style.width, 3),
                        number(style.miter_limit, 3)
                    );
                    if !style.dash_pattern.is_empty() {
                        let dashes: Vec<String> =
                            style.dash_pattern.iter().map(|d| number(*d, 3)).collect();
                        let _ = write!(
                            out,
                            "[{}] {} d ",
                            dashes.join(" "),
                            number(style.dash_offset, 3)
                        );
                    }
                    path_ops(out, path, 3);
                    *out += "S Q\n";
                }
                Op::Push {
                    alpha,
                    transform,
                    clip,
                } => {
                    let clip = transformed(*transform, clip);
                    *out += "q ";
                    path_ops(out, &clip, 3);
                    *out += "W n\n";
                    if *alpha < 1.0 {
                        // Drawn as a whole: a transparency group.
                        let mut inner = String::new();
                        self.content(ops, at, &mut inner);
                        let b = clip.bounding_box();
                        let id = self.file.reserve();
                        let name = format!("f{id}");
                        let dict = format!(
                            "/Type /XObject /Subtype /Form /BBox [{} {} {} {}] \
                             /Group << /S /Transparency >> /Resources {} 0 R",
                            number(b.x0, 3),
                            number(b.y0, 3),
                            number(b.x1, 3),
                            number(b.y1, 3),
                            self.resources
                        );
                        self.file.stream(id, &dict, inner.as_bytes());
                        self.forms.push((name.clone(), id));
                        let gs = self.alpha(f64::from(*alpha));
                        let _ = writeln!(out, "/a{gs} gs /{name} Do");
                    } else {
                        self.content(ops, at, out);
                    }
                    *out += "Q\n";
                }
                Op::Pop => return,
                Op::Shadow {
                    transform,
                    rect,
                    color,
                    std_dev,
                } => {
                    // A blur is beyond PDF's basic means: stacked, see-through rounded
                    // rectangles, from two spreads out to one in, darker inwards.
                    const STEPS: usize = 12;
                    let faint = Rgba8 {
                        a: (f64::from(color.a) / STEPS as f64).round().max(1.0) as u8,
                        ..*color
                    };
                    *out += "q ";
                    cm(out, *transform);
                    self.color(out, faint, "rg");
                    for step in 0..STEPS {
                        let d = (2.0 - 3.0 * step as f64 / (STEPS - 1) as f64) * std_dev;
                        let r = rect.inflate(d, d);
                        if r.width() > 0.0 && r.height() > 0.0 {
                            let shape = RoundedRect::from_rect(r, d.max(0.0));
                            path_ops(out, &shape.path_elements(0.01).collect(), 3);
                            *out += "f ";
                        }
                    }
                    *out += "Q\n";
                }
                Op::Glyphs {
                    transform,
                    color,
                    font,
                    size,
                    glyphs,
                } => {
                    let units = self.outlines.units_per_em(font);
                    *out += "q ";
                    cm(out, *transform);
                    self.color(out, *color, "rg");
                    out.push('\n');
                    for glyph in glyphs {
                        let Some(name) = self.glyph(font, glyph.id) else {
                            continue;
                        };
                        *out += "q ";
                        cm(out, Outlines::place(units, *size, glyph));
                        let _ = writeln!(out, "/{name} Do Q");
                    }
                    *out += "Q\n";
                }
                Op::Image {
                    transform,
                    bounds,
                    picture,
                    placed,
                } => {
                    let Some((name, w, h)) = self.picture(*picture) else {
                        continue;
                    };
                    *out += "q ";
                    let area: BezPath = bounds.path_elements(0.1).collect();
                    path_ops(out, &transformed(*transform, &area), 3);
                    *out += "W n ";
                    // The image fills the unit square, its first row at the top.
                    let unit = Affine::new([w, 0.0, 0.0, -h, 0.0, h]);
                    cm(out, *transform * *placed * unit);
                    let _ = writeln!(out, "/{name} Do Q");
                }
            }
        }
    }

    /// Sets `color` with operator `op` ("rg" to fill, "RG" to stroke) and its opacity.
    fn color(&mut self, out: &mut String, color: Rgba8, op: &str) {
        let c = |v: u8| number(f64::from(v) / 255.0, 3);
        let _ = write!(out, "{} {} {} {op} ", c(color.r), c(color.g), c(color.b));
        if color.a < 255 {
            let gs = self.alpha(color.opacity());
            let _ = write!(out, "/a{gs} gs ");
        }
    }

    /// The graphics state with opacity `alpha`.
    fn alpha(&mut self, alpha: f64) -> usize {
        let key = (alpha.clamp(0.0, 1.0) * 1000.0).round() as u16;
        let count = self.alphas.len();
        *self.alphas.entry(key).or_insert(count)
    }

    /// The form drawing glyph `id` of `font`, in font units.
    fn glyph(&mut self, font: &vello::peniko::FontData, id: u32) -> Option<String> {
        let key = (font.data.id(), font.index, id);
        if let Some(name) = self.glyphs.get(&key) {
            return name.clone();
        }
        let name = self.outlines.get(font, id).cloned().map(|outline| {
            let mut content = String::new();
            path_ops(&mut content, &outline, 1);
            content += "f";
            let b = outline.bounding_box();
            let object = self.file.reserve();
            let dict = format!(
                "/Type /XObject /Subtype /Form /BBox [{} {} {} {}] /Resources {} 0 R",
                number(b.x0.floor(), 0),
                number(b.y0.floor(), 0),
                number(b.x1.ceil(), 0),
                number(b.y1.ceil(), 0),
                self.resources
            );
            self.file.stream(object, &dict, content.as_bytes());
            let name = format!("g{object}");
            self.forms.push((name.clone(), object));
            name
        });
        self.glyphs.insert(key, name.clone());
        name
    }

    /// The image of picture `index` of the current drawing, with its size in the
    /// picture's pixels.
    fn picture(&mut self, index: usize) -> Option<(String, f64, f64)> {
        let drawing = self.drawing?;
        let picture = drawing.pictures.get(index)?;
        let size = (f64::from(picture.width), f64::from(picture.height));
        let key = (std::ptr::from_ref(drawing) as usize, index);
        if let Some(name) = self.pictures.get(&key) {
            return name.clone().map(|n| (n, size.0, size.1));
        }
        let id = self.file.reserve();
        let jpeg = match &picture.file {
            Some((file, pb_io::image::Stored::Jpeg { components })) => {
                let space = match components {
                    1 => Some("/DeviceGray"),
                    3 => Some("/DeviceRGB"),
                    _ => None,
                };
                let dims = pb_io::image::dimensions(file).ok();
                space.zip(dims).map(|(space, (w, h))| (file, space, w, h))
            }
            _ => None,
        };
        if let Some((file, space, w, h)) = jpeg {
            // A JPEG file is already what PDF can show.
            let dict = format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace {space} \
                 /BitsPerComponent 8 /Filter /DCTDecode"
            );
            self.file.raw_stream(id, &dict, file);
        } else {
            let rgba = picture.rgba.data();
            let rgb: Vec<u8> = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, _]| [r, g, b])
                .collect();
            let (w, h) = (picture.width, picture.height);
            let mut dict = format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB \
                 /BitsPerComponent 8"
            );
            if rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255) {
                let alpha: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
                let mask = self.file.reserve();
                let mask_dict = format!(
                    "/Type /XObject /Subtype /Image /Width {w} /Height {h} \
                     /ColorSpace /DeviceGray /BitsPerComponent 8"
                );
                self.file.stream(mask, &mask_dict, &alpha);
                let _ = write!(dict, " /SMask {mask} 0 R");
            }
            self.file.stream(id, &dict, &rgb);
        }
        let name = format!("p{id}");
        self.forms.push((name.clone(), id));
        self.pictures.insert(key, Some(name.clone()));
        Some((name, size.0, size.1))
    }
}

/// Appends a `cm` operator for `t`, unless it does nothing.
fn cm(out: &mut String, t: Affine) {
    if t == Affine::IDENTITY {
        return;
    }
    let [a, b, c, d, e, f] = t.as_coeffs();
    let _ = write!(
        out,
        "{} {} {} {} {} {} cm ",
        number(a, 6),
        number(b, 6),
        number(c, 6),
        number(d, 6),
        number(e, 3),
        number(f, 3)
    );
}

/// Appends path construction operators for `path` (quadratic curves made cubic).
fn path_ops(out: &mut String, path: &BezPath, digits: usize) {
    let p = |pt: Point| format!("{} {}", number(pt.x, digits), number(pt.y, digits));
    let mut last = Point::ZERO;
    let mut start = Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(a) => {
                let _ = write!(out, "{} m ", p(a));
                (last, start) = (a, a);
            }
            PathEl::LineTo(a) => {
                let _ = write!(out, "{} l ", p(a));
                last = a;
            }
            PathEl::QuadTo(a, b) => {
                let c1 = last + (a - last) * (2.0 / 3.0);
                let c2 = b + (a - b) * (2.0 / 3.0);
                let _ = write!(out, "{} {} {} c ", p(c1), p(c2), p(b));
                last = b;
            }
            PathEl::CurveTo(a, b, c) => {
                let _ = write!(out, "{} {} {} c ", p(a), p(b), p(c));
                last = c;
            }
            PathEl::ClosePath => {
                *out += "h ";
                last = start;
            }
        }
    }
}
