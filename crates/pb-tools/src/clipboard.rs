use std::sync::Arc;

use pb_core::{Command, Content, Document, ObjectKind};
use pb_geom::kurbo::{Point, Vec2};

use crate::Selection;

/// How far a duplicate or a repeated paste lands from the original, in world units.
const PASTE_OFFSET: f64 = 20.0;

/// In-app clipboard for board objects.
///
/// TODO: mirror to the system clipboard through pb-platform once objects have a serialized
/// form, so content can travel between boards and app instances.
#[derive(Default)]
pub struct Clipboard {
    items: Vec<Content>,
    /// Files of the copied images, so they can be pasted into another board.
    assets: Vec<Arc<[u8]>>,
    /// Pastes since the last copy, so repeated pastes cascade instead of stacking.
    pastes: u32,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns false if nothing was selected.
    pub fn copy(&mut self, doc: &Document, selection: &Selection) -> bool {
        let (items, assets) = copy_of(doc, selection);
        if items.is_empty() {
            return false;
        }
        self.assets = assets;
        self.items = items;
        self.pastes = 0;
        true
    }

    /// Files the pasted images need; add them to the board before applying a paste.
    pub fn assets(&self) -> &[Arc<[u8]>] {
        &self.assets
    }

    /// Pastes centered on `at` (the pointer), or else offset from the originals.
    pub fn paste(&mut self, at: Option<Point>) -> Option<Command> {
        let bounds = self
            .items
            .iter()
            .flat_map(Content::shapes)
            .map(|d| d.frame().aabb())
            .reduce(|a, b| a.union(b))?;
        let delta = match at {
            Some(at) => at - bounds.center(),
            None => {
                self.pastes += 1;
                Vec2::splat(PASTE_OFFSET * f64::from(self.pastes))
            }
        };
        Some(Command::Create(offset(&self.items, delta)))
    }
}

/// The selection as copied: its objects back to front (with what selected frames hold),
/// and the files its images need.
pub fn copy_of(doc: &Document, selection: &Selection) -> (Vec<Content>, Vec<Arc<[u8]>>) {
    let items = in_z_order(doc, selection);
    let assets = items
        .iter()
        .flat_map(Content::shapes)
        .flat_map(|data| match data.kind {
            // A book needs its PDF too, to turn pages.
            ObjectKind::Image(image) => [Some(image.asset), image.book.map(|b| b.pdf)],
            _ => [None, None],
        })
        .flatten()
        .filter_map(|asset| doc.assets().get(asset).cloned())
        .collect();
    (items, assets)
}

/// Copies of the selection next to the originals (Ctrl+D).
pub fn duplicate(doc: &Document, selection: &Selection) -> Option<Command> {
    let items = in_z_order(doc, selection);
    (!items.is_empty()).then(|| Command::Create(offset(&items, Vec2::splat(PASTE_OFFSET))))
}

/// Selected objects and groups, with what selected frames hold, back to front, so copies
/// stack the same way as the originals.
pub(crate) fn in_z_order(doc: &Document, selection: &Selection) -> Vec<Content> {
    let ids = doc.with_frame_members(selection.ids());
    doc.roots()
        .iter()
        .filter(|&id| ids.contains(id))
        .filter_map(|&id| doc.content(id))
        .collect()
}

pub(crate) fn offset(items: &[Content], delta: Vec2) -> Vec<Content> {
    items
        .iter()
        .map(|item| {
            let mut item = item.clone();
            item.translate(delta);
            item
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use pb_core::{ObjectData, ShapeKind, Style};
    use pb_geom::kurbo::Rect;

    use super::*;

    fn rect(x: f64) -> Content {
        ObjectData {
            kind: ShapeKind::Rectangle.into(),
            bounds: Rect::new(x, 0.0, x + 10.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        }
        .into()
    }

    fn setup() -> (Document, Selection) {
        let mut doc = Document::new();
        let ids = doc
            .apply(Command::Create(vec![rect(0.0), rect(100.0), rect(200.0)]))
            .unwrap()
            .created;
        let mut selection = Selection::default();
        // Selected in reverse order; copies must still follow the z-order.
        selection.set([ids[2], ids[0]]);
        (doc, selection)
    }

    fn created(cmd: Option<Command>) -> Vec<Rect> {
        match cmd {
            Some(Command::Create(items)) => items
                .iter()
                .flat_map(Content::shapes)
                .map(|d| d.bounds)
                .collect(),
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn repeated_paste_cascades() {
        let (doc, selection) = setup();
        let mut clipboard = Clipboard::default();
        assert!(clipboard.paste(None).is_none());
        assert!(clipboard.copy(&doc, &selection));
        let first = created(clipboard.paste(None));
        assert_eq!(first[0], Rect::new(20.0, 20.0, 30.0, 30.0));
        assert_eq!(first[1], Rect::new(220.0, 20.0, 230.0, 30.0));
        let second = created(clipboard.paste(None));
        assert_eq!(second[0], Rect::new(40.0, 40.0, 50.0, 50.0));
    }

    #[test]
    fn paste_at_pointer_centers_group() {
        let (doc, selection) = setup();
        let mut clipboard = Clipboard::default();
        clipboard.copy(&doc, &selection);
        // Group spans x 0..210, y 0..10: center (105, 5).
        let pasted = created(clipboard.paste(Some(Point::new(1105.0, 505.0))));
        assert_eq!(pasted[0], Rect::new(1000.0, 500.0, 1010.0, 510.0));
    }

    #[test]
    fn copy_takes_image_files_along() {
        let mut doc = Document::new();
        let asset = doc.add_asset(b"picture".to_vec());
        let data = ObjectData {
            kind: ObjectKind::Image(pb_core::Image::new(asset)),
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        let ids = doc
            .apply(Command::Create(vec![data.into(), rect(50.0)]))
            .unwrap()
            .created;
        let mut selection = Selection::default();
        selection.set(ids);
        let mut clipboard = Clipboard::default();
        clipboard.copy(&doc, &selection);
        assert_eq!(clipboard.assets().len(), 1);
        assert_eq!(&*clipboard.assets()[0], b"picture");
    }

    #[test]
    fn duplicate_offsets_selection() {
        let (doc, selection) = setup();
        let copies = created(duplicate(&doc, &selection));
        assert_eq!(copies.len(), 2);
        assert_eq!(copies[0], Rect::new(20.0, 20.0, 30.0, 30.0));
        assert!(duplicate(&doc, &Selection::default()).is_none());
    }

    #[test]
    fn copies_keep_groups() {
        let (mut doc, _) = setup();
        let ids = doc.roots().to_vec();
        let g = doc
            .apply(Command::Group(vec![ids[0], ids[1]]))
            .unwrap()
            .created[0];
        let mut selection = Selection::default();
        selection.set([g]);
        match duplicate(&doc, &selection) {
            Some(Command::Create(items)) => {
                assert!(matches!(&items[..], [Content::Group(children)] if children.len() == 2));
            }
            other => panic!("expected Create, got {other:?}"),
        }
    }
}
