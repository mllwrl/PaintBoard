//! The board, a frame or the selection saved as a picture (PNG, JPEG), a vector drawing
//! (SVG, PDF), a PDF of pictures, its text (Markdown) or a table (CSV).

use std::collections::{HashMap, HashSet};

use pb_core::{Document, ObjectId, ObjectKind};
use pb_geom::Camera;
use pb_geom::kurbo::{Rect, Size, Vec2};
use pb_io::image::Pixels;
use pb_render::vello::Scene;
use pb_render::vello::peniko::Color;
use pb_render::{
    Gpu, GridStyle, ImageCache, Overlay, TextRenderer, Theme, View, build_scene, draw_vector,
    vector,
};

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
    /// Shapes and text as vector drawing.
    Svg,
    /// Pages of vector drawing.
    Pdf,
    /// Pages that are each a JPEG picture.
    PdfPictures,
    /// The text, as an outline.
    Markdown,
    /// The first table.
    Csv,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Svg => "svg",
            Format::Pdf | Format::PdfPictures => "pdf",
            Format::Markdown => "md",
            Format::Csv => "csv",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "PNG image",
            Format::Jpeg => "JPEG image",
            Format::Svg => "SVG image",
            Format::Pdf | Format::PdfPictures => "PDF document",
            Format::Markdown => "Markdown",
            Format::Csv => "CSV table",
        }
    }

    /// Text rather than a picture: size and background do not apply.
    pub fn is_text(self) -> bool {
        matches!(self, Format::Markdown | Format::Csv)
    }

    /// Drawn as shapes and text rather than pixels: the size does not apply.
    pub fn is_vector(self) -> bool {
        matches!(self, Format::Svg | Format::Pdf)
    }

    /// A document of pages, which can have a page per frame.
    pub fn has_pages(self) -> bool {
        matches!(self, Format::Pdf | Format::PdfPictures)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub scope: Scope,
    pub format: Format,
    /// Pixels per world unit.
    pub scale: f64,
    /// PNG and SVG only: no background behind the objects.
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

/// The objects `target` covers, back to front: shown ones, not hidden.
fn objects<'a>(doc: &'a Document, target: &Target) -> Vec<&'a pb_core::Object> {
    let wanted = |o: &&pb_core::Object| match &target.only {
        Some(only) => only.contains(&o.id),
        None => target.area.contains(o.data.frame().aabb().center()),
    };
    doc.objects()
        .iter()
        .filter(wanted)
        .filter(|o| doc.is_shown(o.id) && !doc.is_hidden(o.id))
        .collect()
}

/// The file `settings` ask for, of `targets` (one picture, or a PDF page each); `title`
/// heads a Markdown outline. Waits for the GPU, so not for the web: see [`Job`].
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn file(
    doc: &Document,
    targets: Vec<Target>,
    settings: &Settings,
    title: &str,
    mut painters: Painters<'_>,
) -> Result<Vec<u8>, String> {
    let mut job = Job::start(doc, targets, settings, title, &mut painters);
    loop {
        if let Some(done) = job.step(doc, &mut painters, true) {
            return done;
        }
    }
}

/// A file being made. Pictures are drawn on the GPU a target at a time and read back
/// when it has drawn them, which in the browser is only on a later frame: call
/// [`Job::step`] until it is done. Text formats are done right away.
pub struct Job {
    targets: Vec<Target>,
    settings: Settings,
    /// The next target to draw.
    next: usize,
    drawing: Option<Drawing>,
    /// PDF pages so far: JPEG bytes, size in pixels and pixels per world unit.
    pages: Vec<(Vec<u8>, u32, u32, f64)>,
    done: Option<Result<Vec<u8>, String>>,
}

/// A target being drawn: its size and tiles on their way back from the GPU.
struct Drawing {
    width: u32,
    height: u32,
    scale: f64,
    /// Each tile's left and top edges and width, in pixels.
    tiles: Vec<(u32, u32, u32, pb_render::PendingImage)>,
}

impl Job {
    pub fn start(
        doc: &Document,
        targets: Vec<Target>,
        settings: &Settings,
        title: &str,
        painters: &mut Painters<'_>,
    ) -> Self {
        let mut job = Self {
            targets,
            settings: *settings,
            next: 0,
            drawing: None,
            pages: Vec::new(),
            done: None,
        };
        if settings.format.is_text() {
            job.done = Some(text_file(doc, &job.targets, settings.format, title));
        } else if job.targets.is_empty() {
            job.done = Some(Err("there is nothing to export".into()));
        } else if settings.format.is_vector() {
            let (text, images) = (&mut *painters.text, &mut *painters.images);
            let looks = (painters.theme, painters.me);
            // Files get every picture, not placeholders for those still decoding.
            let was = images.set_blocking(true);
            job.done = Some(vector_file(
                doc,
                &job.targets,
                settings,
                text,
                images,
                looks,
            ));
            images.set_blocking(was);
        } else {
            job.draw_next(doc, painters);
        }
        job
    }

    /// Takes in what the GPU has drawn and draws the next target; the file once it is
    /// all done. `wait` waits for the GPU (not on the web).
    pub fn step(
        &mut self,
        doc: &Document,
        painters: &mut Painters<'_>,
        wait: bool,
    ) -> Option<Result<Vec<u8>, String>> {
        if self.done.is_none() {
            painters.gpu.poll(wait);
            let drawn = self
                .drawing
                .as_ref()
                .is_some_and(|d| d.tiles.iter().all(|(.., tile)| tile.is_ready()));
            if drawn {
                let drawing = self.drawing.take().expect("checked above");
                if let Err(err) = self.finish_drawing(drawing) {
                    self.done = Some(Err(err));
                } else if self.done.is_none() {
                    if self.next < self.targets.len() {
                        self.draw_next(doc, painters);
                    } else {
                        self.done = Some(Ok(self.pdf()));
                    }
                }
            }
        }
        self.done.take()
    }

    fn draw_next(&mut self, doc: &Document, painters: &mut Painters<'_>) {
        let target = &self.targets[self.next];
        self.next += 1;
        match draw(doc, target, &self.settings, painters) {
            Ok(drawing) => self.drawing = Some(drawing),
            Err(err) => self.done = Some(Err(err)),
        }
    }

    /// Puts the tiles together into the picture: the file, or the next PDF page.
    fn finish_drawing(&mut self, drawing: Drawing) -> Result<(), String> {
        let Drawing {
            width,
            height,
            scale,
            tiles,
        } = drawing;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        for (left, top, w, tile) in tiles {
            let pixels = tile.take()?;
            let row = w as usize * 4;
            for (y, line) in pixels.chunks_exact(row).enumerate() {
                let start = ((top as usize + y) * width as usize + left as usize) * 4;
                rgba[start..start + row].copy_from_slice(line);
            }
        }
        let pixels = Pixels {
            width,
            height,
            rgba,
        };
        if self.settings.format == Format::PdfPictures {
            let jpeg = encode(&pixels, Format::Jpeg)?;
            self.pages.push((jpeg, width, height, scale));
        } else {
            self.done = Some(encode(&pixels, self.settings.format));
        }
        Ok(())
    }

    fn pdf(&self) -> Vec<u8> {
        let pages: Vec<_> = self
            .pages
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
        pb_io::pdf::write_pictures(&pages)
    }
}

/// The Markdown outline or CSV table of the first target.
fn text_file(
    doc: &Document,
    targets: &[Target],
    format: Format,
    title: &str,
) -> Result<Vec<u8>, String> {
    let target = targets.first().ok_or("there is nothing to export")?;
    let objects = objects(doc, target);
    if format == Format::Markdown {
        return Ok(pb_io::outline::markdown(title, &objects).into_bytes());
    }
    let mut tables: Vec<_> = objects
        .iter()
        .filter_map(|o| match &o.data.kind {
            ObjectKind::Table(table) => Some((o.data.frame().aabb(), table)),
            _ => None,
        })
        .collect();
    tables.sort_by(|(a, _), (b, _)| a.y0.total_cmp(&b.y0).then(a.x0.total_cmp(&b.x0)));
    let (_, table) = tables.first().ok_or("there is no table to export")?;
    Ok(pb_io::outline::table_csv(table).into_bytes())
}

/// The SVG drawing of the first target, or a PDF with a page of vector drawing per target.
/// A board unit is a CSS pixel, as at 100%.
fn vector_file(
    doc: &Document,
    targets: &[Target],
    settings: &Settings,
    text: &mut TextRenderer,
    images: &mut ImageCache,
    (theme, me): (&Theme, &str),
) -> Result<Vec<u8>, String> {
    let filter = pb_core::Filter::default();
    let votes = HashMap::new();
    let overlay = Overlay::default();
    let count = if settings.format == Format::Svg {
        1
    } else {
        targets.len()
    };
    let mut pages = Vec::new();
    for target in &targets[..count] {
        let pad = if target.margin { MARGIN } else { 0.0 };
        let origin = target.area.origin() - Vec2::new(pad, pad);
        let size = Size::new(
            target.area.width() + 2.0 * pad,
            target.area.height() + 2.0 * pad,
        );
        let camera = Camera::new(origin, 1.0);
        let view = View {
            camera: &camera,
            viewport: size,
            scale: 1.0,
            grid: GridStyle::None,
            theme,
            document: doc,
            assets: doc.assets(),
            overrides: &[],
            hidden: &[],
            previews: &[],
            overlay: &overlay,
            filter: &filter,
            me,
            votes: &votes,
            plain: true,
            only: target.only.as_ref(),
            frame_titles: target.margin,
        };
        let drawing = draw_vector(&view, &mut *text, &mut *images);
        pages.push(vector::Page { drawing, size });
    }
    if settings.format == Format::Svg {
        let page = pages.first().ok_or("there is nothing to export")?;
        let background = (!settings.transparent).then_some(Color::WHITE);
        return Ok(vector::svg(&page.drawing, page.size, background).into_bytes());
    }
    Ok(vector::pdf(&pages))
}

/// Starts drawing `target` as `settings` ask, tile by tile.
fn draw(
    doc: &Document,
    target: &Target,
    settings: &Settings,
    painters: &mut Painters<'_>,
) -> Result<Drawing, String> {
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
    let mut tiles = Vec::new();
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
                frame_titles: target.margin,
            };
            // Files get every picture, not placeholders for those still decoding.
            let was = painters.images.set_blocking(true);
            build_scene(
                &mut scene,
                &view,
                &mut *painters.text,
                &mut *painters.images,
            );
            painters.images.set_blocking(was);
            let pending = painters.gpu.start_image(&scene, w, h, background)?;
            tiles.push((left, top, w, pending));
        }
    }
    Ok(Drawing {
        width,
        height,
        scale,
        tiles,
    })
}

/// A picture file's bytes in `format` (PNG or JPEG).
fn encode(pixels: &Pixels, format: Format) -> Result<Vec<u8>, String> {
    let bytes = match format {
        Format::Png => pb_io::image::encode_png(pixels),
        Format::Jpeg
        | Format::Svg
        | Format::Pdf
        | Format::PdfPictures
        | Format::Markdown
        | Format::Csv => pb_io::image::encode_jpeg(pixels, 92),
    };
    bytes.map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board of every template built in code, side by side.
    fn template_board(lang: crate::i18n::Lang) -> Document {
        use crate::templates::{Builder, Make, TEMPLATES};
        let mut doc = Document::new();
        let mut engine = pb_text::TextEngine::new();
        let mut x = 0.0;
        for template in &TEMPLATES {
            let Make::Objects(make) = template.make else {
                continue;
            };
            let mut b = Builder::new(&doc, &mut engine).in_lang(lang);
            make(&mut b);
            let items = b.finish(pb_geom::kurbo::Point::new(x, 0.0));
            doc.apply(pb_core::Command::Create(items)).unwrap();
            x += 3000.0;
        }
        doc
    }

    fn vector(doc: &Document, scope: Scope, format: Format) -> Vec<u8> {
        let settings = Settings {
            scope,
            format,
            ..Settings::default()
        };
        let targets = targets(doc, scope, &[]);
        let mut text = TextRenderer::new();
        let mut images = ImageCache::default();
        let theme = Theme::light();
        let looks = (&theme, "me");
        vector_file(doc, &targets, &settings, &mut text, &mut images, looks).unwrap()
    }

    #[test]
    fn templates_export_as_svg_and_vector_pdf() {
        for lang in [crate::i18n::Lang::En, crate::i18n::Lang::Ru] {
            templates_export_in(lang);
        }
    }

    fn templates_export_in(lang: crate::i18n::Lang) {
        let doc = template_board(lang);
        let svg = String::from_utf8(vector(&doc, Scope::Board, Format::Svg)).unwrap();
        assert!(svg.starts_with("<?xml"));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("<g").count(), svg.matches("</g>").count());
        assert!(svg.matches("<use ").count() > 500);
        let frames = doc
            .objects()
            .iter()
            .filter(|o| o.data.kind.is_frame())
            .count();
        assert!(frames > 1);
        let pdf = vector(&doc, Scope::Frames, Format::Pdf);
        assert_eq!(pb_io::pdf::page_count(&pdf).unwrap(), frames);
    }

    /// Writes the templates board as SVG and PDF, and pictures of them, to
    /// `PB_VECTOR_OUT`, to look at.
    #[test]
    #[ignore = "writes files, run by hand"]
    fn write_template_board() {
        let dir = std::path::PathBuf::from(std::env::var("PB_VECTOR_OUT").unwrap());
        let doc = template_board(crate::i18n::Lang::Ru);
        let svg = vector(&doc, Scope::Board, Format::Svg);
        std::fs::write(dir.join("templates.svg"), &svg).unwrap();
        let drawn = pb_io::svg::draw(&svg, None).unwrap();
        std::fs::write(dir.join("templates-svg.png"), drawn.png).unwrap();
        let pdf = vector(&doc, Scope::Frames, Format::Pdf);
        std::fs::write(dir.join("templates.pdf"), &pdf).unwrap();
        for (i, page) in pb_io::pdf::render_pages(&pdf, 8)
            .unwrap()
            .iter()
            .enumerate()
        {
            std::fs::write(dir.join(format!("templates-pdf{i}.png")), &page.png).unwrap();
        }
    }

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
