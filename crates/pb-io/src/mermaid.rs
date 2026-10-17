//! Mermaid flowcharts: text like `flowchart LR; A[Start] --> B{OK?}` turned into shapes and
//! the lines between them, laid out in layers.
//!
//! Supported: `flowchart` / `graph` with a direction (TD, TB, BT, LR, RL); nodes `A[box]`,
//! `A(rounded)`, `A([stadium])`, `A[(database)]`, `A((circle))`, `A{decision}`,
//! `A{{hexagon}}`, `A[/data/]`, `A[\data\]`, `A>flag]`, with optional quotes; edges `-->`,
//! `---`, `-.->`, `==>` (and longer), labelled `-->|text|` or `-- text -->`; chains and
//! `&`; `;` between statements; `%%` comments. Styling, classes, clicks and subgraphs are
//! skipped.

use std::fmt;

use pb_core::ShapeKind;
use pb_geom::kurbo::{Point, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Down,
    Up,
    Right,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeShape {
    Box,
    Rounded,
    Stadium,
    Database,
    Circle,
    Decision,
    Hexagon,
    Data,
    Flag,
}

impl NodeShape {
    /// The board shape drawn for it.
    pub fn kind(self) -> ShapeKind {
        match self {
            Self::Box => ShapeKind::Rectangle,
            Self::Rounded => ShapeKind::RoundedRectangle,
            Self::Stadium => ShapeKind::Terminator,
            Self::Database => ShapeKind::Cylinder,
            Self::Circle => ShapeKind::Ellipse,
            Self::Decision => ShapeKind::Diamond,
            Self::Hexagon => ShapeKind::Hexagon,
            Self::Data => ShapeKind::Parallelogram,
            Self::Flag => ShapeKind::OffPage,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub text: String,
    pub shape: NodeShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    Solid,
    Dotted,
    Thick,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// Indices into [`Flowchart::nodes`].
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    /// It ends in an arrowhead.
    pub arrow: bool,
    pub line: LineStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Flowchart {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// What is wrong with the text, and on which line (from 1).
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Reads a Mermaid flowchart.
pub fn parse(source: &str) -> Result<Flowchart, ParseError> {
    let mut chart = Flowchart {
        direction: Direction::Down,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut header = false;
    for (n, raw) in source.lines().enumerate() {
        let line = n + 1;
        let text = raw.split("%%").next().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        for statement in split_statements(text) {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            if !header {
                chart.direction = parse_header(statement).ok_or_else(|| ParseError {
                    line,
                    message: "start with `flowchart TD` or `flowchart LR`".into(),
                })?;
                header = true;
                continue;
            }
            let first = statement.split_whitespace().next().unwrap_or("");
            if matches!(
                first,
                "subgraph"
                    | "end"
                    | "classDef"
                    | "class"
                    | "style"
                    | "linkStyle"
                    | "click"
                    | "direction"
            ) {
                continue;
            }
            Parser {
                chars: statement.chars().collect(),
                at: 0,
                line,
                chart: &mut chart,
            }
            .statement()?;
        }
    }
    if !header {
        return Err(ParseError {
            line: 1,
            message: "start with `flowchart TD` or `flowchart LR`".into(),
        });
    }
    if chart.nodes.is_empty() {
        return Err(ParseError {
            line: source.lines().count().max(1),
            message: "no nodes yet: add a line like `A[Start] --> B[End]`".into(),
        });
    }
    Ok(chart)
}

/// `flowchart TD`, `graph LR` and so on.
fn parse_header(text: &str) -> Option<Direction> {
    let mut words = text.split_whitespace();
    if !matches!(words.next()?, "flowchart" | "graph") {
        return None;
    }
    Some(match words.next() {
        None | Some("TD" | "TB") => Direction::Down,
        Some("BT") => Direction::Up,
        Some("LR") => Direction::Right,
        Some("RL") => Direction::Left,
        Some(_) => return None,
    })
}

/// Splits at `;` outside brackets and quotes.
fn split_statements(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quoted, mut start) = (0i32, false, 0);
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            ';' if !quoted && depth <= 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    line: usize,
    chart: &'a mut Flowchart,
}

impl Parser<'_> {
    fn error<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            line: self.line,
            message: message.into(),
        })
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn rest(&self) -> String {
        self.chars[self.at..].iter().collect()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    /// Nodes joined by edges: `A & B --> C -- label --> D`.
    fn statement(&mut self) -> Result<(), ParseError> {
        let mut from = self.group()?;
        loop {
            self.skip_spaces();
            if self.peek().is_none() {
                return Ok(());
            }
            let (label, arrow, line) = self.edge()?;
            let to = self.group()?;
            for &a in &from {
                for &b in &to {
                    self.chart.edges.push(Edge {
                        from: a,
                        to: b,
                        label: label.clone(),
                        arrow,
                        line,
                    });
                }
            }
            from = to;
        }
    }

    fn group(&mut self) -> Result<Vec<usize>, ParseError> {
        let mut nodes = vec![self.node()?];
        loop {
            self.skip_spaces();
            if self.peek() != Some('&') {
                return Ok(nodes);
            }
            self.at += 1;
            nodes.push(self.node()?);
        }
    }

    /// A node reference, with its shape and text when given.
    fn node(&mut self) -> Result<usize, ParseError> {
        self.skip_spaces();
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.at += 1;
        }
        if self.at == start {
            return match self.peek() {
                Some(c) => self.error(format!("expected a node name, found `{c}`")),
                None => self.error("expected a node name at the end of the line"),
            };
        }
        let id: String = self.chars[start..self.at].iter().collect();
        // Longer openings first: `([` before `(`.
        const SHAPES: [(&str, &str, NodeShape); 10] = [
            ("([", "])", NodeShape::Stadium),
            ("[(", ")]", NodeShape::Database),
            ("((", "))", NodeShape::Circle),
            ("{{", "}}", NodeShape::Hexagon),
            ("[/", "/]", NodeShape::Data),
            ("[\\", "\\]", NodeShape::Data),
            ("[", "]", NodeShape::Box),
            ("(", ")", NodeShape::Rounded),
            ("{", "}", NodeShape::Decision),
            (">", "]", NodeShape::Flag),
        ];
        let rest = self.rest();
        let shape = SHAPES.iter().find(|(open, ..)| rest.starts_with(open));
        let mut text_shape = None;
        if let Some(&(open, close, shape)) = shape {
            let inner = &rest[open.len()..];
            let Some(end) = inner.find(close) else {
                return self.error(format!("missing `{close}` after `{id}{open}`"));
            };
            let text = inner[..end].trim();
            let text = text
                .strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .unwrap_or(text);
            text_shape = Some((text.replace("<br>", "\n").replace("<br/>", "\n"), shape));
            self.at += open.chars().count() + inner[..end].chars().count() + close.chars().count();
        }
        let nodes = &mut self.chart.nodes;
        let index = match nodes.iter().position(|n| n.id == id) {
            Some(i) => i,
            None => {
                nodes.push(Node {
                    text: id.clone(),
                    id,
                    shape: NodeShape::Box,
                });
                nodes.len() - 1
            }
        };
        if let Some((text, shape)) = text_shape {
            nodes[index].text = text;
            nodes[index].shape = shape;
        }
        Ok(index)
    }

    /// An edge: its label, whether it has an arrowhead, and its line.
    fn edge(&mut self) -> Result<(Option<String>, bool, LineStyle), ParseError> {
        self.skip_spaces();
        let start = self.at;
        while self.peek().is_some_and(|c| matches!(c, '-' | '=' | '.')) {
            self.at += 1;
        }
        let head: String = self.chars[start..self.at].iter().collect();
        if head.len() < 2 {
            return match self.peek().or(head.chars().next()) {
                Some(c) => self.error(format!("expected `-->` between nodes, found `{c}`")),
                None => self.error("expected `-->` between nodes"),
            };
        }
        let style = |s: &str| {
            if s.contains('.') {
                LineStyle::Dotted
            } else if s.contains('=') {
                LineStyle::Thick
            } else {
                LineStyle::Solid
            }
        };
        let mut arrow = false;
        if self.peek() == Some('>') {
            self.at += 1;
            arrow = true;
        } else if self.peek().is_some_and(char::is_whitespace)
            && matches!(&*head, "--" | "-." | "==")
        {
            // `-- text -->`: the label runs to the rest of the edge.
            let rest = self.rest();
            let ends = ["-->", "---", ".->", "-.-", "==>", "==="];
            let found = ends
                .iter()
                .filter_map(|e| rest.find(e).map(|i| (i, *e)))
                .min_by_key(|(i, _)| *i);
            let Some((i, end)) = found else {
                return self.error(format!("missing the end of the edge after `{head}`"));
            };
            let label = rest[..i].trim().to_owned();
            self.at += rest[..i].chars().count() + end.chars().count();
            // Longer ends, like `--->`.
            while self.peek().is_some_and(|c| matches!(c, '-' | '=' | '.')) {
                self.at += 1;
            }
            arrow = end.ends_with('>');
            if self.peek() == Some('>') {
                self.at += 1;
                arrow = true;
            }
            return Ok(((!label.is_empty()).then_some(label), arrow, style(&head)));
        }
        let mut label = None;
        self.skip_spaces();
        if self.peek() == Some('|') {
            let rest = self.rest();
            let Some(end) = rest[1..].find('|') else {
                return self.error("missing the closing `|` of the edge label");
            };
            let text = rest[1..1 + end].trim().to_owned();
            self.at += rest[..end + 2].chars().count();
            label = (!text.is_empty()).then_some(text);
        }
        Ok((label, arrow, style(&head)))
    }
}

/// Space between layers, and between nodes in a layer, in world units.
const LAYER_GAP: f64 = 70.0;
const NODE_GAP: f64 = 40.0;

/// Centers for the nodes of `chart` with sizes `sizes`, in layers along its direction:
/// every edge goes from one layer to a later one (except those closing loops), and nodes
/// in a layer are ordered to keep lines short. The first layer is centered on the origin
/// across the direction.
pub fn layout(chart: &Flowchart, sizes: &[Size]) -> Vec<Point> {
    let n = chart.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let edges = acyclic_edges(n, &chart.edges);
    // Longest path from the sources.
    let mut layer = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for &(a, b) in &edges {
            if layer[b] < layer[a] + 1 {
                layer[b] = layer[a] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let layers = layer.iter().max().map_or(0, |m| m + 1);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for v in 0..n {
        rows[layer[v]].push(v);
    }
    // Order each layer by the average place of its neighbors, sweeping down and up.
    let mut place = vec![0.0; n];
    let set_places = |rows: &Vec<Vec<usize>>, place: &mut Vec<f64>| {
        for row in rows {
            for (i, &v) in row.iter().enumerate() {
                place[v] = i as f64;
            }
        }
    };
    set_places(&rows, &mut place);
    for sweep in 0..8 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> = if down {
            (1..layers).collect()
        } else {
            (0..layers.saturating_sub(1)).rev().collect()
        };
        for l in order {
            let mut keyed: Vec<(f64, usize)> = rows[l]
                .iter()
                .map(|&v| {
                    let neighbors: Vec<f64> = edges
                        .iter()
                        .filter_map(|&(a, b)| match down {
                            true if b == v => Some(place[a]),
                            false if a == v => Some(place[b]),
                            _ => None,
                        })
                        .collect();
                    let key = if neighbors.is_empty() {
                        place[v]
                    } else {
                        neighbors.iter().sum::<f64>() / neighbors.len() as f64
                    };
                    (key, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(place[a.1].total_cmp(&place[b.1])));
            rows[l] = keyed.into_iter().map(|(_, v)| v).collect();
            set_places(&rows, &mut place);
        }
    }
    // Along the direction: layers one after another; across it: each layer centered.
    let across_down = matches!(chart.direction, Direction::Down | Direction::Up);
    let along = |s: Size| if across_down { s.height } else { s.width };
    let across = |s: Size| if across_down { s.width } else { s.height };
    let mut centers = vec![Point::ZERO; n];
    let mut offset = 0.0;
    for row in &rows {
        let depth = row.iter().map(|&v| along(sizes[v])).fold(0.0, f64::max);
        let total: f64 = row.iter().map(|&v| across(sizes[v])).sum::<f64>()
            + NODE_GAP * row.len().saturating_sub(1) as f64;
        let mut x = -total / 2.0;
        for &v in row {
            let w = across(sizes[v]);
            let (a, c) = (offset + depth / 2.0, x + w / 2.0);
            centers[v] = match chart.direction {
                Direction::Down => Point::new(c, a),
                Direction::Up => Point::new(c, -a),
                Direction::Right => Point::new(a, c),
                Direction::Left => Point::new(-a, c),
            };
            x += w + NODE_GAP;
        }
        offset += depth + LAYER_GAP;
    }
    centers
}

/// The edges as pairs, with those closing a loop turned around and self loops dropped,
/// so that the result has no cycles.
fn acyclic_edges(n: usize, edges: &[Edge]) -> Vec<(usize, usize)> {
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        if e.from != e.to {
            out_edges[e.from].push(e.to);
        }
    }
    // 0 unseen, 1 on the current path, 2 done.
    let mut state = vec![0u8; n];
    let mut result = Vec::new();
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        state[root] = 1;
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if let Some(&w) = out_edges[v].get(*next) {
                *next += 1;
                match state[w] {
                    0 => {
                        result.push((v, w));
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => result.push((w, v)),
                    _ => result.push((v, w)),
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nodes_edges_and_labels() {
        let chart = parse(
            "flowchart LR\n\
             %% a comment\n\
             A([Start]) --> B{Is it OK?}\n\
             B -->|yes| C[Ship it]; B -- no --> D[(Fix)]\n\
             D -.-> B\n\
             C & D ==> E((Done))\n",
        )
        .unwrap();
        assert_eq!(chart.direction, Direction::Right);
        let names: Vec<_> = chart.nodes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(names, ["Start", "Is it OK?", "Ship it", "Fix", "Done"]);
        let shapes: Vec<_> = chart.nodes.iter().map(|n| n.shape).collect();
        assert_eq!(
            shapes,
            [
                NodeShape::Stadium,
                NodeShape::Decision,
                NodeShape::Box,
                NodeShape::Database,
                NodeShape::Circle
            ]
        );
        assert_eq!(chart.edges.len(), 6);
        assert_eq!(chart.edges[1].label.as_deref(), Some("yes"));
        assert_eq!(chart.edges[2].label.as_deref(), Some("no"));
        assert!(chart.edges[2].arrow);
        assert_eq!(chart.edges[3].line, LineStyle::Dotted);
        assert_eq!(chart.edges[4].line, LineStyle::Thick);
        assert_eq!((chart.edges[4].from, chart.edges[5].from), (2, 3));
    }

    #[test]
    fn plain_names_quotes_and_lines_without_arrows() {
        let chart = parse("graph TD\n  a --- b\n  b --> c[\"Say <br>hi\"]\n  a\n").unwrap();
        assert_eq!(chart.nodes[0].text, "a");
        assert_eq!(chart.nodes[2].text, "Say \nhi");
        assert!(!chart.edges[0].arrow);
        assert!(chart.edges[1].arrow);
    }

    #[test]
    fn errors_name_the_line() {
        let err = parse("A --> B").unwrap_err();
        assert_eq!(err.line, 1);
        let err = parse("flowchart TD\nA --> B\nB --> C[oops\n").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(err.message.contains(']'), "{}", err.message);
        let err = parse("flowchart TD\nA -> B\n").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(parse("flowchart TD\n").is_err());
    }

    #[test]
    fn layers_follow_the_edges_and_survive_loops() {
        let chart = parse("flowchart TD\nA --> B\nA --> C\nB --> D\nC --> D\nD --> A\n").unwrap();
        let sizes = vec![Size::new(100.0, 50.0); chart.nodes.len()];
        let at = layout(&chart, &sizes);
        // A on top, B and C side by side below it, D under them.
        assert!(at[0].y < at[1].y && at[1].y == at[2].y && at[2].y < at[3].y);
        assert!((at[1].x - at[2].x).abs() >= 100.0 + NODE_GAP - 1e-9);
        assert!((at[0].x).abs() < 1e-9);
        // Left to right puts layers along x.
        let lr = Flowchart {
            direction: Direction::Right,
            ..chart
        };
        let at = layout(&lr, &sizes);
        assert!(at[0].x < at[1].x && at[1].x < at[3].x);
    }
}
