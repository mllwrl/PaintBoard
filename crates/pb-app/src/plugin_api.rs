//! The board as plugins see it: objects as JSON, and plugins' JSON turned into new
//! objects and commands. What each call does is in `plugins` (the calls) and in
//! `packaging/plugins/paintboard.d.ts` (for plugin authors); keep the three in step.
//!
//! An object is `{id, type, x, y, width, height, rotation, fill, stroke, strokeWidth,
//! opacity, text, tags, locked, parent}` plus, by type: `shape` (its kind) for shapes,
//! `fontSize` for text, `title` for frames, `from`/`to` (ids or null) for connectors.
//! Positions are the unrotated top left corner in board units; rotation is in degrees,
//! clockwise; colors are `#rrggbb` or `#rrggbbaa`.

use std::sync::Arc;

use pb_core::connector::{Anchor, Cap, Connector, End, Label, Route};
use pb_core::{
    Command, Document, FrameData, Object, ObjectData, ObjectId, ObjectKind, Rgba, Shape, ShapeKind,
    Sticky, Style, StyleChange, Text,
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
pub fn new_object(
    item: &Value,
    doc: &Document,
    center: Point,
    measure: &mut Measure,
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
                    label: Label::empty(0.5),
                    bends: Arc::from([]),
                }),
                Rect::from_points(from.point(), to.point()),
                Style::default(),
            )
        }
        other => {
            return Err(format!(
                "cannot create \"{other}\": sticky, shape, text, frame or connector"
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
        tags: Default::default(),
        link: None,
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
        let data = new_object(&item, doc, Point::new(0.0, 0.0), &mut measure).unwrap();
        doc.apply(Command::Create(vec![data.into()])).unwrap();
        doc.objects().last().unwrap().id
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
        assert!(update_commands(&doc, line, &json!({"title": "x"}), &mut measure).is_err());
        assert!(update_commands(&doc, shape, &json!({"colour": "#000000"}), &mut measure).is_err());
        for bad in [
            json!({"type": "shape", "shape": "blob"}),
            json!({"type": "image"}),
            json!({"type": "connector", "from": "9@9", "to": {"x": 0, "y": 0}}),
            json!({"type": "sticky", "x": "left"}),
        ] {
            assert!(
                new_object(&bad, &doc, Point::ZERO, &mut measure).is_err(),
                "{bad}"
            );
        }
    }
}
