//! The user's own templates: selections saved as small boards in the app data folder
//! (in the browser's storage on the web), to insert again on any board.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pb_core::{Command, Content, Document};
use pb_geom::kurbo::Point;

/// A saved template, as the gallery lists it.
#[derive(Clone, Debug)]
pub struct MyTemplate {
    pub name: String,
    pub path: PathBuf,
    /// How many objects it holds.
    pub objects: usize,
}

/// What a template puts on the board: objects around the origin, and their images' files.
pub struct Contents {
    pub items: Vec<Content>,
    pub assets: Vec<Arc<[u8]>>,
}

/// Where the templates are kept.
fn dir() -> io::Result<PathBuf> {
    Ok(pb_platform::fs::app_data_dir()?.join("templates"))
}

/// `name` made safe as a file name: no path separators or characters Windows forbids.
fn file_name(name: &str) -> String {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = safe.trim_end_matches(['.', ' ']).to_owned();
    if safe.is_empty() { "_".into() } else { safe }
}

/// The saved templates, by name.
pub fn list() -> Vec<MyTemplate> {
    let Ok(dir) = dir() else {
        return Vec::new();
    };
    list_in(&dir)
}

fn list_in(dir: &Path) -> Vec<MyTemplate> {
    let mut out: Vec<MyTemplate> = pb_platform::fs::list(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == pb_storage::EXTENSION))
        .filter_map(|path| {
            let contents = load(&path).ok()?;
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let objects = contents.items.iter().flat_map(Content::shapes).count();
            Some(MyTemplate {
                name,
                path,
                objects,
            })
        })
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    out
}

/// Saves `items` (with the files of their images) as template `name`, replacing one of
/// that name. The objects are kept centered on the origin.
pub fn save(name: &str, items: &[Content], assets: &[Arc<[u8]>]) -> io::Result<PathBuf> {
    let path = dir()?.join(format!("{}.{}", file_name(name), pb_storage::EXTENSION));
    pb_platform::fs::write_atomic(&path, &encode(items, assets))?;
    Ok(path)
}

fn encode(items: &[Content], assets: &[Arc<[u8]>]) -> Vec<u8> {
    let mut doc = Document::new();
    for bytes in assets {
        doc.add_asset(bytes.clone());
    }
    let centered = around(items, Point::ZERO);
    if !centered.is_empty() {
        let _ = doc.apply(Command::Create(centered));
    }
    pb_storage::encode(&doc)
}

/// What template file `path` holds.
pub fn load(path: &Path) -> Result<Contents, String> {
    let bytes = pb_platform::fs::read(path).map_err(|e| e.to_string())?;
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Result<Contents, String> {
    let doc = pb_storage::decode(bytes).map_err(|e| e.to_string())?;
    let mut all = pb_tools::Selection::default();
    all.set(doc.roots().iter().copied());
    let (items, assets) = pb_tools::copy_of(&doc, &all);
    Ok(Contents { items, assets })
}

/// Removes template file `path`.
pub fn remove(path: &Path) -> io::Result<()> {
    pb_platform::fs::remove(path)
}

/// `items` moved so that their bounds are centered on `center`.
pub fn around(items: &[Content], center: Point) -> Vec<Content> {
    let Some(bounds) = items
        .iter()
        .flat_map(Content::shapes)
        .map(|d| d.frame().aabb())
        .reduce(|a, b| a.union(b))
    else {
        return Vec::new();
    };
    let delta = center - bounds.center();
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
    use pb_core::{Image, ObjectData, ObjectKind, ShapeKind, Style};
    use pb_geom::kurbo::Rect;

    use super::*;

    fn object(kind: ObjectKind, x: f64) -> Content {
        ObjectData {
            kind,
            bounds: Rect::new(x, 100.0, x + 10.0, 110.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        }
        .into()
    }

    #[test]
    fn saved_templates_keep_objects_and_pictures() {
        let mut board = Document::new();
        let asset = board.add_asset(b"picture".to_vec());
        let ids = board
            .apply(Command::Create(vec![
                object(ShapeKind::Rectangle.into(), 0.0),
                object(ObjectKind::Image(Image::new(asset)), 90.0),
            ]))
            .unwrap()
            .created;
        let mut selection = pb_tools::Selection::default();
        selection.set(ids);
        let (items, assets) = pb_tools::copy_of(&board, &selection);
        let back = decode(&encode(&items, &assets)).unwrap();
        assert_eq!(back.items.len(), 2);
        assert_eq!(back.assets.len(), 1);
        assert_eq!(&*back.assets[0], b"picture");
        // Kept around the origin.
        let bounds = back
            .items
            .iter()
            .flat_map(Content::shapes)
            .map(|d| d.bounds)
            .reduce(|a, b| a.union(b))
            .unwrap();
        assert!(bounds.center().to_vec2().hypot() < 1e-9);
        // Inserted into another board: the picture comes along.
        let mut other = Document::new();
        for bytes in &back.assets {
            other.add_asset(bytes.clone());
        }
        let placed = around(&back.items, Point::new(500.0, 500.0));
        other.apply(Command::Create(placed)).unwrap();
        assert_eq!(other.roots().len(), 2);
        assert!(other.assets().get(asset).is_some());
    }

    #[test]
    fn names_become_safe_file_names() {
        assert_eq!(file_name("  Team: plan/v2?  "), "Team_ plan_v2_");
        assert_eq!(file_name("..."), "_");
        assert_eq!(file_name("Ретро"), "Ретро");
    }

    #[test]
    fn templates_in_a_folder_are_listed_with_their_sizes() {
        let dir = std::env::temp_dir().join(format!("pb-my-templates-{}", std::process::id()));
        let items = vec![
            object(ShapeKind::Rectangle.into(), 0.0),
            object(ShapeKind::Ellipse.into(), 50.0),
        ];
        pb_platform::fs::write_atomic(&dir.join("b.pboard"), &encode(&items, &[])).unwrap();
        pb_platform::fs::write_atomic(&dir.join("a.pboard"), &encode(&items[..1], &[])).unwrap();
        pb_platform::fs::write_atomic(&dir.join("junk.pboard"), b"not a board").unwrap();
        let listed = list_in(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let names: Vec<_> = listed
            .iter()
            .map(|t| (t.name.as_str(), t.objects))
            .collect();
        assert_eq!(names, [("a", 1), ("b", 2)]);
    }
}
