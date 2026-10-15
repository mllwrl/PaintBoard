use std::fmt;

use kurbo::Rect;
use loro::TreeID;
use pb_geom::Frame;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Rectangle,
}

impl ObjectKind {
    /// Name stored in the document. Never change existing names: old files depend on them.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Rectangle => "rect",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "rect" => Some(Self::Rectangle),
            _ => None,
        }
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
    /// In world units, so it scales with zoom.
    pub stroke_width: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Rgba::rgb(0xFF, 0xFF, 0xFF),
            stroke: Rgba::rgb(0x1F, 0x29, 0x37),
            stroke_width: 2.0,
        }
    }
}

/// Everything about an object except its identity.
#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Object {
    pub id: ObjectId,
    pub data: ObjectData,
}
