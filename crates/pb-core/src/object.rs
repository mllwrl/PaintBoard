use std::fmt;

use std::sync::Arc;

use kurbo::{Point, Rect, Vec2};
use loro::TreeID;
use pb_geom::{Frame, ShapeKind};

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

    pub(crate) fn parse(s: &str) -> Option<Self> {
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
    Shape(ShapeKind),
    /// Freehand drawing; drawn with the outline color and width.
    Ink(Ink),
    /// Text in the outline color, wrapped at the object's width and centered in it.
    Text(Text),
    /// A sticky note: a square of the fill color with text in the outline color, sized to
    /// fit. It has no outline.
    Sticky(Sticky),
}

impl ObjectKind {
    /// Name stored in the document. Never change existing names: old files depend on them.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Ink(_) => "ink",
            Self::Text(_) => "text",
            Self::Sticky(_) => "sticky",
            Self::Shape(shape) => match shape {
                ShapeKind::Rectangle => "rect",
                ShapeKind::RoundedRectangle => "roundRect",
                ShapeKind::Ellipse => "ellipse",
                ShapeKind::Diamond => "diamond",
                ShapeKind::Triangle => "triangle",
                ShapeKind::Star => "star",
                ShapeKind::Arrow => "arrow",
                ShapeKind::DoubleArrow => "doubleArrow",
                ShapeKind::Cloud => "cloud",
            },
        }
    }

    /// The shape stored under `s`. Ink is read separately, with its points.
    pub(crate) fn parse_shape(s: &str) -> Option<ShapeKind> {
        ShapeKind::ALL
            .into_iter()
            .find(|&shape| Self::Shape(shape).as_str() == s)
    }

    pub fn is_ink(&self) -> bool {
        matches!(self, Self::Ink(_))
    }

    /// The text of a text object or a sticky note.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(&text.text),
            Self::Sticky(sticky) => Some(&sticky.text),
            Self::Shape(_) | Self::Ink(_) => None,
        }
    }
}

impl From<ShapeKind> for ObjectKind {
    fn from(shape: ShapeKind) -> Self {
        Self::Shape(shape)
    }
}

/// Color as `0xRRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Rgba::rgb(0xFF, 0xFF, 0xFF),
            stroke: Rgba::rgb(0x1F, 0x29, 0x37),
            stroke_width: 2.0,
            opacity: 1.0,
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
}

impl StyleChange {
    /// `style` with this change applied, values clamped to their valid ranges.
    pub fn apply_to(&self, style: Style) -> Style {
        Style {
            fill: self.fill.unwrap_or(style.fill),
            stroke: self.stroke.unwrap_or(style.stroke),
            stroke_width: self.stroke_width.unwrap_or(style.stroke_width).max(0.0),
            opacity: self.opacity.unwrap_or(style.opacity).clamp(0.0, 1.0),
        }
    }
}

/// Plain text.
///
/// TODO: store it as a Loro text container once it can be edited, so concurrent edits merge.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub text: Arc<str>,
    /// Font size in world units.
    pub size: f64,
}

/// A sticky note's text. Its font size is not stored: the text is drawn as large as fits.
#[derive(Clone, Debug, PartialEq)]
pub struct Sticky {
    pub text: Arc<str>,
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
}

impl ObjectData {
    pub fn frame(&self) -> Frame {
        Frame::new(self.bounds, self.rotation)
    }

    pub fn set_frame(&mut self, frame: Frame) {
        self.bounds = frame.rect.abs();
        self.rotation = frame.rotation;
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
}

/// Objects and nested groups that move, select and stack as one unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: ObjectId,
    pub parent: Option<ObjectId>,
    /// Direct children in z-order, back to front. Never empty.
    pub children: Vec<ObjectId>,
    pub locked: bool,
}

/// New board content: a shape, or a group of further content (paste keeps groups).
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Object(ObjectData),
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
            Self::Object(data) => out.push(data.clone()),
            Self::Group(children) => children.iter().for_each(|c| c.collect_shapes(out)),
        }
    }

    /// Moves every shape inside by `delta`.
    pub fn translate(&mut self, delta: Vec2) {
        match self {
            Self::Object(data) => data.bounds = data.bounds + delta,
            Self::Group(children) => children.iter_mut().for_each(|c| c.translate(delta)),
        }
    }
}

impl From<ObjectData> for Content {
    fn from(data: ObjectData) -> Self {
        Self::Object(data)
    }
}
