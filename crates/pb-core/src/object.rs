use std::fmt;

use std::sync::Arc;

use kurbo::{Point, Rect, Vec2};
use loro::TreeID;
use pb_geom::{Frame, ShapeKind};

use crate::{AssetId, Connector, Kanban, MindMap, Span, Spans, Table};

/// Stable identifier of a board object, unique across replicas.
///
/// It is taken from the tree node that first held the object, but it is stored in the
/// object's own data rather than being the node id: Loro's undo of a deletion recreates the
/// node under a new id, and references (selection, connectors) must survive that.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId {
    peer: u64,
    counter: i32,
}

impl ObjectId {
    pub(crate) fn from_node(node: TreeID) -> Self {
        Self {
            peer: node.peer,
            counter: node.counter,
        }
    }

    /// Reads the text form, `counter@peer`.
    pub fn parse(s: &str) -> Option<Self> {
        let (counter, peer) = s.split_once('@')?;
        Some(Self {
            peer: peer.parse().ok()?,
            counter: counter.parse().ok()?,
        })
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({self})")
    }
}

/// `counter@peer`; this is also the stored form.
impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.counter, self.peer)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    /// A geometric shape of the fill color with an outline, and text in the outline color
    /// fitted inside it.
    Shape(Shape),
    /// Freehand drawing; drawn with the outline color and width.
    Ink(Ink),
    /// Text in the outline color, wrapped at the object's width and centered in it.
    Text(Text),
    /// A sticky note: a square of the fill color with text in the outline color, sized to
    /// fit. It has no outline.
    Sticky(Sticky),
    /// A picture, stretched over the object's bounds. It ignores colors and outline.
    Image(Image),
    /// A line between two points or objects, in the outline color and width. Its bounds are
    /// worked out from its ends; see [`crate::connector`].
    Connector(Connector),
    /// A titled area of the fill color that holds the objects inside it; see
    /// [`crate::Document::frame_members`]. It is drawn below everything else and does not
    /// rotate.
    Frame(FrameData),
    /// A grid of cells with text, in the fill color with lines in the outline color.
    Table(Table),
    /// Columns of cards: columns in the fill color, cards outlined in the outline color.
    Kanban(Kanban),
    /// A tree of topics around a central one, laid out automatically over the bounds.
    MindMap(MindMap),
    /// A Markdown document on a page in the fill color, outlined in the outline color.
    Doc(Doc),
    /// A card for a web page, in the fill color with its strip in the outline color.
    Embed(Embed),
}

impl ObjectKind {
    /// Name stored in the document. Never change existing names: old files depend on them.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Ink(_) => "ink",
            Self::Text(_) => "text",
            Self::Sticky(_) => "sticky",
            Self::Image(_) => "image",
            Self::Connector(_) => "connector",
            Self::Frame(_) => "frame",
            Self::Table(_) => "table",
            Self::Kanban(_) => "kanban",
            Self::MindMap(_) => "mindmap",
            Self::Doc(_) => "doc",
            Self::Embed(_) => "embed",
            Self::Shape(shape) => shape_name(shape.kind),
        }
    }

    /// The shape stored under `s`. Ink is read separately, with its points.
    pub fn parse_shape(s: &str) -> Option<ShapeKind> {
        ShapeKind::ALL
            .into_iter()
            .find(|&shape| shape_name(shape) == s)
    }

    pub fn is_ink(&self) -> bool {
        matches!(self, Self::Ink(_))
    }

    pub fn is_frame(&self) -> bool {
        matches!(self, Self::Frame(_))
    }

    /// Whether the fill color shows: sticky notes, shapes, frames, tables and kanban
    /// boards.
    pub fn has_fill(&self) -> bool {
        matches!(
            self,
            Self::Sticky(_)
                | Self::Shape(_)
                | Self::Frame(_)
                | Self::Table(_)
                | Self::Kanban(_)
                | Self::MindMap(_)
                | Self::Doc(_)
                | Self::Embed(_)
        )
    }

    /// The text of a text object, sticky note or shape, or a frame's title.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(&text.text),
            Self::Sticky(sticky) => Some(&sticky.text),
            Self::Shape(shape) => Some(&shape.text),
            Self::Frame(frame) => Some(&frame.title),
            Self::Doc(doc) => Some(&doc.text),
            Self::Embed(embed) => Some(&embed.title),
            Self::Ink(_)
            | Self::Image(_)
            | Self::Connector(_)
            | Self::Table(_)
            | Self::Kanban(_)
            | Self::MindMap(_) => None,
        }
    }

    /// Inline styles of the text of a text object, sticky note or shape.
    pub fn spans(&self) -> &[Span] {
        match self {
            Self::Text(text) => &text.spans,
            Self::Sticky(sticky) => &sticky.spans,
            Self::Shape(shape) => &shape.spans,
            Self::Frame(_)
            | Self::Ink(_)
            | Self::Image(_)
            | Self::Connector(_)
            | Self::Table(_)
            | Self::Kanban(_)
            | Self::MindMap(_)
            | Self::Doc(_)
            | Self::Embed(_) => &[],
        }
    }
}

/// The stored name of a shape kind, like `rect` or `roundRect`; plugins use it too.
pub fn shape_name(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Rectangle => "rect",
        ShapeKind::RoundedRectangle => "roundRect",
        ShapeKind::Ellipse => "ellipse",
        ShapeKind::Diamond => "diamond",
        ShapeKind::Triangle => "triangle",
        ShapeKind::Star => "star",
        ShapeKind::Arrow => "arrow",
        ShapeKind::DoubleArrow => "doubleArrow",
        ShapeKind::Cloud => "cloud",
        ShapeKind::Terminator => "terminator",
        ShapeKind::Parallelogram => "parallelogram",
        ShapeKind::Trapezoid => "trapezoid",
        ShapeKind::Hexagon => "hexagon",
        ShapeKind::Document => "document",
        ShapeKind::PredefinedProcess => "predefinedProcess",
        ShapeKind::ManualInput => "manualInput",
        ShapeKind::Cylinder => "cylinder",
        ShapeKind::Delay => "delay",
        ShapeKind::OffPage => "offPage",
        ShapeKind::Note => "note",
        ShapeKind::Package => "package",
        ShapeKind::UmlClass => "umlClass",
        ShapeKind::Actor => "actor",
        ShapeKind::Pool => "pool",
        ShapeKind::Server => "server",
        ShapeKind::Monitor => "monitor",
    }
}

impl From<ShapeKind> for ObjectKind {
    fn from(kind: ShapeKind) -> Self {
        Self::Shape(kind.into())
    }
}

/// A frame's own data; its area is the object's bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameData {
    pub title: Arc<str>,
}

impl FrameData {
    /// Font size of titles, in logical pixels whatever the zoom, so they stay readable.
    pub const TITLE_SIZE: f64 = 13.0;
    /// Space between a title and its frame, in logical pixels.
    pub const TITLE_GAP: f64 = 6.0;
    /// Line height of titles, relative to their size.
    const TITLE_LINE: f64 = 1.2;
    /// Side of a frame placed with a click, in logical pixels at the zoom it is placed at:
    /// 16:9, like a slide.
    pub const CLICK_SIZE: (f64, f64) = (640.0, 360.0);

    /// The band above a frame with `bounds` (world) where its title sits and can be
    /// clicked, at `zoom`.
    pub fn title_band(bounds: Rect, zoom: f64) -> Rect {
        let height = (Self::TITLE_SIZE * Self::TITLE_LINE + Self::TITLE_GAP) / zoom;
        Rect::new(bounds.x0, bounds.y0 - height, bounds.x1, bounds.y0)
    }
}

/// Where text fitted into an object goes: wrapped at the area's width, aligned in it, and
/// drawn at `max_size`, or smaller if it does not fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBox {
    pub area: Rect,
    pub max_size: f64,
    pub align: HAlign,
    pub valign: VAlign,
}

impl TextBox {
    /// Top of text `height` tall, aligned in the area.
    pub fn top(&self, height: f64) -> f64 {
        let area = self.area;
        match self.valign {
            VAlign::Top => area.y0,
            VAlign::Middle => area.y0 + (area.height() - height) / 2.0,
            VAlign::Bottom => area.y1 - height,
        }
    }
}

/// How lines of text sit across their box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Where text sits from top to bottom of its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

impl HAlign {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    /// Stored form. Never change it: saved boards depend on it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == s)
    }
}

impl VAlign {
    pub const ALL: [Self; 3] = [Self::Top, Self::Middle, Self::Bottom];

    /// Stored form. Never change it: saved boards depend on it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Middle => "middle",
            Self::Bottom => "bottom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == s)
    }
}

/// A geometric shape with text inside. The text's color and size are part of the object's
/// [`Style`]; by default it is drawn as large as fits.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub text: Arc<str>,
    /// Inline styles of `text`.
    pub spans: Spans,
}

impl Shape {
    /// Space between the shape's text area and its text, relative to the shape's smaller
    /// side.
    const PADDING: f64 = 0.04;
    /// Largest font size, relative to the shape's smaller side. It does not depend on the
    /// kind, so short texts in shapes of one size match; long ones shrink to fit.
    const MAX_FONT: f64 = 0.15;

    /// Where the text of a `kind` shape with `bounds` goes, sized and aligned as `style`
    /// says.
    pub fn text_box(kind: ShapeKind, bounds: Rect, style: &Style) -> TextBox {
        let side = bounds.width().min(bounds.height());
        let area = kind.text_area(bounds);
        let pad = (side * Self::PADDING).min(area.width().min(area.height()) / 4.0);
        TextBox {
            area: area.inset(-pad),
            max_size: style.font_size.unwrap_or(side * Self::MAX_FONT),
            align: style.text_align,
            valign: style.text_valign,
        }
    }
}

impl From<ShapeKind> for Shape {
    fn from(kind: ShapeKind) -> Self {
        Self {
            kind,
            text: Arc::from(""),
            spans: Spans::default(),
        }
    }
}

/// Color as `0xRRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub u32);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(u32::from_be_bytes([r, g, b, 0xFF]))
    }

    pub const fn to_bytes(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub fill: Rgba,
    pub stroke: Rgba,
    /// In world units, so it scales with zoom. Zero means no outline.
    pub stroke_width: f64,
    /// Opacity of the whole object in `[0, 1]`, applied on top of the colors' own alpha.
    pub opacity: f64,
    /// Color of a shape's text; `None` draws it in the outline color. Other objects ignore
    /// it.
    pub text_color: Option<Rgba>,
    /// Font size of a shape's text, in world units; `None` makes it as large as fits. Text
    /// that does not fit shrinks either way. Other objects ignore it.
    pub font_size: Option<f64>,
    /// Alignment of a shape's text; other objects ignore it.
    pub text_align: HAlign,
    pub text_valign: VAlign,
}

impl Style {
    /// How new shapes look: an outline with no fill.
    pub fn shape() -> Self {
        Self {
            fill: Rgba(0),
            ..Self::default()
        }
    }

    /// Color of a shape's text.
    pub fn text_color(&self) -> Rgba {
        self.text_color.unwrap_or(self.stroke)
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Rgba::rgb(0xFF, 0xFF, 0xFF),
            stroke: Rgba::rgb(0x1F, 0x29, 0x37),
            stroke_width: 2.0,
            opacity: 1.0,
            text_color: None,
            font_size: None,
            text_align: HAlign::Center,
            text_valign: VAlign::Middle,
        }
    }
}

/// A partial style edit: only the set fields change, so concurrent edits of different
/// properties all survive a merge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StyleChange {
    pub fill: Option<Rgba>,
    pub stroke: Option<Rgba>,
    pub stroke_width: Option<f64>,
    pub opacity: Option<f64>,
    pub text_color: Option<Rgba>,
    /// `Some(None)` switches back to fitting the text.
    pub font_size: Option<Option<f64>>,
    pub text_align: Option<HAlign>,
    pub text_valign: Option<VAlign>,
}

impl StyleChange {
    /// Smallest font size that can be set.
    pub const MIN_FONT_SIZE: f64 = 1.0;

    /// `style` with this change applied, values clamped to their valid ranges.
    pub fn apply_to(&self, style: Style) -> Style {
        Style {
            fill: self.fill.unwrap_or(style.fill),
            stroke: self.stroke.unwrap_or(style.stroke),
            stroke_width: self.stroke_width.unwrap_or(style.stroke_width).max(0.0),
            opacity: self.opacity.unwrap_or(style.opacity).clamp(0.0, 1.0),
            text_color: self.text_color.or(style.text_color),
            font_size: self
                .font_size
                .unwrap_or(style.font_size)
                .map(|size| size.max(Self::MIN_FONT_SIZE)),
            text_align: self.text_align.unwrap_or(style.text_align),
            text_valign: self.text_valign.unwrap_or(style.text_valign),
        }
    }

    /// The part of this change that applies to an object of `kind`: only shapes and tables
    /// have text styles; kanban boards have a text color only, their layout is fixed.
    pub fn for_kind(mut self, kind: &ObjectKind) -> Self {
        if !matches!(kind, ObjectKind::Shape(_) | ObjectKind::Table(_)) {
            if !matches!(
                kind,
                ObjectKind::Kanban(_)
                    | ObjectKind::MindMap(_)
                    | ObjectKind::Doc(_)
                    | ObjectKind::Embed(_)
            ) {
                self.text_color = None;
            }
            self.font_size = None;
            self.text_align = None;
            self.text_valign = None;
        }
        self
    }
}

/// Plain text.
///
/// TODO: store it as a Loro text container once it can be edited, so concurrent edits merge.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub text: Arc<str>,
    /// Inline styles of `text`.
    pub spans: Spans,
    /// Font size in world units.
    pub size: f64,
}

/// A card for a web page: its site, title, description and address. The object's link
/// opens the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Embed {
    pub url: Arc<str>,
    pub title: Arc<str>,
    pub description: Arc<str>,
    /// The site's name, or its host when the page does not say.
    pub site: Arc<str>,
}

impl Embed {
    /// Size of a new card, in logical pixels at the zoom it is placed at.
    pub const SIZE: (f64, f64) = (340.0, 130.0);
    /// Space around the card's text and the colored strip at its left, relative to its
    /// height at [`Self::SIZE`].
    const PAD: f64 = 14.0;
    const STRIP: f64 = 5.0;

    /// A card for `url` before anything is known about the page: titled by its address.
    pub fn new(url: &str) -> Self {
        let host = host(url);
        let rest = url
            .split_once("://")
            .map_or(url, |(_, r)| r)
            .trim_start_matches("www.")
            .trim_end_matches('/');
        Self {
            url: url.into(),
            title: rest.into(),
            description: "".into(),
            site: host.into(),
        }
    }

    /// The card's scale: its height over the height at [`Self::SIZE`].
    fn unit(bounds: Rect) -> f64 {
        bounds.height() / Self::SIZE.1
    }

    /// The colored strip along the card's left edge.
    pub fn strip(bounds: Rect) -> Rect {
        Rect::new(
            bounds.x0,
            bounds.y0,
            bounds.x0 + Self::STRIP * Self::unit(bounds),
            bounds.y1,
        )
    }

    /// Where the site's name, title, description and address go: text boxes from the top.
    pub fn boxes(bounds: Rect) -> [TextBox; 4] {
        let u = Self::unit(bounds);
        let (x0, x1) = (
            bounds.x0 + (Self::STRIP + Self::PAD) * u,
            (bounds.x1 - Self::PAD * u).max(bounds.x0 + (Self::STRIP + Self::PAD + 1.0) * u),
        );
        let mut y = bounds.y0 + Self::PAD * 0.8 * u;
        let mut next = |height: f64, size: f64| {
            let area = Rect::new(x0, y, x1, (y + height * u).min(bounds.y1));
            y += height * u;
            TextBox {
                area,
                max_size: size * u,
                align: HAlign::Left,
                valign: VAlign::Top,
            }
        };
        let site = next(18.0, 12.0);
        let title = next(42.0, 16.0);
        let description = next(36.0, 13.0);
        let url = TextBox {
            area: Rect::new(
                x0,
                bounds.y1 - (Self::PAD * 0.8 + 15.0) * u,
                x1,
                bounds.y1 - Self::PAD * 0.6 * u,
            ),
            max_size: 11.0 * u,
            align: HAlign::Left,
            valign: VAlign::Bottom,
        };
        [site, title, description, url]
    }

    /// Default look: a white card with a light outline and a blue strip.
    pub fn style() -> Style {
        Style {
            fill: Rgba::rgb(0xFF, 0xFF, 0xFF),
            stroke: Rgba::rgb(0xD0, 0xD4, 0xDA),
            stroke_width: 1.0,
            text_color: Some(Rgba::rgb(0x1F, 0x29, 0x37)),
            ..Style::default()
        }
    }
}

/// The host of `url`, without `www.`: `example.com` for `https://www.example.com/a`.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let rest = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    rest.trim_start_matches("www.")
}

/// A Markdown document: its source, drawn formatted on a page over the object's bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub text: Arc<str>,
    /// Body text size in world units; headings are larger.
    pub size: f64,
}

impl Doc {
    /// Space between the page's edge and its text, relative to the body text size.
    pub const PADDING: f64 = 1.0;
    /// Width of a new document, in logical pixels at the zoom it is placed at.
    pub const WIDTH: f64 = 480.0;
    /// Body text size of a new document, in logical pixels at the zoom it is placed at.
    pub const SIZE: f64 = 16.0;

    /// Default look: a white page with a light outline and dark text.
    pub fn style() -> Style {
        Style {
            fill: Rgba::rgb(0xFF, 0xFF, 0xFF),
            stroke: Rgba::rgb(0xD0, 0xD4, 0xDA),
            stroke_width: 1.0,
            text_color: Some(Rgba::rgb(0x1F, 0x29, 0x37)),
            ..Style::default()
        }
    }
}

/// A sticky note's text. Its font size is not stored: the text is drawn as large as fits.
#[derive(Clone, Debug, PartialEq)]
pub struct Sticky {
    pub text: Arc<str>,
    /// Inline styles of `text`.
    pub spans: Spans,
}

impl Sticky {
    /// Side of a new sticky note, in logical pixels at the zoom it is created at.
    pub const SIZE: f64 = 200.0;
    /// Space between the note's edge and its text, relative to its smaller side.
    const PADDING: f64 = 0.08;
    /// Largest font size, relative to the note's smaller side.
    const MAX_FONT: f64 = 0.16;

    /// The area text is laid out in, for a note with `bounds`.
    pub fn text_area(bounds: Rect) -> Rect {
        let pad = bounds.width().min(bounds.height()) * Self::PADDING;
        let area = bounds.inset(-pad);
        if area.width() > 0.0 && area.height() > 0.0 {
            area
        } else {
            Rect::from_center_size(bounds.center(), (0.0, 0.0))
        }
    }

    /// The font size used when even a single short line fits.
    pub fn max_font_size(bounds: Rect) -> f64 {
        bounds.width().min(bounds.height()) * Self::MAX_FONT
    }

    /// Where the text of a note with `bounds` goes.
    pub fn text_box(bounds: Rect) -> TextBox {
        TextBox {
            area: Self::text_area(bounds),
            max_size: Self::max_font_size(bounds),
            align: HAlign::Center,
            valign: VAlign::Middle,
        }
    }
}

/// A picture on the board.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    /// The image file, in [`crate::Assets`].
    pub asset: AssetId,
    /// The part of the picture shown, as fractions of its width and height: [`Image::FULL`]
    /// shows all of it.
    pub crop: Rect,
    /// For a PDF shown a page at a time: the file and the page; `asset` is that page.
    pub book: Option<Book>,
}

impl Image {
    pub const FULL: Rect = Rect::new(0.0, 0.0, 1.0, 1.0);

    pub fn new(asset: AssetId) -> Self {
        Self {
            asset,
            crop: Self::FULL,
            book: None,
        }
    }
}

/// A PDF on the board like a book: one page shown at a time, turned in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Book {
    /// The PDF file, in [`crate::Assets`].
    pub pdf: AssetId,
    /// The page shown, from 0.
    pub page: u32,
    /// How many pages the file has.
    pub pages: u32,
}

/// A point of a freehand stroke, relative to the object's bounds: `(0, 0)` is the top-left
/// corner and `(1, 1)` the bottom-right one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    /// Pen pressure in `[0, 1]`; 1 for a mouse.
    pub pressure: f32,
}

/// A freehand stroke. Its points are relative to the object's bounds, so moving, resizing
/// and rotating the object reshape the stroke with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ink {
    points: Arc<[InkPoint]>,
}

impl Ink {
    /// A stroke through world points with their pressures, and the bounds it spans.
    /// `None` for no points.
    pub fn from_world(points: &[(Point, f64)]) -> Option<(Self, Rect)> {
        let (first, _) = points.first()?;
        let bounds = points
            .iter()
            .fold(Rect::from_points(*first, *first), |r, (p, _)| {
                r.union_pt(*p)
            });
        let relative = |v: f64, min: f64, size: f64| {
            if size > 0.0 {
                ((v - min) / size) as f32
            } else {
                0.5
            }
        };
        let points = points
            .iter()
            .map(|&(p, pressure)| InkPoint {
                x: relative(p.x, bounds.x0, bounds.width()),
                y: relative(p.y, bounds.y0, bounds.height()),
                pressure: pressure.clamp(0.0, 1.0) as f32,
            })
            .collect();
        Some((Self { points }, bounds))
    }

    pub fn points(&self) -> &[InkPoint] {
        &self.points
    }

    /// The points placed in `bounds` (unrotated), with their pressures.
    pub fn points_in(&self, bounds: Rect) -> Vec<(Point, f64)> {
        self.points
            .iter()
            .map(|p| {
                let x = bounds.x0 + f64::from(p.x) * bounds.width();
                let y = bounds.y0 + f64::from(p.y) * bounds.height();
                (Point::new(x, y), f64::from(p.pressure))
            })
            .collect()
    }

    /// Whether the pressure varies, so the stroke needs a variable-width outline.
    pub fn has_pressure(&self) -> bool {
        self.points.iter().any(|p| p.pressure != 1.0)
    }

    /// Stored form: little-endian `f32` triples `x, y, pressure`.
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.points.len() * 12);
        for p in self.points.iter() {
            for v in [p.x, p.y, p.pressure] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        bytes
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(12) {
            return None;
        }
        let f = |b: &[u8]| f32::from_le_bytes(b.try_into().expect("chunk of 4 bytes"));
        let (chunks, _) = bytes.as_chunks::<12>();
        let points = chunks
            .iter()
            .map(|c| InkPoint {
                x: f(&c[0..4]),
                y: f(&c[4..8]),
                pressure: f(&c[8..12]),
            })
            .collect();
        Some(Self { points })
    }
}

/// Everything about an object except its identity.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectData {
    pub kind: ObjectKind,
    /// Unrotated bounds in world coordinates, always normalized (`x0 <= x1`, `y0 <= y1`).
    pub bounds: Rect,
    /// Rotation around the center of `bounds`, in radians, clockwise on screen.
    pub rotation: f64,
    pub style: Style,
    /// Ids of the board's tags on it (see [`crate::Document::tags`]); unknown ones are
    /// ignored.
    pub tags: Tags,
    /// Where clicking its link badge goes.
    pub link: Option<Link>,
}

/// Where an object links to.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    /// A web page or mail address: `http`, `https` or `mailto` only.
    Url(Arc<str>),
    /// Another object on the board, often a frame.
    Object(ObjectId),
}

impl Link {
    /// Text that links to object `id` when given to [`Link::from_input`], for the clipboard.
    pub fn anchor(id: ObjectId) -> String {
        format!("{ANCHOR_PREFIX}{id}")
    }

    /// A link from what someone typed or pasted: an anchor copied from an object, or a web
    /// address (`https://` is added when there is no scheme). `None` for anything else.
    pub fn from_input(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(id) = text.strip_prefix(ANCHOR_PREFIX) {
            return ObjectId::parse(id).map(Self::Object);
        }
        if text.is_empty() || text.chars().any(char::is_whitespace) {
            return None;
        }
        let lower = text.to_ascii_lowercase();
        if ["http://", "https://", "mailto:"]
            .iter()
            .any(|s| lower.starts_with(s))
        {
            return Some(Self::Url(text.into()));
        }
        // Something like `example.com/page` or `example.com:8080`, but not another scheme
        // (`file:`, `javascript:`): the host needs a dot and only digits may follow a colon.
        let host = text.split(['/', '?', '#']).next().unwrap_or("");
        let (name, port) = host.split_once(':').unwrap_or((host, ""));
        let plausible = name.contains('.')
            && !name.starts_with('.')
            && !name.ends_with('.')
            && port.chars().all(|c| c.is_ascii_digit());
        plausible.then(|| Self::Url(format!("https://{text}").into()))
    }

    /// The stored form: `url:` or `obj:` followed by the address or object id.
    pub(crate) fn to_stored(&self) -> String {
        match self {
            Self::Url(url) => format!("url:{url}"),
            Self::Object(id) => format!("obj:{id}"),
        }
    }

    pub(crate) fn from_stored(s: &str) -> Option<Self> {
        if let Some(url) = s.strip_prefix("url:") {
            return Some(Self::Url(url.into()));
        }
        s.strip_prefix("obj:")
            .and_then(ObjectId::parse)
            .map(Self::Object)
    }
}

/// How an anchor to an object starts.
const ANCHOR_PREFIX: &str = "pboard://object/";

/// Tag ids, in the order they were added.
pub type Tags = Arc<[Arc<str>]>;

/// A label objects can carry, defined once per board.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    /// Unique across replicas (see [`crate::Document::new_line_id`]).
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub color: Rgba,
}

impl Tag {
    /// Colors offered for new tags, in the order they are handed out.
    pub const COLORS: [Rgba; 8] = [
        Rgba::rgb(0xE5, 0x48, 0x4D),
        Rgba::rgb(0xF5, 0x9E, 0x0B),
        Rgba::rgb(0x22, 0xA0, 0x6B),
        Rgba::rgb(0x3B, 0x82, 0xF6),
        Rgba::rgb(0x8B, 0x5C, 0xF6),
        Rgba::rgb(0xEC, 0x48, 0x99),
        Rgba::rgb(0x14, 0xB8, 0xA6),
        Rgba::rgb(0x6B, 0x72, 0x80),
    ];
}

/// What a filter on the board lets through: objects with any of `tags`, or filled with
/// any of `colors`. Empty, it lets everything through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub tags: Vec<Arc<str>>,
    pub colors: Vec<Rgba>,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.colors.is_empty()
    }

    pub fn matches(&self, data: &ObjectData) -> bool {
        self.is_empty()
            || data.tags.iter().any(|t| self.tags.contains(t))
            || (data.kind.has_fill() && self.colors.contains(&data.style.fill))
    }

    /// Turns `tag` on or off.
    pub fn toggle_tag(&mut self, tag: &Arc<str>) {
        match self.tags.iter().position(|t| t == tag) {
            Some(i) => {
                self.tags.remove(i);
            }
            None => self.tags.push(tag.clone()),
        }
    }

    /// Turns `color` on or off.
    pub fn toggle_color(&mut self, color: Rgba) {
        match self.colors.iter().position(|&c| c == color) {
            Some(i) => {
                self.colors.remove(i);
            }
            None => self.colors.push(color),
        }
    }
}

impl ObjectData {
    /// Where the text of a sticky note or shape goes; `None` for objects whose text is not
    /// placed inside them.
    pub fn text_box(&self) -> Option<TextBox> {
        match &self.kind {
            ObjectKind::Sticky(_) => Some(Sticky::text_box(self.bounds)),
            ObjectKind::Shape(shape) => Some(Shape::text_box(shape.kind, self.bounds, &self.style)),
            ObjectKind::Text(_) | ObjectKind::Ink(_) | ObjectKind::Image(_) => None,
            ObjectKind::Connector(_) | ObjectKind::Frame(_) => None,
            ObjectKind::Table(_) | ObjectKind::Kanban(_) | ObjectKind::MindMap(_) => None,
            ObjectKind::Doc(_) => None,
            ObjectKind::Embed(_) => Some(Embed::boxes(self.bounds)[1]),
        }
    }

    pub fn frame(&self) -> Frame {
        Frame::new(self.bounds, self.rotation)
    }

    pub fn set_frame(&mut self, frame: Frame) {
        self.bounds = frame.rect.abs();
        self.rotation = frame.rotation;
    }

    /// Moves the object; a connector moves its free ends (attached ones follow objects).
    pub fn translate(&mut self, delta: Vec2) {
        self.bounds = self.bounds + delta;
        if let ObjectKind::Connector(c) = &mut self.kind {
            c.start.translate(delta);
            c.end.translate(delta);
            c.bends = c.bends.iter().map(|&p| p + delta).collect();
        }
    }
}

/// An object on the board, as cached by the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub id: ObjectId,
    pub data: ObjectData,
    /// The group directly containing this object.
    pub parent: Option<ObjectId>,
    /// Set on the object itself; see [`crate::Document::is_locked`] for the effective state.
    pub locked: bool,
    /// Set on the object itself; see [`crate::Document::is_hidden`] for the effective state.
    pub hidden: bool,
}

/// Objects and nested groups that move, select and stack as one unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: ObjectId,
    pub parent: Option<ObjectId>,
    /// Direct children in z-order, back to front. Never empty.
    pub children: Vec<ObjectId>,
    pub locked: bool,
    pub hidden: bool,
}

/// New board content: a shape, or a group of further content (paste keeps groups).
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Object(ObjectData),
    /// A copy of an existing object, so connectors copied with it can attach to the copy.
    Copied {
        from: ObjectId,
        data: ObjectData,
    },
    Group(Vec<Content>),
}

impl Content {
    /// The shapes inside, back to front.
    pub fn shapes(&self) -> Vec<ObjectData> {
        let mut out = Vec::new();
        self.collect_shapes(&mut out);
        out
    }

    fn collect_shapes(&self, out: &mut Vec<ObjectData>) {
        match self {
            Self::Object(data) | Self::Copied { data, .. } => out.push(data.clone()),
            Self::Group(children) => children.iter().for_each(|c| c.collect_shapes(out)),
        }
    }

    /// Moves every shape inside by `delta`.
    pub fn translate(&mut self, delta: Vec2) {
        match self {
            Self::Object(data) | Self::Copied { data, .. } => data.translate(delta),
            Self::Group(children) => children.iter_mut().for_each(|c| c.translate(delta)),
        }
    }
}

impl From<ObjectData> for Content {
    fn from(data: ObjectData) -> Self {
        Self::Object(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_font_size_is_set_or_follows_the_size() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let fitted = Shape::text_box(ShapeKind::Rectangle, bounds, &Style::default());
        let style = Style {
            font_size: Some(40.0),
            ..Style::default()
        };
        let set = Shape::text_box(ShapeKind::Rectangle, bounds, &style);
        assert_eq!(fitted.area, set.area);
        assert_eq!(set.max_size, 40.0);
        // Equal shapes of different kinds get the same largest size.
        let diamond = Shape::text_box(ShapeKind::Diamond, bounds, &Style::default());
        assert_eq!(diamond.max_size, fitted.max_size);
        assert!(diamond.area.width() < fitted.area.width());
    }

    #[test]
    fn text_sits_where_it_is_aligned() {
        let mut text_box = Shape::text_box(
            ShapeKind::Rectangle,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            &Style::default(),
        );
        let area = text_box.area;
        assert_eq!(text_box.top(20.0), area.y0 + (area.height() - 20.0) / 2.0);
        text_box.valign = VAlign::Top;
        assert_eq!(text_box.top(20.0), area.y0);
        text_box.valign = VAlign::Bottom;
        assert_eq!(text_box.top(20.0), area.y1 - 20.0);
        for a in HAlign::ALL {
            assert_eq!(HAlign::parse(a.as_str()), Some(a));
        }
        for a in VAlign::ALL {
            assert_eq!(VAlign::parse(a.as_str()), Some(a));
        }
    }
}
