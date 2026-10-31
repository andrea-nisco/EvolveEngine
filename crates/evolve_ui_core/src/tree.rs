//! The tree of nodes of one screen: styles in, rectangles and a draw list out (EVOLVE_UI_PLAN.md 4.3, 4.7).
//!
//! The layout is Taffy's (flexbox and grid); this module turns the plan's style into Taffy's, tells it how big a text or an image is,
//! and walks the result to make the draw list. Only what changed is laid out again: Taffy keeps what it worked out for every node
//! and a change dirties the node and its ancestors only. A tree that did not change is neither laid out nor drawn again; the draw
//! list it made is kept.

use crate::draw::{DrawCmd, DrawList, Fill, ImageId};
use crate::geometry::{Color, Point, Rect};
use crate::style::{Align, Dim, Direction, Display, Justify, Overflow, Position, Sides, Style, Track};
use crate::text::{TextAlign, TextStyle, Typesetter};
use crate::units::{Insets, ScreenMetrics};
use std::collections::HashMap;
use taffy as tf;
use taffy::style_helpers::{auto, fr, length, percent};

/// A node of a [`UiTree`]. Only meaningful for the tree that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(tf::NodeId);

#[derive(Clone, Debug)]
struct TextContent {
    text: String,
    style: TextStyle,
}

#[derive(Clone, Debug)]
struct ImageContent {
    image: ImageId,
    tint: Option<Color>,
    /// The drawing's own size, which gives a node its shape when nothing else does.
    size: (f32, f32),
}

#[derive(Clone, Debug)]
enum Content {
    Text(TextContent),
    Image(ImageContent),
}

/// What the tree has done, for tests and for the debug panel (EVOLVE_UI_PLAN.md section 7: the cost of the UI must be visible).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeStats {
    /// Times the layout was worked out (a tree that did not change does not).
    pub layouts: u64,
    /// Times a text was measured while laying out.
    pub text_measures: u64,
    /// Times the draw list was made.
    pub paints: u64,
}

/// What the layout was last made for.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ScreenKey {
    canvas: (u32, u32),
    scale: u32,
    safe_area: [u32; 4],
}

pub struct UiTree {
    taffy: tf::TaffyTree<Content>,
    styles: HashMap<tf::NodeId, Style>,
    root: tf::NodeId,
    laid_out_for: Option<ScreenKey>,
    list: Option<DrawList>,
    stats: TreeStats,
}

// --- Style conversion -------------------------------------------------------------------------------------------------------

fn dimension(dim: Dim) -> tf::Dimension {
    match dim {
        Dim::Px(v) if v.is_finite() => length(v.max(0.0)),
        Dim::Percent(v) if v.is_finite() => percent(v.max(0.0) / 100.0),
        _ => auto(),
    }
}

fn limit(value: Option<f32>) -> tf::LengthPercentageAuto {
    match value {
        Some(v) if v.is_finite() => length(v.max(0.0)),
        _ => auto(),
    }
}

fn non_negative(v: f32) -> f32 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}

fn margin(sides: Sides) -> tf::Rect<tf::LengthPercentageAuto> {
    tf::Rect { left: length(sides.left), right: length(sides.right), top: length(sides.top), bottom: length(sides.bottom) }
}

fn inset(sides: (Option<f32>, Option<f32>, Option<f32>, Option<f32>)) -> tf::Rect<tf::LengthPercentageAuto> {
    let edge = |v: Option<f32>| match v {
        Some(v) if v.is_finite() => length(v),
        _ => auto(),
    };
    tf::Rect { top: edge(sides.0), right: edge(sides.1), bottom: edge(sides.2), left: edge(sides.3) }
}

fn align(a: Align) -> tf::AlignItems {
    match a {
        Align::Start => tf::AlignItems::FLEX_START,
        Align::Center => tf::AlignItems::CENTER,
        Align::End => tf::AlignItems::FLEX_END,
        Align::Stretch => tf::AlignItems::STRETCH,
    }
}

fn justify(j: Justify) -> tf::JustifyContent {
    match j {
        Justify::Start => tf::JustifyContent::FLEX_START,
        Justify::Center => tf::JustifyContent::CENTER,
        Justify::End => tf::JustifyContent::FLEX_END,
        Justify::SpaceBetween => tf::JustifyContent::SPACE_BETWEEN,
        Justify::SpaceAround => tf::JustifyContent::SPACE_AROUND,
        Justify::SpaceEvenly => tf::JustifyContent::SPACE_EVENLY,
    }
}

fn overflow(o: Overflow) -> tf::Overflow {
    match o {
        Overflow::Visible => tf::Overflow::Visible,
        Overflow::Clip => tf::Overflow::Clip,
        Overflow::Scroll => tf::Overflow::Scroll,
    }
}

fn tracks(list: &[Track]) -> Vec<tf::GridTemplateComponent<String>> {
    list.iter()
        .map(|t| {
            tf::GridTemplateComponent::Single(match *t {
                Track::Px(v) if v.is_finite() => length(v.max(0.0)),
                Track::Fr(v) if v.is_finite() => fr(v.max(0.0)),
                _ => auto(),
            })
        })
        .collect()
}

/// Taffy's style for `style`, a child of a container that lays out in `parent` (`None`: no container, or a grid).
/// `padding_extra` is what the safe area adds, `canvas` fixes the size of the root.
fn to_taffy(style: &Style, parent: Option<Direction>, padding_extra: Insets, canvas: Option<(f32, f32)>) -> tf::Style {
    let border = style.border.map_or(0.0, |(w, _)| non_negative(w));
    let mut grow = non_negative(style.grow);
    let mut basis = auto();
    let mut align_self = style.align_self.map(align);
    let (mut width, mut height) = (dimension(style.width), dimension(style.height));

    // `Fill` along the container's direction is a share of what is left (as if `flex: 1 1 0`); across it, the whole side.
    if let Some(direction) = parent {
        let (main_fill, cross_fill) = if direction.is_row() { (style.width == Dim::Fill, style.height == Dim::Fill) } else { (style.height == Dim::Fill, style.width == Dim::Fill) };
        if main_fill {
            grow = grow.max(1.0);
            basis = length(0.0);
        }
        if cross_fill && align_self.is_none() {
            align_self = Some(tf::AlignItems::STRETCH);
        }
    }
    if let Some((w, h)) = canvas {
        (width, height) = (length(w), length(h));
    }

    tf::Style {
        display: match style.display {
            Display::Flex => tf::Display::Flex,
            Display::Grid => tf::Display::Grid,
            Display::None => tf::Display::None,
        },
        position: match style.position {
            Position::Relative => tf::Position::Relative,
            Position::Absolute => tf::Position::Absolute,
        },
        inset: inset(style.inset),
        size: tf::Size { width, height },
        min_size: tf::Size { width: limit(style.min_width), height: limit(style.min_height) },
        max_size: tf::Size { width: limit(style.max_width), height: limit(style.max_height) },
        aspect_ratio: style.aspect_ratio.filter(|r| r.is_finite() && *r > 0.0),
        margin: margin(style.margin),
        padding: tf::Rect {
            left: length(non_negative(style.padding.left) + padding_extra.left),
            right: length(non_negative(style.padding.right) + padding_extra.right),
            top: length(non_negative(style.padding.top) + padding_extra.top),
            bottom: length(non_negative(style.padding.bottom) + padding_extra.bottom),
        },
        border: tf::Rect { left: length(border), right: length(border), top: length(border), bottom: length(border) },
        gap: tf::Size { width: length(non_negative(style.gap.0)), height: length(non_negative(style.gap.1)) },
        align_items: align(style.align),
        align_self,
        justify_content: justify(style.justify),
        // Lines of a wrapped row stay packed at the start (as in a design tool) instead of growing to fill the height.
        align_content: tf::AlignContent::FLEX_START,
        flex_direction: match style.direction {
            Direction::Row => tf::FlexDirection::Row,
            Direction::Column => tf::FlexDirection::Column,
            Direction::RowReverse => tf::FlexDirection::RowReverse,
            Direction::ColumnReverse => tf::FlexDirection::ColumnReverse,
        },
        flex_wrap: if style.wrap { tf::FlexWrap::Wrap } else { tf::FlexWrap::NoWrap },
        flex_grow: grow,
        flex_shrink: non_negative(style.shrink),
        flex_basis: basis,
        overflow: tf::Point { x: overflow(style.overflow), y: overflow(style.overflow) },
        grid_template_columns: tracks(&style.columns),
        grid_template_rows: tracks(&style.rows),
        grid_column: tf::Line { start: tf::GridPlacement::Auto, end: tf::GridPlacement::Span(style.span.0.max(1)) },
        grid_row: tf::Line { start: tf::GridPlacement::Auto, end: tf::GridPlacement::Span(style.span.1.max(1)) },
        ..tf::Style::DEFAULT
    }
}

// --- Measuring text and images -----------------------------------------------------------------------------------------------

/// The style a text is shaped with in a box of `width` (the width of the node's content, if it is known).
fn constrained(style: &TextStyle, width: Option<f32>) -> TextStyle {
    let mut s = style.clone();
    s.max_width = match width {
        Some(w) if w.is_finite() && (style.wrap || style.ellipsis || style.align != TextAlign::Left) => Some(w.max(1.0)),
        _ => None,
    };
    s
}

/// Up to the next whole physical pixel, so that a box holds its text exactly instead of cutting a hair off it.
fn ceil_to_pixel(v: f32, scale: f32) -> f32 {
    ((v * scale) - 1e-3).ceil().max(0.0) / scale
}

fn measure_text(ts: &mut dyn Typesetter, scale: f32, text: &TextContent, known: tf::Size<Option<f32>>, available: tf::Size<tf::AvailableSpace>) -> tf::Size<f32> {
    let width = match (known.width, available.width) {
        (Some(w), _) => Some(w),
        (None, tf::AvailableSpace::Definite(w)) if text.style.wrap => Some(w),
        // The narrowest the text can be: every word on a line of its own.
        (None, tf::AvailableSpace::MinContent) if text.style.wrap => Some(1.0),
        _ => None,
    };
    let layout = ts.layout(&text.text, &constrained(&text.style, width), scale);
    tf::Size { width: known.width.unwrap_or_else(|| ceil_to_pixel(layout.width, scale)), height: known.height.unwrap_or_else(|| ceil_to_pixel(layout.height, scale)) }
}

fn measure_image(image: &ImageContent, known: tf::Size<Option<f32>>) -> tf::Size<f32> {
    let (iw, ih) = (image.size.0.max(0.0), image.size.1.max(0.0));
    match (known.width, known.height) {
        (Some(w), Some(h)) => tf::Size { width: w, height: h },
        (Some(w), None) => tf::Size { width: w, height: if iw > 0.0 { w * ih / iw } else { 0.0 } },
        (None, Some(h)) => tf::Size { width: if ih > 0.0 { h * iw / ih } else { 0.0 }, height: h },
        (None, None) => tf::Size { width: iw, height: ih },
    }
}

// --- The tree ---------------------------------------------------------------------------------------------------------------

impl UiTree {
    /// A tree with a root of this style. The root is as large as the screen's canvas, whatever its own size says.
    pub fn new(root_style: Style) -> Self {
        let mut taffy = tf::TaffyTree::new();
        // Rounding is done when drawing, to the physical pixels of the screen it is drawn on.
        taffy.disable_rounding();
        let root = taffy.new_leaf(to_taffy(&root_style, None, Insets::ZERO, None)).expect("a node can always be made");
        let mut styles = HashMap::new();
        styles.insert(root, root_style);
        Self { taffy, styles, root, laid_out_for: None, list: None, stats: TreeStats::default() }
    }

    pub fn root(&self) -> NodeId {
        NodeId(self.root)
    }

    pub fn stats(&self) -> TreeStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.len() <= 1
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.styles.contains_key(&id.0)
    }

    pub fn style(&self, id: NodeId) -> Option<&Style> {
        self.styles.get(&id.0)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.contains(id).then(|| self.taffy.parent(id.0)).flatten().map(NodeId)
    }

    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        if !self.contains(id) {
            return Vec::new();
        }
        self.taffy.children(id.0).map(|c| c.into_iter().map(NodeId).collect()).unwrap_or_default()
    }

    fn changed(&mut self) {
        self.list = None;
    }

    /// Gives `node` the Taffy style that follows from its own, from its parent's direction and (for the root) from the screen.
    fn sync(&mut self, node: tf::NodeId, safe: Insets, canvas: Option<(f32, f32)>) {
        let Some(style) = self.styles.get(&node) else { return };
        let parent = self.taffy.parent(node).and_then(|p| self.styles.get(&p)).filter(|p| p.display == Display::Flex).map(|p| p.direction);
        let extra = if style.safe_area && node == self.root { safe } else { Insets::ZERO };
        let converted = to_taffy(style, parent, extra, if node == self.root { canvas } else { None });
        let _ = self.taffy.set_style(node, converted);
    }

    fn current_screen(&self) -> (Insets, Option<(f32, f32)>) {
        match self.laid_out_for {
            Some(key) => {
                let f = f32::from_bits;
                (Insets::new(f(key.safe_area[0]), f(key.safe_area[1]), f(key.safe_area[2]), f(key.safe_area[3])), Some((f(key.canvas.0), f(key.canvas.1))))
            }
            None => (Insets::ZERO, None),
        }
    }

    fn attach(&mut self, parent: NodeId, node: tf::NodeId, style: Style) -> Option<NodeId> {
        if !self.contains(parent) {
            let _ = self.taffy.remove(node);
            return None;
        }
        self.styles.insert(node, style);
        let _ = self.taffy.add_child(parent.0, node);
        let (safe, canvas) = self.current_screen();
        self.sync(node, safe, canvas);
        self.changed();
        Some(NodeId(node))
    }

    /// Adds a node with no content of its own (a box that holds other nodes, or only looks like something) as the last child of
    /// `parent`. `None` if `parent` is not in the tree.
    pub fn add_view(&mut self, parent: NodeId, style: Style) -> Option<NodeId> {
        let node = self.taffy.new_leaf(tf::Style::DEFAULT).ok()?;
        self.attach(parent, node, style)
    }

    /// Adds a text.
    pub fn add_text(&mut self, parent: NodeId, text: &str, text_style: TextStyle, style: Style) -> Option<NodeId> {
        let content = Content::Text(TextContent { text: text.to_string(), style: text_style });
        let node = self.taffy.new_leaf_with_context(tf::Style::DEFAULT, content).ok()?;
        self.attach(parent, node, style)
    }

    /// Adds an image of the backend (an SVG), whose drawing is `size` in its own units; that is its size unless the style says more.
    pub fn add_image(&mut self, parent: NodeId, image: ImageId, size: (f32, f32), tint: Option<Color>, style: Style) -> Option<NodeId> {
        let content = Content::Image(ImageContent { image, tint, size });
        let node = self.taffy.new_leaf_with_context(tf::Style::DEFAULT, content).ok()?;
        self.attach(parent, node, style)
    }

    /// Replaces a node's style. Its children are laid out again if the direction of the node changed.
    pub fn set_style(&mut self, id: NodeId, style: Style) -> bool {
        let Some(slot) = self.styles.get_mut(&id.0) else { return false };
        let direction_changed = slot.direction != style.direction || slot.display != style.display;
        if *slot == style {
            return true;
        }
        *slot = style;
        let (safe, canvas) = self.current_screen();
        self.sync(id.0, safe, canvas);
        if direction_changed {
            for child in self.children(id) {
                self.sync(child.0, safe, canvas);
            }
        }
        self.changed();
        true
    }

    /// Changes the words of a text node. Nothing happens (and nothing is laid out again) if they are the same.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> bool {
        match self.taffy.get_node_context_mut(id.0) {
            Some(Content::Text(content)) => {
                if content.text != text {
                    content.text = text.to_string();
                    let _ = self.taffy.mark_dirty(id.0);
                    self.changed();
                }
                true
            }
            _ => false,
        }
    }

    /// Changes how a text node looks.
    pub fn set_text_style(&mut self, id: NodeId, text_style: TextStyle) -> bool {
        match self.taffy.get_node_context_mut(id.0) {
            Some(Content::Text(content)) => {
                if content.style != text_style {
                    content.style = text_style;
                    let _ = self.taffy.mark_dirty(id.0);
                    self.changed();
                }
                true
            }
            _ => false,
        }
    }

    pub fn text(&self, id: NodeId) -> Option<&str> {
        match self.taffy.get_node_context(id.0) {
            Some(Content::Text(content)) => Some(&content.text),
            _ => None,
        }
    }

    /// Takes a node and everything in it out of the tree. The root cannot be removed.
    pub fn remove(&mut self, id: NodeId) -> bool {
        if id.0 == self.root || !self.contains(id) {
            return false;
        }
        if let Some(parent) = self.taffy.parent(id.0) {
            let _ = self.taffy.remove_child(parent, id.0);
        }
        let mut pending = vec![id.0];
        while let Some(node) = pending.pop() {
            pending.extend(self.taffy.children(node).unwrap_or_default());
            self.styles.remove(&node);
            let _ = self.taffy.remove(node);
        }
        self.changed();
        true
    }

    // --- Layout ------------------------------------------------------------------------------------------------------------

    /// Works the layout out for `screen`, if anything changed since it was last done for it. Returns whether it did.
    pub fn layout(&mut self, screen: &ScreenMetrics, ts: &mut dyn Typesetter) -> bool {
        let (scale, canvas, safe) = (screen.scale(), screen.canvas(), screen.safe_area_logical());
        let key = ScreenKey { canvas: (canvas.0.to_bits(), canvas.1.to_bits()), scale: scale.to_bits(), safe_area: [safe.left.to_bits(), safe.top.to_bits(), safe.right.to_bits(), safe.bottom.to_bits()] };
        if self.laid_out_for != Some(key) {
            let scale_changed = self.laid_out_for.is_none_or(|k| k.scale != key.scale);
            self.laid_out_for = Some(key);
            let root = self.root;
            self.sync(root, safe, Some(canvas));
            if scale_changed {
                // Text is shaped for the pixels of the screen (it snaps to them): at another scale it measures differently.
                let texts: Vec<tf::NodeId> = self.styles.keys().copied().filter(|n| matches!(self.taffy.get_node_context(*n), Some(Content::Text(_)))).collect();
                for node in texts {
                    let _ = self.taffy.mark_dirty(node);
                }
            }
        }
        if !self.taffy.dirty(self.root).unwrap_or(true) {
            return false;
        }
        let mut measures = 0u64;
        let available = tf::Size { width: tf::AvailableSpace::Definite(canvas.0), height: tf::AvailableSpace::Definite(canvas.1) };
        let result = self.taffy.compute_layout_with_measure(self.root, available, |inputs, _id, context, style| {
            tf::compute_leaf_layout(
                inputs,
                style,
                |_, _| 0.0,
                |known, available| match context {
                    Some(Content::Text(text)) => {
                        measures += 1;
                        measure_text(ts, scale, text, known, available)
                    }
                    Some(Content::Image(image)) => measure_image(image, known),
                    None => tf::Size::ZERO,
                },
            )
        });
        debug_assert!(result.is_ok(), "layout failed: {result:?}");
        self.stats.layouts += 1;
        self.stats.text_measures += measures;
        self.list = None;
        true
    }

    /// Where a node is, in logical pixels of the canvas, as of the last layout.
    pub fn rect(&self, id: NodeId) -> Option<Rect> {
        if !self.contains(id) {
            return None;
        }
        let own = self.taffy.layout(id.0).ok()?;
        let (mut x, mut y) = (own.location.x, own.location.y);
        let mut up = self.taffy.parent(id.0);
        while let Some(node) = up {
            let l = self.taffy.layout(node).ok()?;
            (x, y) = (x + l.location.x, y + l.location.y);
            up = self.taffy.parent(node);
        }
        Some(Rect::new(x, y, own.size.width, own.size.height))
    }

    // --- Drawing -----------------------------------------------------------------------------------------------------------

    /// The draw list made by the last call of [`UiTree::draw_list`], if nothing changed since.
    pub fn last_list(&self) -> Option<&DrawList> {
        self.list.as_ref()
    }

    /// The draw list of the tree for `screen`: laid out if needed, made if anything changed. The second value says whether it was
    /// made now (false: it is the one of the last call).
    pub fn draw_list(&mut self, screen: &ScreenMetrics, ts: &mut dyn Typesetter) -> (&DrawList, bool) {
        self.layout(screen, ts);
        let made = self.list.is_none();
        if made {
            let mut list = DrawList::new();
            paint_node(&self.taffy, &self.styles, self.root, (0.0, 0.0), screen.scale(), ts, &mut list);
            self.stats.paints += 1;
            self.list = Some(list);
        }
        (self.list.as_ref().expect("made above"), made)
    }
}

/// A coordinate moved to the nearest whole physical pixel.
fn snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

/// A box with both edges on whole physical pixels, so that borders and fills are sharp whatever the scale.
fn snap_rect(x: f32, y: f32, w: f32, h: f32, scale: f32) -> Rect {
    let (x0, y0, x1, y1) = (snap(x, scale), snap(y, scale), snap(x + w, scale), snap(y + h, scale));
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// A gradient given in fractions of the box of its node, placed on the box.
fn place(fill: &Fill, r: Rect) -> Fill {
    match fill {
        Fill::LinearGradient { start, end, stops } => Fill::LinearGradient {
            start: Point::new(r.x + start.x * r.w, r.y + start.y * r.h),
            end: Point::new(r.x + end.x * r.w, r.y + end.y * r.h),
            stops: stops.clone(),
        },
        other => other.clone(),
    }
}

fn paint_node(taffy: &tf::TaffyTree<Content>, styles: &HashMap<tf::NodeId, Style>, node: tf::NodeId, origin: (f32, f32), scale: f32, ts: &mut dyn Typesetter, list: &mut DrawList) {
    let (Some(style), Ok(layout)) = (styles.get(&node), taffy.layout(node)) else { return };
    if style.display == Display::None {
        return;
    }
    let (x, y) = (origin.0 + layout.location.x, origin.1 + layout.location.y);
    let rect = snap_rect(x, y, layout.size.width, layout.size.height, scale);

    for shadow in &style.shadows {
        list.push(DrawCmd::Shadow { rect, radii: style.radii, offset: shadow.offset, blur: shadow.blur, spread: shadow.spread, color: shadow.color });
    }
    let border = style.border.filter(|(w, _)| *w > 0.0);
    if style.background.is_some() || border.is_some() {
        list.push(DrawCmd::RoundedRect { rect, radii: style.radii, fill: style.background.as_ref().map(|f| place(f, rect)), border });
    }

    // The content box: inside the border and the padding.
    let inset_by = border.map_or(0.0, |(w, _)| w);
    let content = Rect::new(
        rect.x + inset_by + style.padding.left,
        rect.y + inset_by + style.padding.top,
        (rect.w - 2.0 * inset_by - style.padding.left - style.padding.right).max(0.0),
        (rect.h - 2.0 * inset_by - style.padding.top - style.padding.bottom).max(0.0),
    );
    match taffy.get_node_context(node) {
        Some(Content::Text(text)) => {
            let shaped = ts.layout(&text.text, &constrained(&text.style, Some(content.w)), scale);
            let at = Point::new(snap(content.x, scale), snap(content.y, scale));
            for run in shaped.runs {
                list.text(at, run);
            }
        }
        Some(Content::Image(image)) => list.image(content, image.image, image.tint),
        None => {}
    }

    let mut children = taffy.children(node).unwrap_or_default();
    if children.is_empty() {
        return;
    }
    // Higher `z_index` is drawn later, so it is on top; equal ones keep their order.
    children.sort_by_key(|c| styles.get(c).map_or(0, |s| s.z_index));
    let clipped = style.overflow != Overflow::Visible;
    if clipped {
        list.push_clip(rect, style.radii);
    }
    for child in children {
        paint_node(taffy, styles, child, (x, y), scale, ts, list);
    }
    if clipped {
        list.pop_clip();
    }
}
