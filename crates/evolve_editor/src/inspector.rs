//! The Objects panel (scene tree) and the Inspector panel (property editor).

use crate::app::EditorApp;
use crate::objects::{icon_of, paint_icon};
use crate::theme;
use eframe::egui::{self, Sense};
use evolve_engine::geometry::{scale_around, Affine2, Rect, Vec2};
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene};
use evolve_engine::style::{Color, FillRule, Gradient, GradientKind, GradientStop, LineCap, LineJoin, Paint, Stroke};
use std::collections::HashSet;

// --- Scene tree ------------------------------------------------------------------------------

struct Row<'a> {
    node: &'a Node,
    depth: usize,
}

/// Where a dragged layer lands relative to the row it was dropped on.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Place {
    Above,
    Below,
    Into,
}

pub(crate) enum TreeAction {
    Select { id: NodeId, extend: bool },
    ToggleVisible(NodeId),
    ToggleLocked(NodeId),
    /// The pin: select the object and take the Scene view to it.
    Locate(NodeId),
    ToggleCollapsed(NodeId),
    Rename(NodeId),
    Duplicate(NodeId),
    Delete(NodeId),
    Drop { dragged: NodeId, target: NodeId, place: Place },
    /// The name being typed in the rename box.
    SetName(NodeId, String),
    StopRenaming,
}

/// What the tree needs to know to draw itself, besides the scene.
pub(crate) struct TreeView<'a> {
    pub collapsed: &'a HashSet<NodeId>,
    pub selection: &'a [NodeId],
    pub renaming: Option<NodeId>,
}

/// Rows top-most first, like the layers panel of most editors.
fn tree_rows<'a>(nodes: &'a [Node], depth: usize, collapsed: &HashSet<NodeId>, out: &mut Vec<Row<'a>>) {
    for node in nodes.iter().rev() {
        out.push(Row { node, depth });
        if node.is_group() && !collapsed.contains(&node.id) {
            tree_rows(node.children(), depth + 1, collapsed, out);
        }
    }
}

fn drop_place(row: &egui::Response, is_group: bool) -> Place {
    let Some(pointer) = row.ctx.pointer_interact_pos() else { return Place::Below };
    let t = (pointer.y - row.rect.top()) / row.rect.height().max(1.0);
    if is_group && (0.25..0.75).contains(&t) {
        Place::Into
    } else if t < 0.5 {
        Place::Above
    } else {
        Place::Below
    }
}

/// Draws the tree of all objects in `scene` in a scroll area and returns what the user asked for; the caller
/// applies it. Only the rows that are on screen are laid out: a scene can have tens of thousands of rows.
pub(crate) fn show_tree(ui: &mut egui::Ui, scene: &Scene, view: &TreeView) -> Vec<TreeAction> {
    let mut rows = Vec::new();
    tree_rows(&scene.nodes, 0, view.collapsed, &mut rows);
    if rows.is_empty() {
        ui.weak("Empty. Pick a tool and draw something.");
    }
    // Every row is exactly this tall (the minimum height of a line of widgets), which is what lets
    // the scroll area skip the rows above and below the window.
    let row_height = ui.spacing().interact_size.y;
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]).show_rows(ui, row_height, rows.len(), |ui, range| {
        for row in &rows[range] {
            tree_row(ui, row, view, &mut actions);
        }
    });
    actions
}

fn tree_row(ui: &mut egui::Ui, row: &Row, view: &TreeView, actions: &mut Vec<TreeAction>) {
    let node = row.node;
    ui.horizontal(|ui| {
        ui.add_space(row.depth as f32 * 14.0);
        let icon_size = egui::vec2(16.0, 16.0);
        if node.is_group() {
            let (_, response) = ui.allocate_exact_size(icon_size, Sense::click());
            let openness = if view.collapsed.contains(&node.id) { 0.0 } else { 1.0 };
            egui::collapsing_header::paint_default_icon(ui, openness, &response);
            if response.clicked() {
                actions.push(TreeAction::ToggleCollapsed(node.id));
            }
        } else {
            ui.add_space(icon_size.x);
        }
        let (icon_rect, _) = ui.allocate_exact_size(icon_size, Sense::hover());
        let icon_color = if node.visible { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
        paint_icon(ui.painter(), icon_rect, icon_of(node), icon_color);

        if view.renaming == Some(node.id) {
            let mut name = node.name.clone();
            let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0));
            if edit.changed() {
                actions.push(TreeAction::SetName(node.id, name));
            }
            // Ask for focus only until the field has it: asking every frame would cancel
            // the focus loss that Enter (or a click elsewhere) causes, and it would never close.
            if edit.lost_focus() {
                actions.push(TreeAction::StopRenaming);
            } else if !edit.has_focus() {
                edit.request_focus();
            }
        } else {
            tree_label(ui, row, view.selection.contains(&node.id), actions);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let lock = if node.locked { "🔒" } else { "🔓" };
            if ui.add(egui::Button::new(lock).frame(false).small()).on_hover_text("Lock").clicked() {
                actions.push(TreeAction::ToggleLocked(node.id));
            }
            let eye = egui::RichText::new("👁").weak().strikethrough();
            let eye = if node.visible { egui::RichText::new("👁") } else { eye };
            if ui.add(egui::Button::new(eye).frame(false).small()).on_hover_text("Show / hide").clicked() {
                actions.push(TreeAction::ToggleVisible(node.id));
            }
            if pin_button(ui).clicked() {
                actions.push(TreeAction::Locate(node.id));
            }
        });
    });
}

/// The pin at the end of a row: a click takes the Scene view to the object. Drawn, not written, so it looks the same everywhere.
fn pin_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), Sense::click());
    let response = response.on_hover_text("Show in the Scene view");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Show in the Scene view"));
    let color = if response.hovered() { theme::ACCENT } else { ui.visuals().weak_text_color() };
    let stroke = egui::Stroke::new(1.4, color);
    let painter = ui.painter();
    let head = rect.center() + egui::vec2(0.0, -2.5);
    painter.circle_stroke(head, 3.8, stroke);
    painter.circle_filled(head, 1.3, color);
    painter.line_segment([head + egui::vec2(0.0, 3.8), rect.center() + egui::vec2(0.0, 6.5)], stroke);
    response
}

fn tree_label(ui: &mut egui::Ui, row: &Row, selected: bool, actions: &mut Vec<TreeAction>) {
    let node = row.node;
    let mut text = egui::RichText::new(&node.name);
    if !node.visible {
        text = text.weak();
    }
    let response = ui.selectable_label(selected, text).interact(Sense::click_and_drag());
    response.dnd_set_drag_payload(node.id);

    response.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            actions.push(TreeAction::Rename(node.id));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            actions.push(TreeAction::Duplicate(node.id));
            ui.close();
        }
        if ui.button("Delete").clicked() {
            actions.push(TreeAction::Delete(node.id));
            ui.close();
        }
    });
    if response.double_clicked() {
        actions.push(TreeAction::Rename(node.id));
    } else if response.clicked() {
        let extend = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        actions.push(TreeAction::Select { id: node.id, extend });
    }

    if response.dnd_hover_payload::<NodeId>().is_some() {
        let place = drop_place(&response, node.is_group());
        let stroke = egui::Stroke::new(2.0, theme::ACCENT);
        match place {
            Place::Above => {
                ui.painter().hline(response.rect.x_range(), response.rect.top(), stroke);
            }
            Place::Below => {
                ui.painter().hline(response.rect.x_range(), response.rect.bottom(), stroke);
            }
            Place::Into => {
                ui.painter().rect_stroke(response.rect, 2.0, stroke, egui::StrokeKind::Inside);
            }
        }
    }
    if let Some(dragged) = response.dnd_release_payload::<NodeId>() {
        let place = drop_place(&response, node.is_group());
        actions.push(TreeAction::Drop { dragged: *dragged, target: node.id, place });
    }
}

impl EditorApp {
    /// The Inspector panel: properties of the selected object (or the document).
    pub fn inspector_pane(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("properties").auto_shrink([false, false]).show(ui, |ui| {
            self.properties(ui);
        });
    }

    /// The tree of all objects (the body of the Objects panel).
    pub(crate) fn scene_tree(&mut self, ui: &mut egui::Ui) {
        let view = TreeView { collapsed: &self.tree.collapsed, selection: &self.selection, renaming: self.tree.renaming };
        let actions = show_tree(ui, &self.scene, &view);
        for action in actions {
            self.apply_tree_action(action);
        }
    }

    fn apply_tree_action(&mut self, action: TreeAction) {
        match action {
            TreeAction::Select { id, extend } => {
                self.canvas.anchor = None;
                if !extend {
                    self.selection = vec![id];
                } else if let Some(i) = self.selection.iter().position(|&s| s == id) {
                    self.selection.remove(i);
                } else {
                    self.selection.push(id);
                }
            }
            TreeAction::ToggleVisible(id) => {
                if let Some(mut node) = self.scene.find_mut(id) {
                    let visible = !node.visible;
                    node.set_visible(visible);
                }
            }
            TreeAction::ToggleLocked(id) => {
                if let Some(mut node) = self.scene.find_mut(id) {
                    let locked = !node.locked;
                    *node.locked_mut() = locked;
                }
            }
            TreeAction::Locate(id) => self.locate(id),
            TreeAction::ToggleCollapsed(id) => {
                if !self.tree.collapsed.remove(&id) {
                    self.tree.collapsed.insert(id);
                }
            }
            TreeAction::Rename(id) => self.tree.renaming = Some(id),
            TreeAction::Duplicate(id) => {
                self.select_for_command(id);
                self.duplicate_selection();
            }
            TreeAction::Delete(id) => {
                self.select_for_command(id);
                self.delete_selection();
            }
            TreeAction::Drop { dragged, target, place } => self.drop_layer(dragged, target, place),
            TreeAction::SetName(id, name) => {
                if let Some(mut node) = self.scene.find_mut(id) {
                    *node.name_mut() = name;
                }
            }
            TreeAction::StopRenaming => self.tree.renaming = None,
        }
    }

    /// A command from a row's menu acts on the selection if the row is part of it, otherwise on the row alone.
    fn select_for_command(&mut self, id: NodeId) {
        if !self.selection.contains(&id) {
            self.selection = vec![id];
        }
    }

    fn drop_layer(&mut self, dragged: NodeId, target: NodeId, place: Place) {
        if dragged == target {
            return;
        }
        if place == Place::Into {
            self.scene.move_node(dragged, Some(target), usize::MAX);
            return;
        }
        let Some((parent, target_index)) = self.scene.locate(target) else { return };
        // The list shows the top-most layer first, so "above" means a higher index.
        let mut index = if place == Place::Above { target_index + 1 } else { target_index };
        // `move_node` counts positions after the dragged node is taken out.
        if matches!(self.scene.locate(dragged), Some((p, i)) if p == parent && i < target_index) {
            index -= 1;
        }
        self.scene.move_node(dragged, parent, index);
    }

    // --- Properties ----------------------------------------------------------------------------

    fn properties(&mut self, ui: &mut egui::Ui) {
        if self.selection.is_empty() {
            self.document_properties(ui);
            self.rig_properties(ui);
            return;
        }
        if self.camera_properties(ui) {
            return;
        }
        self.node_properties(ui);
        self.instance_properties(ui);
        self.style_properties(ui);
        self.rig_properties(ui);
        self.game_properties(ui);
    }

    /// Color overrides of a selected symbol instance.
    fn instance_properties(&mut self, ui: &mut egui::Ui) {
        let [id] = self.selection[..] else { return };
        let Some(Node { kind: NodeKind::Instance(instance), .. }) = self.scene.find(id) else { return };
        let symbol_name = self.scene.symbol(instance.symbol).map_or("(missing)", |s| s.name.as_str()).to_string();
        let (symbol, original) = (instance.symbol, instance.clone());
        let mut edited = original.clone();

        ui.separator();
        ui.strong("Symbol instance");
        ui.label(format!("Master: {symbol_name}"));
        optional_color(ui, "Override fill", &mut edited.fill);
        optional_color(ui, "Override stroke", &mut edited.stroke);
        ui.horizontal(|ui| {
            if ui.button("Detach").on_hover_text("Turn into an ordinary group").clicked() {
                self.detach_selected_instances();
            }
            if ui.button("Edit master").clicked() {
                self.edit_master(symbol);
            }
        });
        if edited != original {
            if let Some(mut node) = self.scene.find_mut(id) {
                if let NodeKind::Instance(instance) = &mut node.kind {
                    *instance = edited;
                }
            }
        }
    }

    /// Shown when nothing is selected: the scene and its camera.
    fn document_properties(&mut self, ui: &mut egui::Ui) {
        ui.strong("Scene");
        let count = self.scene.all_ids().len();
        ui.label(format!("{count} {}", if count == 1 { "object" } else { "objects" }));
        match self.scene.main_camera() {
            Some(camera) => {
                let name = self.scene.find(camera).map_or(String::new(), |n| n.name.clone());
                ui.horizontal(|ui| {
                    ui.label(format!("Camera: {name}"));
                    if ui.button("Select").clicked() {
                        self.selection = vec![camera];
                    }
                });
            }
            None => {
                ui.label("No camera: the game shows a default 1280 x 720 frame.");
                if ui.button("Add camera").clicked() {
                    self.add_camera();
                }
            }
        }
    }

    /// The Inspector of a single selected camera. Returns false for anything else.
    fn camera_properties(&mut self, ui: &mut egui::Ui) -> bool {
        let [id] = self.selection[..] else { return false };
        let Some(Node { kind: NodeKind::Camera(camera), name, .. }) = self.scene.find(id) else { return false };
        let (original, mut name) = (camera.clone(), name.clone());
        let view = self.scene.camera_view();
        let (mut edited, mut center, mut degrees) = (original.clone(), view.center, view.rotation.to_degrees());
        let main = self.scene.main_camera() == Some(id);

        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut name);
        });
        if !main {
            ui.weak("Only the first camera in the Objects list is used by the game.");
        }
        ui.separator();
        ui.strong("Camera");
        let mut moved = false;
        egui::Grid::new("camera_grid").num_columns(4).show(ui, |ui| {
            ui.label("X");
            moved |= ui.add(egui::DragValue::new(&mut center.x).speed(0.5)).changed();
            ui.label("Y");
            moved |= ui.add(egui::DragValue::new(&mut center.y).speed(0.5)).changed();
            ui.end_row();
            ui.label("W");
            ui.add(egui::DragValue::new(&mut edited.size.x).speed(1.0).range(16.0..=16384.0));
            ui.label("H");
            ui.add(egui::DragValue::new(&mut edited.size.y).speed(1.0).range(16.0..=16384.0));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            ui.label("Rotation");
            moved |= ui.add(egui::DragValue::new(&mut degrees).speed(0.5).suffix("°")).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Background");
            color_button(ui, &mut edited.background);
        });
        ui.weak("The frame is what the player sees. W and H are in world units; only their ratio matters on screen.");

        if moved {
            self.set_camera_pose(id, center, degrees.to_radians());
        }
        // The panel runs every frame; only a real edit may count as a change.
        let changed = self.scene.find(id).is_some_and(|n| n.name != name || !matches!(&n.kind, NodeKind::Camera(c) if *c == edited));
        if changed {
            if let Some(mut node) = self.scene.find_mut(id) {
                node.name = name;
                if let NodeKind::Camera(camera) = &mut node.kind {
                    *camera = edited;
                }
            }
        }
        true
    }

    /// Puts the camera's frame center and rotation at the given world values (its scale is reset).
    fn set_camera_pose(&mut self, id: NodeId, center: Vec2, rotation: f32) {
        let world = Affine2::from_angle_translation(rotation, center);
        let parent_inverse = self.scene.parent_world(id).inverse();
        if let Some(mut node) = self.scene.find_mut(id) {
            node.set_transform(parent_inverse * world);
        }
    }

    /// Adds a camera whose frame covers the default 1280 x 720 area, and selects it.
    pub fn add_camera(&mut self) {
        let camera = self.scene.new_camera(evolve_engine::game_camera::DEFAULT_SIZE, evolve_engine::game_camera::DEFAULT_SIZE * 0.5);
        let id = camera.id;
        self.scene.insert(camera, None, 0); // at the bottom of the list, so it is the first camera
        self.selection = vec![id];
    }

    fn node_properties(&mut self, ui: &mut egui::Ui) {
        if let [id] = self.selection[..] {
            // Edit copies and write back only on a change: this runs every frame.
            if let Some(node) = self.scene.find(id) {
                let (mut name, mut clip, is_group) = (node.name.clone(), node.clip, node.is_group());
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut name);
                });
                if is_group {
                    ui.checkbox(&mut clip, "Clip to top object");
                }
                if let Some(mut node) = self.scene.find_mut(id).filter(|n| n.name != name || n.clip != clip) {
                    if node.name != name {
                        *node.name_mut() = name;
                    }
                    if node.clip != clip {
                        node.set_clip(clip);
                    }
                }
            }
        } else {
            ui.label(format!("{} objects selected", self.selection.len()));
        }

        // Position and size of the selection's bounding box.
        if let Some(bounds) = self.selection_bounds() {
            let (mut x, mut y) = (bounds.min.x, bounds.min.y);
            let (mut w, mut h) = (bounds.size().x, bounds.size().y);
            let mut changed = false;
            egui::Grid::new("transform_grid").num_columns(4).show(ui, |ui| {
                ui.label("X");
                changed |= ui.add(egui::DragValue::new(&mut x).speed(0.5)).changed();
                ui.label("Y");
                changed |= ui.add(egui::DragValue::new(&mut y).speed(0.5)).changed();
                ui.end_row();
                ui.label("W");
                changed |= ui.add(egui::DragValue::new(&mut w).speed(0.5).range(0.01..=f32::MAX)).changed();
                ui.label("H");
                changed |= ui.add(egui::DragValue::new(&mut h).speed(0.5).range(0.01..=f32::MAX)).changed();
                ui.end_row();
            });
            if changed {
                self.set_selection_bounds(bounds, Rect::from_points(Vec2::new(x, y), Vec2::new(x + w, y + h)));
            }
        }

        // Opacity applies to the selected nodes themselves.
        let ids = self.scene.top_level_selection(&self.selection);
        if let Some(mut opacity) = ids.first().and_then(|&id| self.scene.find(id)).map(|n| n.opacity) {
            let before = opacity;
            ui.horizontal(|ui| {
                ui.label("Opacity");
                ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).show_value(true));
            });
            if opacity != before {
                for id in ids {
                    if let Some(mut node) = self.scene.find_mut(id) {
                        node.set_opacity(opacity);
                    }
                }
            }
        }
    }

    fn set_selection_bounds(&mut self, old: Rect, new: Rect) {
        let scale = new.size() / old.size().max(Vec2::splat(1e-4));
        let delta = Affine2::from_translation(new.min - old.min) * scale_around(scale, old.min);
        let items: Vec<_> = self
            .scene
            .top_level_selection(&self.selection)
            .into_iter()
            .filter_map(|id| Some((id, self.scene.world_transform(id)?)))
            .collect();
        self.apply_world_delta(&items, delta);
    }

    /// Fill and stroke of every shape in the selection. The first shape is shown;
    /// a changed field is written to all of them.
    fn style_properties(&mut self, ui: &mut egui::Ui) {
        let shape_ids = self.selected_shapes();
        let Some(&first) = shape_ids.first() else { return };
        let Some(NodeKind::Shape(original)) = self.scene.find(first).map(|n| n.kind.clone()) else { return };
        let mut edited = original.clone();
        let bounds = edited.path.bounds();

        ui.separator();
        ui.strong("Fill");
        optional_paint(ui, "fill", &mut edited.fill, bounds, Paint::Solid(Color::rgb(86, 156, 214)));
        ui.horizontal(|ui| {
            ui.label("Rule");
            egui::ComboBox::from_id_salt("fill_rule")
                .selected_text(if edited.fill_rule == FillRule::NonZero { "Non-zero" } else { "Even-odd" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut edited.fill_rule, FillRule::NonZero, "Non-zero");
                    ui.selectable_value(&mut edited.fill_rule, FillRule::EvenOdd, "Even-odd");
                });
        });

        ui.separator();
        ui.strong("Stroke");
        let mut stroke_paint = edited.stroke.as_ref().map(|s| s.paint.clone());
        optional_paint(ui, "stroke", &mut stroke_paint, bounds, Paint::Solid(Color::rgb(24, 40, 64)));
        match (&mut edited.stroke, stroke_paint) {
            (None, Some(paint)) => edited.stroke = Some(Stroke { paint, ..Stroke::solid(Color::BLACK, 2.0) }),
            (Some(_), None) => edited.stroke = None,
            (Some(stroke), Some(paint)) => {
                stroke.paint = paint;
                stroke_options(ui, stroke);
            }
            (None, None) => {}
        }

        if edited.fill != original.fill {
            self.edit_shapes(&shape_ids, |s| s.fill = edited.fill.clone());
        }
        if edited.stroke != original.stroke {
            self.edit_shapes(&shape_ids, |s| s.stroke = edited.stroke.clone());
        }
        if edited.fill_rule != original.fill_rule {
            self.edit_shapes(&shape_ids, |s| s.fill_rule = edited.fill_rule);
        }
    }

    /// Every shape in the selection, including those inside selected groups.
    fn selected_shapes(&self) -> Vec<NodeId> {
        fn collect(node: &Node, out: &mut Vec<NodeId>) {
            match &node.kind {
                NodeKind::Shape(_) => out.push(node.id),
                NodeKind::Group(children) => children.iter().for_each(|c| collect(c, out)),
                NodeKind::Instance(_) | NodeKind::Camera(_) => {} // an instance is styled through its overrides; a camera has no style
            }
        }
        let mut shapes = Vec::new();
        for id in self.scene.top_level_selection(&self.selection) {
            if let Some(node) = self.scene.find(id) {
                collect(node, &mut shapes);
            }
        }
        shapes
    }

    fn edit_shapes(&mut self, ids: &[NodeId], edit: impl Fn(&mut evolve_engine::scene::Shape)) {
        for &id in ids {
            if let Some(shape) = self.scene.shape_mut(id) {
                edit(shape);
            }
        }
    }
}

// --- Widgets ---------------------------------------------------------------------------------

pub(crate) fn color_button(ui: &mut egui::Ui, color: &mut Color) {
    let mut rgba = [color.r, color.g, color.b, color.a];
    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
        *color = Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
    }
}

/// A checkbox that turns a color override on or off, with its color button.
fn optional_color(ui: &mut egui::Ui, label: &str, color: &mut Option<Color>) {
    ui.horizontal(|ui| {
        let mut enabled = color.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *color = enabled.then_some(Color::rgb(86, 156, 214));
        }
        if let Some(color) = color {
            color_button(ui, color);
        }
    });
}

/// A checkbox that turns a paint on or off, followed by the paint editor.
fn optional_paint(ui: &mut egui::Ui, id: &str, paint: &mut Option<Paint>, bounds: Option<evolve_engine::geometry::Rect>, default: Paint) {
    let mut enabled = paint.is_some();
    if ui.checkbox(&mut enabled, "Enabled").changed() {
        *paint = enabled.then_some(default);
    }
    if let Some(paint) = paint {
        paint_editor(ui, id, paint, bounds);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PaintKind {
    Solid,
    Linear,
    Radial,
}

fn paint_kind(paint: &Paint) -> PaintKind {
    match paint {
        Paint::Solid(_) => PaintKind::Solid,
        Paint::Gradient(Gradient { kind: GradientKind::Linear { .. }, .. }) => PaintKind::Linear,
        Paint::Gradient(Gradient { kind: GradientKind::Radial { .. }, .. }) => PaintKind::Radial,
    }
}

fn first_color(paint: &Paint) -> Color {
    match paint {
        Paint::Solid(c) => *c,
        Paint::Gradient(g) => g.stops.first().map_or(Color::WHITE, |s| s.color),
    }
}

/// Converts between solid, linear and radial paints, fitting gradients to `bounds`.
fn convert_paint(paint: &Paint, to: PaintKind, bounds: Option<Rect>) -> Paint {
    let bounds = bounds.unwrap_or(Rect { min: Vec2::ZERO, max: Vec2::splat(100.0) });
    let (center, size) = (bounds.center(), bounds.size());
    let stops = match paint {
        Paint::Gradient(g) => g.stops.clone(),
        Paint::Solid(c) => {
            let dark = Color::rgb(c.r / 2, c.g / 2, c.b / 2).with_alpha_scaled(c.a as f32 / 255.0);
            vec![GradientStop { offset: 0.0, color: *c }, GradientStop { offset: 1.0, color: dark }]
        }
    };
    let kind = match to {
        PaintKind::Solid => return Paint::Solid(first_color(paint)),
        PaintKind::Linear => GradientKind::Linear {
            start: Vec2::new(bounds.min.x, center.y),
            end: Vec2::new(bounds.max.x, center.y),
        },
        PaintKind::Radial => GradientKind::Radial { center, radius: size.max_element() * 0.5 },
    };
    Paint::Gradient(Gradient { kind, stops, transform: Affine2::IDENTITY })
}

fn paint_editor(ui: &mut egui::Ui, id: &str, paint: &mut Paint, bounds: Option<Rect>) {
    let current = paint_kind(paint);
    let mut selected = current;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(match current {
                PaintKind::Solid => "Solid",
                PaintKind::Linear => "Linear gradient",
                PaintKind::Radial => "Radial gradient",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, PaintKind::Solid, "Solid");
                ui.selectable_value(&mut selected, PaintKind::Linear, "Linear gradient");
                ui.selectable_value(&mut selected, PaintKind::Radial, "Radial gradient");
            });
        match paint {
            Paint::Solid(color) => color_button(ui, color),
            Paint::Gradient(gradient) => {
                if let (Some(first), Some(last)) = (gradient.stops.first().map(|s| s.color), gradient.stops.last().map(|s| s.color)) {
                    let (mut a, mut b) = (first, last);
                    color_button(ui, &mut a);
                    color_button(ui, &mut b);
                    if a != first {
                        gradient.stops[0].color = a;
                    }
                    if b != last {
                        let end = gradient.stops.len() - 1;
                        gradient.stops[end].color = b;
                    }
                }
            }
        }
    });
    if selected != current {
        *paint = convert_paint(paint, selected, bounds);
    }

    if let Paint::Gradient(Gradient { kind: GradientKind::Linear { start, end }, .. }) = paint {
        let delta = *end - *start;
        let mut degrees = delta.to_angle().to_degrees().rem_euclid(360.0);
        let before = degrees;
        ui.horizontal(|ui| {
            ui.label("Angle");
            ui.add(egui::Slider::new(&mut degrees, 0.0..=360.0).suffix("°"));
        });
        if degrees != before {
            let (center, half) = ((*start + *end) * 0.5, delta.length().max(1.0) * 0.5);
            let direction = Vec2::from_angle(degrees.to_radians());
            *start = center - direction * half;
            *end = center + direction * half;
        }
    }
}

fn stroke_options(ui: &mut egui::Ui, stroke: &mut Stroke) {
    ui.horizontal(|ui| {
        ui.label("Width");
        ui.add(egui::DragValue::new(&mut stroke.width).speed(0.1).range(0.0..=1000.0));
    });
    ui.horizontal(|ui| {
        ui.label("Join");
        egui::ComboBox::from_id_salt("join").selected_text(format!("{:?}", stroke.join)).show_ui(ui, |ui| {
            for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
                ui.selectable_value(&mut stroke.join, join, format!("{join:?}"));
            }
        });
        ui.label("Cap");
        egui::ComboBox::from_id_salt("cap").selected_text(format!("{:?}", stroke.cap)).show_ui(ui, |ui| {
            for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                ui.selectable_value(&mut stroke.cap, cap, format!("{cap:?}"));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::scene::Scene;

    struct Frames {
        ctx: egui::Context,
        time: f64,
    }

    impl Frames {
        fn new() -> Self {
            Self { ctx: egui::Context::default(), time: 0.0 }
        }

        /// Runs one frame of the tree in a 300 x 300 window. Returns what the user asked for and how many shapes were painted.
        fn run(&mut self, scene: &Scene, events: Vec<egui::Event>) -> (Vec<TreeAction>, usize) {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 300.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let collapsed = HashSet::new();
            let view = TreeView { collapsed: &collapsed, selection: &[], renaming: None };
            let mut actions = Vec::new();
            let mut output = self.ctx.run_ui(input, |ui| actions = show_tree(ui, scene, &view));
            output.textures_delta.clear(); // there is no graphics card to hand them to
            (actions, output.shapes.len())
        }

        fn click(&mut self, scene: &Scene, at: egui::Pos2) -> Vec<TreeAction> {
            let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            self.run(scene, vec![egui::Event::PointerMoved(at)]);
            self.run(scene, vec![button(true)]);
            self.run(scene, vec![button(false)]).0
        }
    }

    fn clicked_node(actions: &[TreeAction]) -> Option<NodeId> {
        actions.iter().find_map(|a| match a {
            TreeAction::Select { id, .. } => Some(*id),
            _ => None,
        })
    }

    #[test]
    fn a_long_tree_paints_only_the_visible_rows_and_clicks_hit_the_row_under_the_pointer() {
        let mut scene = Scene::default();
        let ids: Vec<NodeId> = (0..3000)
            .map(|i| {
                let node = scene.new_node(format!("node {i}"), NodeKind::Group(Vec::new()));
                scene.add(node, None)
            })
            .collect();
        let mut frames = Frames::new();
        let (_, painted) = frames.run(&scene, Vec::new());
        let (_, painted) = (painted, frames.run(&scene, Vec::new()).1);
        assert!(painted < 1000, "3000 rows, but {painted} shapes were painted: the hidden rows are being drawn");

        // The top-most object (the last one) is the first row.
        let first_row = egui::pos2(80.0, 12.0);
        assert_eq!(clicked_node(&frames.click(&scene, first_row)), Some(ids[2999]), "the first row is the last object");

        // After scrolling far down, the same spot shows a row from deep in the list, and the click still hits it.
        for _ in 0..40 {
            frames.run(&scene, vec![egui::Event::PointerMoved(first_row), egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -300.0), modifiers: Default::default(), phase: egui::TouchPhase::Move }]);
        }
        for _ in 0..60 {
            frames.run(&scene, Vec::new());
        }
        let clicked = clicked_node(&frames.click(&scene, first_row)).expect("a row is under the pointer");
        let row_from_top = 2999 - ids.iter().position(|&id| id == clicked).unwrap();
        assert!(row_from_top > 100, "the list did not scroll (clicked row {row_from_top})");
        assert!(row_from_top < 3000);
    }
}
