//! Cropping an image: a frame over the whole picture, resized with handles or moved.
//!
//! Everything happens in the image's own unrotated coordinates, so rotated images crop the
//! same way; the cropped part stays where it was on the board.

use pb_core::{Command, Document, Image, ObjectData, ObjectId, ObjectKind};
use pb_geom::Frame;
use pb_geom::kurbo::{Point, Rect, Vec2};

use crate::handles::{self, Handle};
use crate::{CursorHint, HANDLE_HIT_RADIUS};

/// Opacity of the parts of the picture that are cropped away, relative to the image's own.
const GHOST_OPACITY: f64 = 0.35;

#[derive(Clone, Copy, Debug)]
enum Drag {
    Resize {
        handle: Handle,
        from: Rect,
        start: Point,
    },
    Move {
        from: Rect,
        start: Point,
    },
}

/// One image being cropped.
#[derive(Clone, Debug)]
pub struct Crop {
    pub id: ObjectId,
    data: ObjectData,
    image: Image,
    /// The whole picture, in the object's local (unrotated) coordinates.
    full: Rect,
    /// The part kept, in the same coordinates; always inside `full`.
    area: Rect,
    drag: Option<Drag>,
}

impl Crop {
    /// Starts cropping `id`; `None` unless it is an unlocked image.
    pub fn new(doc: &Document, id: ObjectId) -> Option<Self> {
        let object = doc.get(id).filter(|_| !doc.is_locked(id))?;
        let ObjectKind::Image(image) = object.data.kind else {
            return None;
        };
        let b = object.data.bounds;
        let c = image.crop;
        if c.width() <= 0.0 || c.height() <= 0.0 {
            return None;
        }
        let (w, h) = (b.width() / c.width(), b.height() / c.height());
        let full = Rect::new(b.x0 - c.x0 * w, b.y0 - c.y0 * h, 0.0, 0.0);
        let full = Rect::from_origin_size(full.origin(), (w, h));
        Some(Self {
            id,
            data: object.data.clone(),
            image,
            full,
            area: b,
            drag: None,
        })
    }

    /// The world frame of a rectangle in the object's local coordinates. Objects rotate
    /// about their own center, so the rectangle is placed by where its center lands.
    fn placed(&self, local: Rect) -> Frame {
        let frame = self.data.frame();
        let center = frame.to_world(local.center());
        Frame::new(Rect::from_center_size(center, local.size()), frame.rotation)
    }

    /// The kept part on the board, for the selection frame and its handles.
    pub fn area_frame(&self) -> Frame {
        self.placed(self.area)
    }

    fn crop(&self) -> Rect {
        let f = self.full;
        let fraction = |v: f64, min: f64, size: f64| (v - min) / size;
        Rect::new(
            fraction(self.area.x0, f.x0, f.width()),
            fraction(self.area.y0, f.y0, f.height()),
            fraction(self.area.x1, f.x0, f.width()),
            fraction(self.area.y1, f.y0, f.height()),
        )
    }

    /// The whole picture, faded, then the kept part over it.
    pub fn previews(&self) -> [ObjectData; 2] {
        let mut ghost = self.data.clone();
        ghost.kind = ObjectKind::Image(Image {
            crop: Image::FULL,
            ..self.image
        });
        ghost.set_frame(self.placed(self.full));
        ghost.style.opacity *= GHOST_OPACITY;
        let mut kept = self.data.clone();
        kept.kind = ObjectKind::Image(Image {
            crop: self.crop(),
            ..self.image
        });
        kept.set_frame(self.area_frame());
        [ghost, kept]
    }

    /// A press at `world`. Returns false if it is outside the picture, which ends cropping.
    pub fn press(&mut self, world: Point, zoom: f64) -> bool {
        let start = self.data.frame().to_local(world);
        let radius = HANDLE_HIT_RADIUS / zoom;
        let area = self.area_frame();
        self.drag = if let Some(handle) = handles::handle_at(area, world, radius) {
            Some(Drag::Resize {
                handle,
                from: self.area,
                start,
            })
        } else if self.full.contains(start) {
            Some(Drag::Move {
                from: self.area,
                start,
            })
        } else {
            None
        };
        self.drag.is_some()
    }

    /// Follows the pointer during a drag. `keep_aspect` (Shift) keeps a corner's proportions.
    /// Returns true if the crop changed.
    pub fn drag_to(&mut self, world: Point, keep_aspect: bool) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };
        let local = self.data.frame().to_local(world);
        let area = match drag {
            Drag::Resize {
                handle,
                from,
                start,
            } => {
                let r = handles::resize(from, handle, local - start, keep_aspect).abs();
                let r = r.intersect(self.full);
                if r.width() <= 0.0 || r.height() <= 0.0 {
                    return false;
                }
                r
            }
            Drag::Move { from, start } => {
                let moved = from + (local - start);
                let f = self.full;
                let shift = Vec2::new(
                    (f.x0 - moved.x0).max(0.0) + (f.x1 - moved.x1).min(0.0),
                    (f.y0 - moved.y0).max(0.0) + (f.y1 - moved.y1).min(0.0),
                );
                moved + shift
            }
        };
        let changed = area != self.area;
        self.area = area;
        changed
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Cursor for the pointer at `world`.
    pub fn cursor(&self, world: Point, zoom: f64) -> CursorHint {
        let area = self.area_frame();
        let handle = match self.drag {
            Some(Drag::Resize { handle, .. }) => Some(handle),
            Some(Drag::Move { .. }) => return CursorHint::Move,
            None => handles::handle_at(area, world, HANDLE_HIT_RADIUS / zoom),
        };
        match handle {
            Some(handle) => handle.cursor(area.rotation),
            None if self.full.contains(self.data.frame().to_local(world)) => CursorHint::Move,
            None => CursorHint::Default,
        }
    }

    /// The command keeping the crop, if it changed.
    pub fn finish(self) -> Option<Command> {
        let crop = self.crop();
        let bounds = self.area_frame().rect;
        let unchanged = (crop.x0 - self.image.crop.x0).abs() < 1e-9
            && (crop.y0 - self.image.crop.y0).abs() < 1e-9
            && (crop.x1 - self.image.crop.x1).abs() < 1e-9
            && (crop.y1 - self.image.crop.y1).abs() < 1e-9;
        (!unchanged).then_some(Command::SetCrop {
            id: self.id,
            crop,
            bounds,
        })
    }
}

#[cfg(test)]
mod tests {
    use pb_core::Style;

    use super::*;

    fn image(doc: &mut Document, bounds: Rect, rotation: f64, crop: Rect) -> ObjectId {
        let asset = doc.add_asset(b"img".to_vec());
        let data = ObjectData {
            kind: ObjectKind::Image(Image { asset, crop }),
            bounds,
            rotation,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        doc.apply(pb_core::Command::Create(vec![data.into()]))
            .unwrap()
            .created[0]
    }

    fn set_crop(cmd: Option<Command>) -> (Rect, Rect) {
        match cmd {
            Some(Command::SetCrop { crop, bounds, .. }) => (crop, bounds),
            other => panic!("expected SetCrop, got {other:?}"),
        }
    }

    fn close(a: Rect, b: Rect) -> bool {
        [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1]
            .iter()
            .all(|d| d.abs() < 1e-9)
    }

    #[test]
    fn dragging_a_corner_crops_and_keeps_the_rest_in_place() {
        let mut doc = Document::new();
        let id = image(
            &mut doc,
            Rect::new(0.0, 0.0, 200.0, 100.0),
            0.0,
            Image::FULL,
        );
        let mut crop = Crop::new(&doc, id).unwrap();
        assert!(crop.press(Point::new(200.0, 100.0), 1.0));
        assert!(crop.drag_to(Point::new(100.0, 50.0), false));
        crop.release();
        let (fraction, bounds) = set_crop(crop.finish());
        assert!(close(fraction, Rect::new(0.0, 0.0, 0.5, 0.5)));
        assert!(close(bounds, Rect::new(0.0, 0.0, 100.0, 50.0)));
    }

    #[test]
    fn crop_stays_inside_the_picture() {
        let mut doc = Document::new();
        let id = image(
            &mut doc,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            0.0,
            Image::FULL,
        );
        let mut crop = Crop::new(&doc, id).unwrap();
        // Growing past the edge stops at it: nothing changes.
        crop.press(Point::new(100.0, 50.0), 1.0);
        assert!(!crop.drag_to(Point::new(150.0, 50.0), false));
        crop.release();
        assert!(crop.finish().is_none());
    }

    #[test]
    fn moving_the_frame_shows_another_part() {
        let mut doc = Document::new();
        let crop_rect = Rect::new(0.0, 0.0, 0.5, 1.0);
        // The picture is 200 wide; its left half is shown at 0..100.
        let id = image(&mut doc, Rect::new(0.0, 0.0, 100.0, 50.0), 0.0, crop_rect);
        let mut crop = Crop::new(&doc, id).unwrap();
        let [ghost, kept] = crop.previews();
        assert!(close(ghost.bounds, Rect::new(0.0, 0.0, 200.0, 50.0)));
        assert_eq!(kept.bounds, Rect::new(0.0, 0.0, 100.0, 50.0));
        assert!(crop.press(Point::new(50.0, 25.0), 1.0));
        // Dragged far right, it stops at the picture's right edge.
        crop.drag_to(Point::new(500.0, 25.0), false);
        let (fraction, bounds) = set_crop(crop.finish());
        assert!(close(fraction, Rect::new(0.5, 0.0, 1.0, 1.0)));
        assert!(close(bounds, Rect::new(100.0, 0.0, 200.0, 50.0)));
    }

    #[test]
    fn rotated_images_crop_in_place() {
        let mut doc = Document::new();
        let quarter = std::f64::consts::FRAC_PI_2;
        let id = image(
            &mut doc,
            Rect::new(0.0, 0.0, 200.0, 100.0),
            quarter,
            Image::FULL,
        );
        let before = doc.get(id).unwrap().data.frame();
        let kept_corner = before.to_world(Point::new(0.0, 0.0));
        let mut crop = Crop::new(&doc, id).unwrap();
        // The local bottom-right corner, wherever the rotation put it.
        let corner = before.to_world(Point::new(200.0, 100.0));
        assert!(crop.press(corner, 1.0));
        crop.drag_to(before.to_world(Point::new(100.0, 50.0)), false);
        let (_, bounds) = set_crop(crop.finish());
        let after = Frame::new(bounds, quarter);
        // The top-left corner of the picture did not move on the board.
        let moved = after.to_world(bounds.origin()) - kept_corner;
        assert!(moved.hypot() < 1e-9, "{moved:?}");
        assert!((bounds.width() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn only_unlocked_images_crop() {
        let mut doc = Document::new();
        let id = image(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0, Image::FULL);
        doc.apply(Command::SetLocked {
            ids: vec![id],
            locked: true,
        })
        .unwrap();
        assert!(Crop::new(&doc, id).is_none());
    }
}
