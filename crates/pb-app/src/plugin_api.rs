//! The board as plugins see it: objects as JSON, and plugins' JSON turned into new
//! objects and commands. What each call does is in `plugins` (the calls) and in
//! `packaging/plugins/paintboard.d.ts` (for plugin authors); keep the three in step.
//!
//! An object is `{id, type, x, y, width, height, rotation, fill, stroke, strokeWidth,
//! opacity, text, tags, locked, parent}` plus, by type: `shape` (its kind) for shapes,
//! `fontSize` for text, `title` for frames, `from`/`to` (ids or null) for connectors,
//! `cells` (rows of texts) for tables, and `link` (an address or object id) when it has
//! one. Positions are the unrotated top left corner in board units; rotation is in degrees,
//! clockwise; colors are `#rrggbb` or `#rrggbbaa`.

use std::sync::Arc;

use pb_core::connector::{Anchor, Cap, Connector, End, Label, Route};
use pb_core::{
    Cell, Command, Document, FrameData, Link, Object, ObjectData, ObjectId, ObjectKind, Rgba,
    Shape, ShapeKind, Sticky, Style, StyleChange, Table, Tag, Text,
};
use pb_geom::kurbo::{Point, Rect, Size};
use serde_json::{Map, Value, json};

/// Measures text: its size at a font size, wrapped at a width if given.
pub type Measure<'a> = dyn FnMut(&str, f64, Option<f64>) -> (f64, f64) + 'a;

/// Font size of new text objects when none is given.
const TEXT_SIZE: f64 = 24.0;

pub fn color_text(color: Rgba) -> String {
    let [r, g, b, a] = color.to_bytes();
    if a == 0xFF {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub fn parse_color(text: &str) -> Result<Rgba, String> {
    let hex = text.trim().trim_start_matches('#');
    let value = u32::from_str_radix(hex, 16).map_err(|_| format!("not a color: {text}"))?;
    match hex.len() {
        6 => Ok(Rgba((value << 8) | 0xFF)),
        8 => Ok(Rgba(value)),
        _ => Err(format!("not a color: {text} (use #rrggbb or #rrggbbaa)")),
    }
}

pub fn parse_id(value: &Value) -> Result<ObjectId, String> {
    value
        .as_str()
        .and_then(ObjectId::parse)
        .ok_or_else(|| format!("not an object id: {value}"))
}

pub fn parse_ids(value: &Value) -> Result<Vec<ObjectId>, String> {
    match value {
        Value::Array(ids) => ids.iter().map(parse_id).collect(),
        one => Ok(vec![parse_id(one)?]),
    }
}

/// A link as plugins see it: the address, or the id of the object it goes to.
fn link_text(link: &Link) -> String {
    match link {
        Link::Url(url) => url.to_string(),
        Link::Object(id) => id.to_string(),
    }
}

/// A plugin's `link`: an object id (of an object on the board) or a web or mail address;
/// `null` for none.
fn parse_link(value: &Value, doc: &Document) -> Result<Option<Link>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => {
            if let Some(id) = ObjectId::parse(text.trim()) {
                return match doc.get(id).is_some() {
                    true => Ok(Some(Link::Object(id))),
                    false => Err(format!("\"link\": no object {id}")),
                };
            }
            Link::from_input(text)
                .map(Some)
                .ok_or_else(|| format!("\"link\" should be a web address or an object id: {text}"))
        }
        v => Err(format!(
            "\"link\" should be a web address or an object id, not {v}"
        )),
    }
}

/// Tags by name for what a plugin creates or changes: the board's own, and ones to define
/// first (in [`TagNames::commands`]) for names the board has no tag for.
#[derive(Default)]
pub struct TagNames {
    new: Vec<Tag>,
}

impl TagNames {
    /// The id of the tag called `name` (ignoring case), defining one if there is none.
    pub fn id(&mut self, doc: &Document, name: &str, color: Option<Rgba>) -> Arc<str> {
        let same = |t: &&Tag| t.name.to_lowercase() == name.to_lowercase();
        if let Some(tag) = self.new.iter().find(same) {
            return tag.id.clone();
        }
        if let Some(tag) = doc.tags().iter().find(same) {
            if let Some(color) = color.filter(|&c| c != tag.color) {
                self.new.push(Tag {
                    color,
                    ..tag.clone()
                });
            }
            return tag.id.clone();
        }
        let colors = Tag::COLORS;
        let n = doc.tags().len() + self.new.len();
        let tag = Tag {
            id: doc.new_line_id().into(),
            name: name.trim().into(),
            color: color.unwrap_or(colors[n % colors.len()]),
        };
        self.new.push(tag.clone());
        tag.id
    }

    /// The ids of tags named by a plugin's `tags` (a list of names).
    fn ids(&mut self, doc: &Document, value: &Value) -> Result<Vec<Arc<str>>, String> {
        let names = value
            .as_array()
            .ok_or("\"tags\" should be a list of tag names")?;
        let mut ids: Vec<Arc<str>> = Vec::new();
        for name in names {
            let name = name
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or("\"tags\" should be a list of tag names")?;
            let id = self.id(doc, name, None);
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// Commands defining the new tags (and recoloring ones given a new color).
    pub fn commands(self) -> Vec<Command> {
        self.new.into_iter().map(Command::DefineTag).collect()
    }
}

fn type_name(kind: &ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Shape(_) => "shape",
        ObjectKind::Ink(_) => "ink",
        ObjectKind::Text(_) => "text",
        ObjectKind::Sticky(_) => "sticky",
        ObjectKind::Image(_) => "image",
        ObjectKind::Connector(_) => "connector",
        ObjectKind::Frame(_) => "frame",
        ObjectKind::Table(_) => "table",
        ObjectKind::Kanban(_) => "kanban",
        ObjectKind::MindMap(_) => "mindmap",
        ObjectKind::Doc(_) => "doc",
        ObjectKind::Embed(_) => "embed",
    }
}

/// `object` as plugins see it.
pub fn object_json(doc: &Document, object: &Object) -> Value {
    let data = &object.data;
    let b = data.bounds;
    let tags: Vec<&str> = data
        .tags
        .iter()
        .filter_map(|id| doc.tag(id).map(|t| &*t.name))
        .collect();
    let mut o = json!({
        "id": object.id.to_string(),
        "type": type_name(&data.kind),
        "x": b.x0,
        "y": b.y0,
        "width": b.width(),
        "height": b.height(),
        "rotation": data.rotation.to_degrees(),
        "fill": color_text(data.style.fill),
        "stroke": color_text(data.style.stroke),
        "strokeWidth": data.style.stroke_width,
        "opacity": data.style.opacity,
        "tags": tags,
        "locked": doc.is_locked(object.id),
        "parent": object.parent.map(|p| p.to_string()),
        "link": data.link.as_ref().map(link_text),
    });
    let map = o.as_object_mut().expect("an object");
    let mut set = |key: &str, value: Value| {
        map.insert(key.to_owned(), value);
    };
    match &data.kind {
        ObjectKind::Shape(shape) => {
            set("shape", pb_core::shape_name(shape.kind).into());
            set("text", (*shape.text).into());
        }
        ObjectKind::Sticky(sticky) => set("text", (*sticky.text).into()),
        ObjectKind::Text(text) => {
            set("text", (*text.text).into());
            set("fontSize", text.size.into());
        }
        ObjectKind::Frame(frame) => {
            set("title", (*frame.title).into());
            set("text", (*frame.title).into());
        }
        ObjectKind::Connector(c) => {
            let end = |e: &End| e.object().map(|id| id.to_string());
            set("from", end(&c.start).into());
            set("to", end(&c.end).into());
            set("text", (*c.label.text).into());
        }
        ObjectKind::Doc(doc) => set("text", (*doc.text).into()),
        ObjectKind::Embed(embed) => {
            set("text", (*embed.title).into());
            set("url", (*embed.url).into());
        }
        ObjectKind::Table(table) => {
            let cells: Vec<Vec<&str>> = (0..table.rows.len())
                .map(|row| {
                    (0..table.cols.len())
                        .map(|col| table.text(row, col).0)
                        .collect()
                })
                .collect();
            set("cells", json!(cells));
        }
        _ => {}
    }
    o
}

/// The board's objects that match `filter` (`{type, tag, frame}`), back to front.
pub fn objects_json(doc: &Document, filter: &Value) -> Result<Value, String> {
    let kind = filter.get("type").and_then(Value::as_str);
    let tag = filter.get("tag").and_then(Value::as_str);
    let frame = match filter.get("frame") {
        Some(Value::Null) | None => None,
        Some(id) => Some(parse_id(id)?),
    };
    let members = frame.map(|f| doc.frame_members(f));
    let objects = doc
        .objects()
        .iter()
        .filter(|o| doc.is_shown(o.id))
        .filter(|o| kind.is_none_or(|k| type_name(&o.data.kind) == k))
        .filter(|o| {
            tag.is_none_or(|name| {
                o.data
                    .tags
                    .iter()
                    .any(|id| doc.tag(id).is_some_and(|t| &*t.name == name))
            })
        })
        .filter(|o| {
            members
                .as_ref()
                .is_none_or(|m| m.contains(&doc.top_level(o.id)))
        })
        .map(|o| object_json(doc, o))
        .collect();
    Ok(Value::Array(objects))
}

fn number(props: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    match props.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or_else(|| format!("\"{key}\" should be a number")),
    }
}

fn text(props: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match props.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(v) => Ok(Some(v.to_string())),
    }
}

fn color(props: &Map<String, Value>, key: &str) -> Result<Option<Rgba>, String> {
    match props.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => parse_color(s).map(Some),
        Some(v) => Err(format!(
            "\"{key}\" should be a color like \"#ffcc00\", not {v}"
        )),
    }
}

fn end_of(value: &Value, doc: &Document, name: &str) -> Result<End, String> {
    match value {
        Value::String(_) => {
            let id = parse_id(value)?;
            let object = doc
                .get(id)
                .ok_or_else(|| format!("\"{name}\": no object {id}"))?;
            Ok(End::Attached {
                object: id,
                anchor: Anchor::Auto,
                at: object.data.bounds.center(),
            })
        }
        Value::Object(p) => {
            let (Some(x), Some(y)) = (number(p, "x")?, number(p, "y")?) else {
                return Err(format!("\"{name}\" should be an id or {{x, y}}"));
            };
            Ok(End::Free(Point::new(x, y)))
        }
        _ => Err(format!("\"{name}\" should be an id or {{x, y}}")),
    }
}

/// A new object from a plugin's description; `center` is where it goes without `x`/`y`.
/// Tags it names that the board has none of go to `tags`, to define first.
pub fn new_object(
    item: &Value,
    doc: &Document,
    center: Point,
    measure: &mut Measure,
    tags: &mut TagNames,
) -> Result<ObjectData, String> {
    let props = item.as_object().ok_or("each item should be an object")?;
    let kind = props
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("sticky");
    let body = text(props, "text")?.unwrap_or_default();
    let place = |size: Size| -> Result<Rect, String> {
        let width = number(props, "width")?.unwrap_or(size.width).max(1.0);
        let height = number(props, "height")?.unwrap_or(size.height).max(1.0);
        let x = number(props, "x")?.unwrap_or(center.x - width / 2.0);
        let y = number(props, "y")?.unwrap_or(center.y - height / 2.0);
        Ok(Rect::new(x, y, x + width, y + height))
    };
    let dark = Rgba::rgb(0x1F, 0x29, 0x37);
    let (kind, bounds, style) = match kind {
        "sticky" => (
            ObjectKind::Sticky(Sticky {
                text: body.into(),
                spans: Default::default(),
            }),
            place(Size::new(200.0, 200.0))?,
            Style {
                fill: Rgba::rgb(0xFF, 0xEB, 0x7A),
                stroke: dark,
                stroke_width: 0.0,
                ..Style::default()
            },
        ),
        "shape" => {
            let name = props.get("shape").and_then(Value::as_str).unwrap_or("rect");
            let shape = ObjectKind::parse_shape(name).ok_or_else(|| {
                let names: Vec<_> = ShapeKind::ALL
                    .iter()
                    .map(|k| pb_core::shape_name(*k))
                    .collect();
                format!("no shape \"{name}\": one of {}", names.join(", "))
            })?;
            (
                ObjectKind::Shape(Shape {
                    kind: shape,
                    text: body.into(),
                    spans: Default::default(),
                }),
                place(Size::new(160.0, 100.0))?,
                Style::shape(),
            )
        }
        "text" => {
            let size = number(props, "fontSize")?
                .unwrap_or(TEXT_SIZE)
                .clamp(1.0, 1000.0);
            let wrap = number(props, "width")?;
            let (w, h) = measure(&body, size, wrap);
            let mut bounds = place(Size::new(w.max(1.0), h.max(1.0)))?;
            bounds.y1 = bounds.y0 + h.max(1.0);
            (
                ObjectKind::Text(Text {
                    text: body.into(),
                    spans: Default::default(),
                    size,
                }),
                bounds,
                Style {
                    stroke: dark,
                    ..Style::default()
                },
            )
        }
        "frame" => (
            ObjectKind::Frame(FrameData {
                title: text(props, "title")?.unwrap_or(body).into(),
            }),
            place(Size::new(640.0, 360.0))?,
            Style {
                stroke_width: 0.0,
                ..Style::default()
            },
        ),
        "connector" => {
            let from = end_of(props.get("from").unwrap_or(&Value::Null), doc, "from")?;
            let to = end_of(props.get("to").unwrap_or(&Value::Null), doc, "to")?;
            let route = match props.get("route").and_then(Value::as_str) {
                None | Some("straight") => Route::Straight,
                Some("elbow") => Route::Elbow,
                Some("curved") => Route::Curved,
                Some(other) => {
                    return Err(format!("no route \"{other}\": straight, elbow or curved"));
                }
            };
            let arrow = props.get("arrow").and_then(Value::as_bool).unwrap_or(true);
            (
                ObjectKind::Connector(Connector {
                    start: from,
                    end: to,
                    route,
                    start_cap: Cap::None,
                    end_cap: if arrow { Cap::Arrow } else { Cap::None },
                    label: label(&body, 0.5, measure),
                    bends: Arc::from([]),
                }),
                Rect::from_points(from.point(), to.point()),
                Style::default(),
            )
        }
        "table" => {
            let (table, size) = new_table(props, doc, measure)?;
            let mut style = Table::style();
            style.font_size = Some(Table::FONT_SIZE);
            (ObjectKind::Table(table), place(size)?, style)
        }
        other => {
            return Err(format!(
                "cannot create \"{other}\": sticky, shape, text, frame, connector or table"
            ));
        }
    };
    let rotation = number(props, "rotation")?.unwrap_or(0.0).to_radians();
    let change = style_change(props)?;
    Ok(ObjectData {
        kind,
        bounds,
        rotation,
        style: change.apply_to(style),
        tags: match props.get("tags") {
            None | Some(Value::Null) => Default::default(),
            Some(names) => tags.ids(doc, names)?.into(),
        },
        link: parse_link(props.get("link").unwrap_or(&Value::Null), doc)?,
    })
}

/// A connector's label with `text`, measured, `at` along its line.
fn label(text: &str, at: f64, measure: &mut Measure) -> Label {
    if text.is_empty() {
        return Label::empty(at);
    }
    let (width, height) = measure(text, Label::FONT_SIZE, None);
    Label {
        text: text.into(),
        at,
        size: Size::new(width, height),
    }
}

/// A table from a plugin's `rows` (lists of cell texts), and the size its cells need:
/// `columnWidth` and `rowHeight` if given, else fitted to the text like a pasted table.
fn new_table(
    props: &Map<String, Value>,
    doc: &Document,
    measure: &mut Measure,
) -> Result<(Table, Size), String> {
    use crate::app::{MAX_TABLE_COLS, MAX_TABLE_ROWS, TABLE_CELL_PAD, TABLE_MAX_COLUMN};
    let wrong = "\"rows\" should be a list of rows, each a list of cell texts";
    let rows: Vec<Vec<String>> = props
        .get("rows")
        .and_then(Value::as_array)
        .ok_or(wrong)?
        .iter()
        .map(|row| {
            let cells = row.as_array().ok_or(wrong)?;
            Ok(cells.iter().map(cell_text).collect())
        })
        .collect::<Result<_, String>>()?;
    let row_count = rows.len().max(1);
    let col_count = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    if row_count > MAX_TABLE_ROWS || col_count > MAX_TABLE_COLS {
        return Err(format!(
            "a table can have up to {MAX_TABLE_ROWS} rows and {MAX_TABLE_COLS} columns"
        ));
    }
    let size = Table::FONT_SIZE;
    let pad = 2.0 * TABLE_CELL_PAD;
    let (min_w, min_h) = Table::CELL_SIZE;
    let text = |row: usize, col: usize| {
        rows.get(row)
            .and_then(|r| r.get(col))
            .map_or("", String::as_str)
    };
    let widths: Vec<f64> = match number(props, "columnWidth")? {
        Some(width) => vec![width.max(1.0); col_count],
        None => (0..col_count)
            .map(|col| {
                let widest = (0..row_count)
                    .map(|row| measure(text(row, col), size, None).0)
                    .fold(0.0, f64::max);
                (widest + pad).clamp(min_w * 0.6, TABLE_MAX_COLUMN)
            })
            .collect(),
    };
    let heights: Vec<f64> = match number(props, "rowHeight")? {
        Some(height) => vec![height.max(1.0); row_count],
        None => (0..row_count)
            .map(|row| {
                (0..col_count)
                    .filter(|&col| !text(row, col).is_empty())
                    .map(|col| measure(text(row, col), size, Some(widths[col] - pad)).1 + pad)
                    .fold(min_h, f64::max)
            })
            .collect(),
    };
    let mut table = Table::new(row_count, col_count, || doc.new_line_id());
    let mut cells = pb_core::table::Cells::new();
    for (r, line) in table.rows.iter().enumerate() {
        for (c, col) in table.cols.iter().enumerate() {
            let text = text(r, c);
            if !text.is_empty() {
                let cell = Cell {
                    text: text.into(),
                    spans: Default::default(),
                };
                cells.insert((line.id.clone(), col.id.clone()), cell);
            }
        }
    }
    let weigh = |lines: &[pb_core::Line], sizes: &[f64]| -> Arc<[pb_core::Line]> {
        lines
            .iter()
            .zip(sizes)
            .map(|(line, &weight)| pb_core::Line {
                weight,
                ..line.clone()
            })
            .collect()
    };
    table.rows = weigh(&table.rows, &heights);
    table.cols = weigh(&table.cols, &widths);
    table.cells = Arc::new(cells);
    let size = Size::new(widths.iter().sum(), heights.iter().sum());
    Ok((table, size))
}

/// A cell's text from a plugin: strings as they are, other values written out.
fn cell_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

/// The command putting `text` in cell (`row`, `col`) of table `id`.
pub fn cell_command(
    doc: &Document,
    id: ObjectId,
    row: &Value,
    col: &Value,
    text: &Value,
) -> Result<Command, String> {
    let object = doc.get(id).ok_or_else(|| format!("no object {id}"))?;
    if doc.is_locked(id) {
        return Err(format!("object {id} is locked"));
    }
    let ObjectKind::Table(table) = &object.data.kind else {
        return Err(format!("object {id} is not a table"));
    };
    let line = |value: &Value, lines: &[pb_core::Line], what: &str| {
        value
            .as_u64()
            .and_then(|i| lines.get(usize::try_from(i).ok()?))
            .map(|l| l.id.clone())
            .ok_or_else(|| format!("no {what} {value}: the table has {}", lines.len()))
    };
    Ok(Command::SetCell {
        id,
        row: line(row, &table.rows, "row")?,
        col: line(col, &table.cols, "column")?,
        cell: Cell {
            text: cell_text(text).into(),
            spans: Default::default(),
        },
    })
}

fn style_change(props: &Map<String, Value>) -> Result<StyleChange, String> {
    Ok(StyleChange {
        fill: color(props, "fill")?,
        stroke: color(props, "stroke")?,
        stroke_width: number(props, "strokeWidth")?,
        opacity: number(props, "opacity")?,
        ..StyleChange::default()
    })
}

/// The commands that give object `id` the properties in `props`.
pub fn update_commands(
    doc: &Document,
    id: ObjectId,
    props: &Value,
    measure: &mut Measure,
) -> Result<Vec<Command>, String> {
    let props = props.as_object().ok_or("the changes should be an object")?;
    let object = doc.get(id).ok_or_else(|| format!("no object {id}"))?;
    if doc.is_locked(id) {
        return Err(format!("object {id} is locked"));
    }
    let data = &object.data;
    let mut commands = Vec::new();
    let b = data.bounds;
    let geometry = ["x", "y", "width", "height", "rotation"]
        .iter()
        .any(|k| props.contains_key(*k));
    let mut bounds = b;
    if geometry {
        let x = number(props, "x")?.unwrap_or(b.x0);
        let y = number(props, "y")?.unwrap_or(b.y0);
        let width = number(props, "width")?.unwrap_or(b.width()).max(1.0);
        let height = number(props, "height")?.unwrap_or(b.height()).max(1.0);
        bounds = Rect::new(x, y, x + width, y + height);
        let rotation = number(props, "rotation")?.map_or(data.rotation, f64::to_radians);
        if !matches!(data.kind, ObjectKind::Connector(_)) {
            commands.push(Command::SetGeometry(vec![(
                id,
                pb_geom::Frame::new(bounds, rotation),
            )]));
        }
    }
    let new_text = text(props, "text")?.or(text(props, "title")?);
    if let Some(new_text) = new_text {
        match &data.kind {
            ObjectKind::Connector(c) => {
                commands.push(Command::SetLabel {
                    id,
                    label: label(&new_text, c.label.at, measure),
                });
            }
            ObjectKind::Sticky(_) | ObjectKind::Shape(_) | ObjectKind::Frame(_) => {
                commands.push(Command::SetText {
                    id,
                    text: new_text.into(),
                    spans: Default::default(),
                    bounds,
                });
            }
            ObjectKind::Text(t) => {
                let wrap = props.contains_key("width").then_some(bounds.width());
                let (w, h) = measure(&new_text, t.size, wrap);
                let width = if wrap.is_some() {
                    bounds.width()
                } else {
                    w.max(1.0)
                };
                commands.push(Command::SetText {
                    id,
                    text: new_text.into(),
                    spans: Default::default(),
                    bounds: Rect::from_origin_size(bounds.origin(), (width, h.max(1.0))),
                });
            }
            _ => return Err(format!("a {} has no text to set", type_name(&data.kind))),
        }
    }
    let change = style_change(props)?;
    if change != StyleChange::default() {
        commands.push(Command::SetStyle {
            ids: vec![id],
            change,
        });
    }
    if let Some(names) = props.get("tags") {
        let mut tags = TagNames::default();
        let wanted = match names {
            Value::Null => Vec::new(),
            names => tags.ids(doc, names)?,
        };
        let add: Vec<_> = wanted
            .iter()
            .filter(|t| !data.tags.contains(t))
            .cloned()
            .collect();
        let remove: Vec<_> = data
            .tags
            .iter()
            .filter(|t| !wanted.contains(t))
            .cloned()
            .collect();
        // New tags are defined before they are put on.
        let mut define = tags.commands();
        define.append(&mut commands);
        commands = define;
        if !add.is_empty() || !remove.is_empty() {
            commands.push(Command::SetTags {
                ids: vec![id],
                add,
                remove,
            });
        }
    }
    if let Some(link) = props.get("link") {
        let link = parse_link(link, doc)?;
        if link != data.link {
            commands.push(Command::SetLink {
                ids: vec![id],
                link,
            });
        }
    }
    if commands.is_empty() && !props.is_empty() {
        let known = [
            "x",
            "y",
            "width",
            "height",
            "rotation",
            "text",
            "title",
            "fill",
            "stroke",
            "strokeWidth",
            "opacity",
            "tags",
            "link",
        ];
        let unknown: Vec<_> = props
            .keys()
            .filter(|k| !known.contains(&k.as_str()))
            .collect();
        if !unknown.is_empty() {
            return Err(format!("cannot change {unknown:?}"));
        }
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(text: &str, size: f64, wrap: Option<f64>) -> (f64, f64) {
        let width = text.len() as f64 * size * 0.5;
        (wrap.unwrap_or(width), size * 1.2)
    }

    fn create(doc: &mut Document, item: Value) -> ObjectId {
        let mut tags = TagNames::default();
        let data = new_object(&item, doc, Point::new(0.0, 0.0), &mut measure, &mut tags).unwrap();
        for command in tags.commands() {
            doc.apply(command).unwrap();
        }
        doc.apply(Command::Create(vec![data.into()])).unwrap();
        doc.objects().last().unwrap().id
    }

    fn update(doc: &mut Document, id: ObjectId, props: Value) {
        for command in update_commands(doc, id, &props, &mut measure).unwrap() {
            doc.apply(command).unwrap();
        }
    }

    #[test]
    fn plugins_make_and_fill_tables() {
        let mut doc = Document::new();
        let table = create(
            &mut doc,
            json!({"type": "table", "rows": [["Name", "Age"], ["Ann", 31], ["Bob"]], "x": 0, "y": 0}),
        );
        let o = object_json(&doc, doc.get(table).unwrap());
        assert_eq!(o["type"], "table");
        assert_eq!(
            o["cells"],
            json!([["Name", "Age"], ["Ann", "31"], ["Bob", ""]])
        );
        // Fitted to the text, at least as big as a table's usual cells.
        let (min_w, min_h) = Table::CELL_SIZE;
        assert!(o["width"].as_f64().unwrap() >= 2.0 * min_w * 0.6);
        assert_eq!(o["height"].as_f64(), Some(3.0 * min_h));

        let command = cell_command(&doc, table, &json!(2), &json!(1), &json!("40")).unwrap();
        doc.apply(command).unwrap();
        let o = object_json(&doc, doc.get(table).unwrap());
        assert_eq!(o["cells"][2], json!(["Bob", "40"]));
        assert!(cell_command(&doc, table, &json!(3), &json!(0), &json!("x")).is_err());
        assert!(cell_command(&doc, table, &json!(-1), &json!(0), &json!("x")).is_err());

        let fixed = create(
            &mut doc,
            json!({"type": "table", "rows": [["a", "b", "c"]], "columnWidth": 50, "rowHeight": 30}),
        );
        let b = doc.get(fixed).unwrap().data.bounds;
        assert_eq!((b.width(), b.height()), (150.0, 30.0));
        assert!(cell_command(&doc, fixed, &json!(0), &json!(0), &json!("x")).is_ok());
        let note = create(&mut doc, json!({"type": "sticky"}));
        assert!(cell_command(&doc, note, &json!(0), &json!(0), &json!("x")).is_err());
        for bad in [
            json!({"type": "table"}),
            json!({"type": "table", "rows": ["a", "b"]}),
            json!({"type": "table", "rows": [vec!["x"; 51]]}),
        ] {
            assert!(
                new_object(
                    &bad,
                    &doc,
                    Point::ZERO,
                    &mut measure,
                    &mut Default::default()
                )
                .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn plugins_tag_link_and_label() {
        let mut doc = Document::new();
        let note = create(
            &mut doc,
            json!({"type": "sticky", "tags": ["Idea", "idea", "Urgent"]}),
        );
        // Names are matched ignoring case: two tags, made once.
        assert_eq!(doc.tags().len(), 2);
        let o = object_json(&doc, doc.get(note).unwrap());
        assert_eq!(o["tags"], json!(["Idea", "Urgent"]));
        let other = create(
            &mut doc,
            json!({"type": "sticky", "tags": ["URGENT"], "link": "example.com/a"}),
        );
        assert_eq!(doc.tags().len(), 2);
        let o = object_json(&doc, doc.get(other).unwrap());
        assert_eq!(o["tags"], json!(["Urgent"]));
        assert_eq!(o["link"], "https://example.com/a");

        update(
            &mut doc,
            note,
            json!({"tags": ["Urgent", "Later"], "link": other.to_string()}),
        );
        let o = object_json(&doc, doc.get(note).unwrap());
        assert_eq!(o["tags"], json!(["Urgent", "Later"]));
        assert_eq!(o["link"], other.to_string());
        assert_eq!(doc.tags().len(), 3);
        update(&mut doc, note, json!({"tags": [], "link": null}));
        let o = object_json(&doc, doc.get(note).unwrap());
        assert_eq!(
            (o["tags"].clone(), o["link"].clone()),
            (json!([]), Value::Null)
        );
        for bad in [
            json!({"link": "not a link"}),
            json!({"link": "9@9"}),
            json!({"tags": "Idea"}),
        ] {
            assert!(
                update_commands(&doc, note, &bad, &mut measure).is_err(),
                "{bad}"
            );
        }

        let line = create(
            &mut doc,
            json!({"type": "connector", "from": note.to_string(), "to": other.to_string(), "text": "yes"}),
        );
        let label = |doc: &Document| match &doc.get(line).unwrap().data.kind {
            ObjectKind::Connector(c) => c.label.clone(),
            _ => unreachable!(),
        };
        assert_eq!(&*label(&doc).text, "yes");
        assert_eq!(label(&doc).size, Size::new(24.0, 19.2));
        update(&mut doc, line, json!({"text": "no way"}));
        assert_eq!(object_json(&doc, doc.get(line).unwrap())["text"], "no way");
        assert_eq!(label(&doc).size, Size::new(48.0, 19.2));
    }

    #[test]
    fn colors_both_ways() {
        assert_eq!(parse_color("#ff8800").unwrap(), Rgba::rgb(0xFF, 0x88, 0));
        assert_eq!(color_text(Rgba::rgb(0xFF, 0x88, 0)), "#ff8800");
        assert_eq!(color_text(parse_color("11223344").unwrap()), "#11223344");
        assert!(parse_color("red").is_err() && parse_color("#fff").is_err());
    }

    #[test]
    fn plugins_create_read_and_change_objects() {
        let mut doc = Document::new();
        let note = create(
            &mut doc,
            json!({"type": "sticky", "text": "Hi", "x": 10, "y": 20, "fill": "#a0e0ff"}),
        );
        let shape = create(
            &mut doc,
            json!({"type": "shape", "shape": "ellipse", "width": 100, "height": 50}),
        );
        let label = create(
            &mut doc,
            json!({"type": "text", "text": "abcd", "fontSize": 10}),
        );
        let line = create(
            &mut doc,
            json!({"type": "connector", "from": note.to_string(), "to": {"x": 500, "y": 0}}),
        );
        let seen = objects_json(&doc, &json!({})).unwrap();
        let seen = seen.as_array().unwrap();
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[0]["type"], "sticky");
        assert_eq!(
            (seen[0]["x"].as_f64(), seen[0]["width"].as_f64()),
            (Some(10.0), Some(200.0))
        );
        assert_eq!(seen[0]["fill"], "#a0e0ff");
        assert_eq!(seen[1]["shape"], "ellipse");
        assert_eq!(seen[1]["x"].as_f64(), Some(-50.0));
        assert_eq!(seen[2]["width"].as_f64(), Some(20.0));
        assert_eq!(seen[3]["from"], note.to_string());
        assert_eq!(seen[3]["to"], Value::Null);
        let only = objects_json(&doc, &json!({"type": "shape"})).unwrap();
        assert_eq!(only.as_array().unwrap().len(), 1);

        for command in update_commands(
            &doc,
            note,
            &json!({"text": "1. Hi", "x": 0, "rotation": 90, "stroke": "#000000"}),
            &mut measure,
        )
        .unwrap()
        {
            doc.apply(command).unwrap();
        }
        let o = object_json(&doc, doc.get(note).unwrap());
        assert_eq!(
            (o["text"].as_str(), o["x"].as_f64()),
            (Some("1. Hi"), Some(0.0))
        );
        assert!((o["rotation"].as_f64().unwrap() - 90.0).abs() < 1e-9);
        // Text objects are fitted to their new text.
        for command in
            update_commands(&doc, label, &json!({"text": "abcdefgh"}), &mut measure).unwrap()
        {
            doc.apply(command).unwrap();
        }
        assert_eq!(doc.get(label).unwrap().data.bounds.width(), 40.0);

        assert!(update_commands(&doc, shape, &json!({"fill": "blue"}), &mut measure).is_err());
        // Connectors' text is their label; tables have none of their own.
        assert!(update_commands(&doc, line, &json!({"title": "x"}), &mut measure).is_ok());
        let table = create(&mut doc, json!({"type": "table", "rows": [["a"]]}));
        assert!(update_commands(&doc, table, &json!({"text": "x"}), &mut measure).is_err());
        assert!(update_commands(&doc, shape, &json!({"colour": "#000000"}), &mut measure).is_err());
        for bad in [
            json!({"type": "shape", "shape": "blob"}),
            json!({"type": "image"}),
            json!({"type": "connector", "from": "9@9", "to": {"x": 0, "y": 0}}),
            json!({"type": "sticky", "x": "left"}),
        ] {
            assert!(
                new_object(
                    &bad,
                    &doc,
                    Point::ZERO,
                    &mut measure,
                    &mut Default::default()
                )
                .is_err(),
                "{bad}"
            );
        }
    }
}
