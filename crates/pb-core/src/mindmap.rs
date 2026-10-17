//! Mind maps: a tree of topics laid out automatically around the central one.
//!
//! Nodes have stable ids; each names its parent and has a fractional `order` among its
//! siblings, so replicas adding, moving or typing in different nodes merge. Text is
//! measured by the editor and stored with the node, so the layout here needs no fonts.

use std::sync::Arc;

use kurbo::{Point, Rect, Size};

use crate::{HAlign, Spans, Style, TextBox, VAlign};

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Unique within its map (see [`crate::Document::new_line_id`]).
    pub id: Arc<str>,
    /// `None` for the central topic.
    pub parent: Option<Arc<str>>,
    /// Siblings go by `order`, then by id.
    pub order: f64,
    pub text: Arc<str>,
    pub spans: Spans,
    /// The size of the text as laid out at the node's font size, in map units.
    pub text_size: Size,
    /// Its children are hidden.
    pub collapsed: bool,
    /// A main topic (child of the central one) that branches out to the left.
    pub left: bool,
}

/// A node where the layout put it.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// Index into [`MindMap::nodes`].
    pub index: usize,
    /// Its box, in map units with the central topic's middle at the origin.
    pub rect: Rect,
    /// 0 for the central topic, 1 for main topics, and so on.
    pub depth: usize,
    /// Which main topic it belongs to, for its color; 0 for the central topic.
    pub branch: usize,
    /// It sits left of its parent.
    pub left: bool,
    /// How many nodes its collapsing hides.
    pub hidden: usize,
}

/// Where everything in a map goes, in map units around the central topic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// Shown nodes, parents before children.
    pub placed: Vec<Placed>,
    /// Lines from parent to child, as indices into `placed`.
    pub edges: Vec<(usize, usize)>,
    /// Around everything shown.
    pub bounds: Rect,
}

impl Layout {
    pub fn find(&self, index: usize) -> Option<&Placed> {
        self.placed.iter().find(|p| p.index == index)
    }
}

/// A part of a map a click can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MindMapPart {
    /// Node `i` of [`MindMap::nodes`].
    Node(usize),
    /// The collapse toggle of node `i`.
    Toggle(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MindMap {
    /// In no particular order; exactly one has no parent while the map is whole.
    pub nodes: Arc<[Node]>,
}

/// Font size of the central topic and of the others, in map units.
const ROOT_FONT: f64 = 22.0;
const FONT: f64 = 16.0;
/// Space between text and a node's edge.
const ROOT_PAD: Size = Size::new(18.0, 12.0);
const PAD: Size = Size::new(10.0, 6.0);
/// Space between a parent and its children, and between siblings.
const H_GAP: f64 = 48.0;
const V_GAP: f64 = 12.0;
/// Extra width given to text, see [`MindMap::box_size`].
const WRAP_SLACK: f64 = 1.0;
/// Radius of the collapse toggle beside a node with children.
const TOGGLE_RADIUS: f64 = 7.0;

impl MindMap {
    /// Lines of node text wrap at this width, in map units.
    pub const WRAP_WIDTH: f64 = 260.0;

    /// A map with just the central topic `text`, measured as `text_size`.
    pub fn new(id: impl Into<Arc<str>>, text: &str, text_size: Size) -> Self {
        Self {
            nodes: Arc::new([Node {
                id: id.into(),
                parent: None,
                order: 0.0,
                text: text.into(),
                spans: Spans::default(),
                text_size,
                collapsed: false,
                left: false,
            }]),
        }
    }

    /// The central topic: the node without a parent (the first by id if there are more).
    pub fn root(&self) -> Option<usize> {
        (0..self.nodes.len())
            .filter(|&i| self.nodes[i].parent.is_none())
            .min_by(|&a, &b| self.nodes[a].id.cmp(&self.nodes[b].id))
    }

    pub fn index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| &*n.id == id)
    }

    /// Children of node `i`, in order.
    pub fn children(&self, i: usize) -> Vec<usize> {
        let id = &self.nodes[i].id;
        let mut children: Vec<usize> = (0..self.nodes.len())
            .filter(|&c| self.nodes[c].parent.as_deref() == Some(&**id))
            .collect();
        children.sort_by(|&a, &b| {
            let (a, b) = (&self.nodes[a], &self.nodes[b]);
            a.order.total_cmp(&b.order).then_with(|| a.id.cmp(&b.id))
        });
        children
    }

    /// Node `i` and everything under it.
    pub fn subtree(&self, i: usize) -> Vec<usize> {
        let mut out = vec![i];
        let mut at = 0;
        while at < out.len() {
            let children = self.children(out[at]);
            out.extend(children);
            at += 1;
        }
        out
    }

    pub fn font_size(&self, i: usize) -> f64 {
        if self.nodes[i].parent.is_none() {
            ROOT_FONT
        } else {
            FONT
        }
    }

    fn pad(&self, i: usize) -> Size {
        if self.nodes[i].parent.is_none() {
            ROOT_PAD
        } else {
            PAD
        }
    }

    /// The size of node `i`'s box.
    fn box_size(&self, i: usize) -> Size {
        let n = &self.nodes[i];
        let pad = self.pad(i);
        let font = self.font_size(i);
        Size::new(
            // A hair wider than measured, so the text does not wrap again when drawn.
            n.text_size.width.max(font * 2.0) + WRAP_SLACK + 2.0 * pad.width,
            n.text_size.height.max(font * 1.2) + 2.0 * pad.height,
        )
    }

    /// The order for a new last child of node `i`.
    pub fn order_after_children(&self, i: usize) -> f64 {
        self.children(i)
            .last()
            .map_or(0.0, |&c| self.nodes[c].order + 1.0)
    }

    /// The order for a new sibling right after node `i`.
    pub fn order_after(&self, i: usize) -> f64 {
        let Some(parent) = self.nodes[i].parent.as_deref().and_then(|p| self.index(p)) else {
            return 0.0;
        };
        let siblings = self.children(parent);
        let at = siblings.iter().position(|&s| s == i).unwrap_or(0);
        let mine = self.nodes[i].order;
        match siblings.get(at + 1) {
            Some(&next) => (mine + self.nodes[next].order) / 2.0,
            None => mine + 1.0,
        }
    }

    /// Whether a new main topic goes left: to the side with fewer nodes.
    pub fn new_branch_goes_left(&self) -> bool {
        let Some(root) = self.root() else {
            return false;
        };
        let (mut left, mut right) = (0, 0);
        for c in self.children(root) {
            let n = self.subtree(c).len();
            if self.nodes[c].left {
                left += n;
            } else {
                right += n;
            }
        }
        left < right
    }

    /// Where a node added from node `i` goes: a child of it (`child`), or its next sibling;
    /// the parent's id, the order and the side. Siblings of the central topic are children
    /// of it, and new main topics go to the lighter side.
    pub fn new_node_place(&self, i: usize, child: bool) -> Option<(Arc<str>, f64, bool)> {
        let node = self.nodes.get(i)?;
        let parent = node.parent.as_deref().and_then(|p| self.index(p));
        match parent {
            Some(parent) if !child => Some((
                self.nodes[parent].id.clone(),
                self.order_after(i),
                node.left,
            )),
            _ => {
                let left = if node.parent.is_none() {
                    self.new_branch_goes_left()
                } else {
                    node.left
                };
                Some((node.id.clone(), self.order_after_children(i), left))
            }
        }
    }

    /// A copy with `node` in place of the node with its id, or added.
    pub fn with_node(&self, node: Node) -> Self {
        let mut nodes = self.nodes.to_vec();
        match nodes.iter_mut().find(|n| n.id == node.id) {
            Some(old) => *old = node,
            None => nodes.push(node),
        }
        Self {
            nodes: nodes.into(),
        }
    }

    /// A copy without the nodes with ids `gone`.
    pub fn without(&self, gone: &[Arc<str>]) -> Self {
        let nodes: Vec<Node> = self
            .nodes
            .iter()
            .filter(|n| !gone.contains(&n.id))
            .cloned()
            .collect();
        Self {
            nodes: nodes.into(),
        }
    }

    /// Lays the map out: main topics to the right and left of the central one, each
    /// subtree centered on its parent.
    pub fn layout(&self) -> Layout {
        let mut out = Layout::default();
        let Some(root) = self.root() else {
            return out;
        };
        let size = self.box_size(root);
        let rect = Rect::from_center_size(Point::ZERO, size);
        out.placed.push(Placed {
            index: root,
            rect,
            depth: 0,
            branch: 0,
            left: false,
            hidden: self.hidden(root),
        });
        if !self.nodes[root].collapsed {
            let children = self.children(root);
            for left in [false, true] {
                let side: Vec<usize> = children
                    .iter()
                    .copied()
                    .filter(|&c| self.nodes[c].left == left)
                    .collect();
                let total = self.block_height(&side);
                let mut top = -total / 2.0;
                let edge = if left {
                    rect.x0 - H_GAP
                } else {
                    rect.x1 + H_GAP
                };
                for c in side {
                    let branch = children.iter().position(|&x| x == c).unwrap_or(0) + 1;
                    let h = self.subtree_height(c);
                    self.place(&mut out, 0, c, edge, top, left, 1, branch);
                    top += h + V_GAP;
                }
            }
        }
        out.bounds = out
            .placed
            .iter()
            .map(|p| p.rect)
            .reduce(|a, b| a.union(b))
            .unwrap_or_default();
        // Room for collapse toggles at the outer sides.
        let r = 2.0 * TOGGLE_RADIUS + 2.0;
        out.bounds = out.bounds.inflate(r, 0.0);
        out
    }

    /// Places node `i` and its subtree with its block's top at `top`, growing away from
    /// the parent from x `edge`; `parent` is the parent's index in `out.placed`.
    #[allow(clippy::too_many_arguments)]
    fn place(
        &self,
        out: &mut Layout,
        parent: usize,
        i: usize,
        edge: f64,
        top: f64,
        left: bool,
        depth: usize,
        branch: usize,
    ) {
        let size = self.box_size(i);
        let h = self.subtree_height(i);
        let cy = top + h / 2.0;
        let rect = if left {
            Rect::new(
                edge - size.width,
                cy - size.height / 2.0,
                edge,
                cy + size.height / 2.0,
            )
        } else {
            Rect::new(
                edge,
                cy - size.height / 2.0,
                edge + size.width,
                cy + size.height / 2.0,
            )
        };
        let me = out.placed.len();
        out.placed.push(Placed {
            index: i,
            rect,
            depth,
            branch,
            left,
            hidden: self.hidden(i),
        });
        out.edges.push((parent, me));
        if self.nodes[i].collapsed {
            return;
        }
        let children = self.children(i);
        let total = self.block_height(&children);
        let mut top = cy - total / 2.0;
        let edge = if left {
            rect.x0 - H_GAP
        } else {
            rect.x1 + H_GAP
        };
        for c in children {
            let h = self.subtree_height(c);
            self.place(out, me, c, edge, top, left, depth + 1, branch);
            top += h + V_GAP;
        }
    }

    /// Height of node `i` with everything shown under it.
    fn subtree_height(&self, i: usize) -> f64 {
        let own = self.box_size(i).height;
        if self.nodes[i].collapsed {
            return own;
        }
        own.max(self.block_height(&self.children(i)))
    }

    /// Height of `nodes`' subtrees stacked with gaps.
    fn block_height(&self, nodes: &[usize]) -> f64 {
        let sum: f64 = nodes.iter().map(|&c| self.subtree_height(c)).sum();
        sum + V_GAP * nodes.len().saturating_sub(1) as f64
    }

    /// How many nodes collapsing node `i` hides (0 unless it is collapsed).
    fn hidden(&self, i: usize) -> usize {
        if self.nodes[i].collapsed {
            self.subtree(i).len() - 1
        } else {
            0
        }
    }

    /// How map units map onto the object's `bounds`: the scale, and where the map's origin
    /// (the central topic's middle) lands.
    pub fn transform(&self, layout: &Layout, bounds: Rect) -> (f64, Point) {
        let w = layout.bounds.width();
        let scale = if w > 0.0 { bounds.width() / w } else { 1.0 };
        let origin = bounds.origin() - layout.bounds.origin().to_vec2() * scale;
        (scale, origin)
    }

    /// `rect` in map units, in the unrotated world of an object with `bounds`.
    pub fn to_world(&self, layout: &Layout, bounds: Rect, rect: Rect) -> Rect {
        let (scale, origin) = self.transform(layout, bounds);
        Rect::from_points(
            origin + rect.origin().to_vec2() * scale,
            origin + Point::new(rect.x1, rect.y1).to_vec2() * scale,
        )
    }

    /// Bounds for this map that keep the central topic where it is in `old` with
    /// `old_bounds`, at the same scale.
    pub fn refit(&self, old: &MindMap, old_bounds: Rect) -> Rect {
        let old_layout = old.layout();
        let (scale, origin) = old.transform(&old_layout, old_bounds);
        let b = self.layout().bounds;
        Rect::new(
            origin.x + b.x0 * scale,
            origin.y + b.y0 * scale,
            origin.x + b.x1 * scale,
            origin.y + b.y1 * scale,
        )
    }

    /// Node `i`'s box in the unrotated world of an object with `bounds`, if shown.
    pub fn node_rect(&self, bounds: Rect, i: usize) -> Option<Rect> {
        let layout = self.layout();
        let rect = layout.find(i)?.rect;
        Some(self.to_world(&layout, bounds, rect))
    }

    /// Where node `i`'s collapse toggle sits (map units), if it has children.
    pub fn toggle_at(&self, placed: &Placed) -> Option<Point> {
        if self.children(placed.index).is_empty() || self.nodes[placed.index].parent.is_none() {
            return None;
        }
        let r = placed.rect;
        let x = if placed.left {
            r.x0 - TOGGLE_RADIUS - 2.0
        } else {
            r.x1 + TOGGLE_RADIUS + 2.0
        };
        Some(Point::new(x, r.center().y))
    }

    pub fn toggle_radius() -> f64 {
        TOGGLE_RADIUS
    }

    /// The part at `point`, in the unrotated world of an object with `bounds`.
    pub fn part_at(&self, bounds: Rect, point: Point) -> Option<MindMapPart> {
        let layout = self.layout();
        let (scale, origin) = self.transform(&layout, bounds);
        if scale <= 0.0 {
            return None;
        }
        let p = ((point - origin) / scale).to_point();
        for placed in layout.placed.iter().rev() {
            if let Some(t) = self.toggle_at(placed)
                && (t - p).hypot() <= TOGGLE_RADIUS + 2.0
            {
                return Some(MindMapPart::Toggle(placed.index));
            }
            if placed.rect.contains(p) {
                return Some(MindMapPart::Node(placed.index));
            }
        }
        None
    }

    /// Where node `i`'s text goes, in the unrotated world of an object with `bounds`.
    pub fn text_box(&self, bounds: Rect, i: usize) -> Option<TextBox> {
        let layout = self.layout();
        let placed = layout.find(i)?;
        Some(self.placed_text_box(&layout, bounds, placed))
    }

    /// Where the text of `placed` from `layout` goes, for an object with `bounds`.
    pub fn placed_text_box(&self, layout: &Layout, bounds: Rect, placed: &Placed) -> TextBox {
        let (scale, _) = self.transform(layout, bounds);
        let i = placed.index;
        let pad = self.pad(i);
        let rect = self.to_world(layout, bounds, placed.rect);
        TextBox {
            area: rect.inset((-pad.width * scale, -pad.height * scale)),
            max_size: self.font_size(i) * scale,
            align: HAlign::Center,
            valign: VAlign::Middle,
        }
    }

    /// Colors of the main topics' branches, in turn.
    pub const BRANCH_COLORS: [crate::Rgba; 6] = [
        crate::Rgba::rgb(0x3B, 0x82, 0xF6),
        crate::Rgba::rgb(0xE5, 0x48, 0x4D),
        crate::Rgba::rgb(0x22, 0xA0, 0x6B),
        crate::Rgba::rgb(0xF5, 0x9E, 0x0B),
        crate::Rgba::rgb(0x8B, 0x5C, 0xF6),
        crate::Rgba::rgb(0xEC, 0x48, 0x99),
    ];

    /// The color of branch `branch` (0 for the central topic: the outline color).
    pub fn branch_color(branch: usize, style: &Style) -> crate::Rgba {
        match branch {
            0 => style.stroke,
            b => Self::BRANCH_COLORS[(b - 1) % Self::BRANCH_COLORS.len()],
        }
    }

    /// Default look: a dark central topic with white text, white topics.
    pub fn style() -> Style {
        Style {
            fill: crate::Rgba::rgb(0x1F, 0x29, 0x37),
            stroke: crate::Rgba::rgb(0x1F, 0x29, 0x37),
            stroke_width: 2.0,
            text_color: Some(crate::Rgba::rgb(0x1F, 0x29, 0x37)),
            ..Style::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, parent: Option<&str>, order: f64, left: bool) -> Node {
        Node {
            id: id.into(),
            parent: parent.map(Into::into),
            order,
            text: id.into(),
            spans: Spans::default(),
            text_size: Size::new(40.0, 20.0),
            collapsed: false,
            left,
        }
    }

    fn map() -> MindMap {
        MindMap {
            nodes: vec![
                node("root", None, 0.0, false),
                node("b", Some("root"), 1.0, false),
                node("a", Some("root"), 0.0, false),
                node("a1", Some("a"), 0.0, false),
                node("a2", Some("a"), 1.0, false),
                node("l", Some("root"), 2.0, true),
            ]
            .into(),
        }
    }

    #[test]
    fn subtrees_hang_off_their_parents_side() {
        let m = map();
        let layout = m.layout();
        let at = |id: &str| layout.find(m.index(id).unwrap()).unwrap().clone();
        let root = at("root");
        assert_eq!(root.rect.center(), Point::ZERO);
        // Right side in order, a above b; a's children to its right, centered on it.
        let (a, b, a1, a2) = (at("a"), at("b"), at("a1"), at("a2"));
        assert!(a.rect.x0 > root.rect.x1 && b.rect.x0 == a.rect.x0);
        assert!(a.rect.y1 < b.rect.y0);
        assert!(a1.rect.x0 > a.rect.x1);
        let middle = (a1.rect.center().y + a2.rect.center().y) / 2.0;
        assert!((middle - a.rect.center().y).abs() < 1e-9);
        // The left topic is left of the root.
        let l = at("l");
        assert!(l.left && l.rect.x1 < root.rect.x0);
        assert_eq!(layout.edges.len(), 5);
        // Branch colors follow the main topics.
        assert_eq!(a1.branch, a.branch);
        assert_ne!(a.branch, b.branch);
    }

    #[test]
    fn collapsing_hides_a_subtree() {
        let m = map();
        let a = m.index("a").unwrap();
        let collapsed = m.with_node(Node {
            collapsed: true,
            ..m.nodes[a].clone()
        });
        let layout = collapsed.layout();
        assert_eq!(layout.placed.len(), 4);
        assert_eq!(layout.find(a).unwrap().hidden, 2);
        assert!(layout.find(m.index("a1").unwrap()).is_none());
        // The map gets shorter.
        assert!(layout.bounds.height() < m.layout().bounds.height());
    }

    #[test]
    fn the_root_stays_put_when_the_map_grows() {
        let m = map();
        let bounds = Rect::new(100.0, 100.0, 100.0 + m.layout().bounds.width(), 500.0);
        let root = m.root().unwrap();
        let before = m.node_rect(bounds, root).unwrap();
        let grown = m.with_node(node("c", Some("root"), 5.0, false));
        let grown_bounds = grown.refit(&m, bounds);
        let after = grown.node_rect(grown_bounds, root).unwrap();
        assert!((before.center() - after.center()).hypot() < 1e-6);
        assert!((before.width() - after.width()).abs() < 1e-6);
        // Hits land on nodes and toggles.
        let a = m.index("a").unwrap();
        let a_rect = m.node_rect(bounds, a).unwrap();
        assert_eq!(
            m.part_at(bounds, a_rect.center()),
            Some(MindMapPart::Node(a))
        );
        let layout = m.layout();
        let toggle = m.toggle_at(layout.find(a).unwrap()).unwrap();
        let toggle = m.to_world(&layout, bounds, Rect::from_center_size(toggle, (0.0, 0.0)));
        assert_eq!(
            m.part_at(bounds, toggle.center()),
            Some(MindMapPart::Toggle(a))
        );
    }

    #[test]
    fn new_nodes_go_after_and_to_the_lighter_side() {
        let m = map();
        let a = m.index("a").unwrap();
        assert_eq!(m.order_after(a), 0.5);
        assert_eq!(m.order_after_children(a), 2.0);
        assert_eq!(m.subtree(a).len(), 3);
        // Right has a, a1, a2, b; left has l.
        assert!(m.new_branch_goes_left());
        let gone: Vec<Arc<str>> = m
            .subtree(a)
            .iter()
            .map(|&i| m.nodes[i].id.clone())
            .collect();
        assert_eq!(m.without(&gone).nodes.len(), 3);
    }
}
