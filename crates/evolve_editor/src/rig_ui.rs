//! Rigging in the editor: the Bone and Weights tools, bone/weight overlays, IK solving,
//! and the Bone / Skin / IK sections of the properties panel.

use crate::app::EditorApp;
use crate::overlay::Screen;
use crate::theme::{ACCENT, GUIDE};
use crate::tools::{CanvasInput, Drag, Tool};
use eframe::egui::{self, Color32, Painter, Stroke};
use evolve_engine::animation::{compose, decompose};
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::scene::{Node, NodeId, NodeKind};

/// Weight-painting settings.
pub struct RigState {
    /// Index (in the skin's bone list) of the bone being painted.
    pub weight_bone: usize,
    /// Brush radius in screen points.
    pub brush_radius: f32,
    /// Weight added per frame at the brush center.
    pub brush_strength: f32,
}

impl Default for RigState {
    fn default() -> Self {
        Self { weight_bone: 0, brush_radius: 40.0, brush_strength: 0.12 }
    }
}

/// Blue (no influence) to red (full influence).
fn heat(weight: f32) -> Color32 {
    let t = weight.clamp(0.0, 1.0);
    let mix = |a: f32, b: f32| (a + (b - a) * t) as u8;
    Color32::from_rgb(mix(60.0, 255.0), mix(120.0, 70.0), mix(255.0, 60.0))
}

fn draw_bone(painter: &Painter, screen: &Screen, origin: Vec2, tip: Vec2, selected: bool) {
    let (a, b) = (screen.pt(origin), screen.pt(tip));
    let along = b - a;
    let length = along.length();
    if length < 1.0 {
        return;
    }
    let direction = along / length;
    let normal = egui::vec2(-direction.y, direction.x);
    let half_width = (length * 0.12).clamp(3.0, 9.0);
    let shoulder = a + direction * (length * 0.18);
    let (fill, outline) = if selected {
        (Color32::from_rgba_unmultiplied(76, 154, 255, 150), ACCENT)
    } else {
        (Color32::from_rgba_unmultiplied(255, 190, 70, 110), Color32::from_rgb(255, 210, 120))
    };
    painter.add(egui::Shape::convex_polygon(
        vec![a, shoulder + normal * half_width, b, shoulder - normal * half_width],
        fill,
        Stroke::new(1.0, outline),
    ));
    painter.circle_filled(a, 3.0, outline);
}

impl EditorApp {
    // --- Per-frame rig evaluation and commands ------------------------------------------------

    /// Solves IK chains in the scene. The solved bone transforms are also written into the
    /// history's snapshot, so they never count as user edits (and never get auto-keyed).
    pub fn solve_rig(&mut self) {
        if self.scene.ik.is_empty() || self.anim.playing {
            return;
        }
        self.scene.solve_ik();
        let scene = &self.scene;
        self.history.edit_committed(|committed| scene.copy_ik_bones_to(committed));
    }

    /// Binds the selected shapes to the selected bones.
    pub fn bind_selection(&mut self) {
        let bones: Vec<NodeId> = self.selection.iter().copied().filter(|&id| self.scene.is_bone(id)).collect();
        let shapes: Vec<NodeId> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| matches!(self.scene.find(id), Some(Node { kind: NodeKind::Shape(_), .. })))
            .collect();
        if bones.is_empty() || shapes.is_empty() {
            self.status = "Select the shapes and the bones they should follow".into();
            return;
        }
        let bound = shapes.iter().filter(|&&shape| self.scene.bind_skin(shape, &bones)).count();
        self.status = format!("Bound {bound} shape(s) to {} bone(s). Refine with the Weights tool", bones.len());
        self.selection = shapes;
        self.rig.weight_bone = 0;
    }

    /// Adds an IK constraint from the selected tip bone to the selected target object.
    pub fn create_ik_selection(&mut self) {
        let tip = self.selection.iter().copied().find(|&id| self.scene.is_bone(id));
        let target = self.selection.iter().copied().find(|&id| !self.scene.is_bone(id));
        self.status = match (tip, target) {
            (Some(tip), Some(target)) if self.scene.create_ik(tip, target, 2).is_some() => {
                "IK added: move the target to pose the chain. Change the chain length in the properties".into()
            }
            _ => "Select a tip bone and a target object (any shape or group)".into(),
        };
    }

    // --- Bone tool -----------------------------------------------------------------------------

    pub fn bone_tool(&mut self, input: &CanvasInput) {
        if input.drag_started {
            self.begin_bone_drag(input);
        }
        if input.dragging {
            self.update_bone_drag(input);
        }
        if input.drag_stopped {
            self.end_bone_drag(input);
        }
        if input.clicked {
            match input.pos.and_then(|p| self.scene.pick_bone(p, input.tolerance)) {
                Some(bone) => self.selection = vec![bone],
                None if !input.shift => self.selection.clear(),
                None => {}
            }
        }
    }

    fn begin_bone_drag(&mut self, input: &CanvasInput) {
        let origin = input.origin;
        // A bone's tip starts a child bone.
        let parent = self.scene.bone_ids().into_iter().find(|&b| {
            self.scene.bone_segment(b).is_some_and(|(_, tip)| tip.distance(origin) <= input.tolerance * 1.6)
        });
        if let Some(parent) = parent {
            if let Some((_, tip)) = self.scene.bone_segment(parent) {
                self.canvas.drag = Drag::BoneCreate { start: tip, parent: Some(parent) };
                return;
            }
        }
        // A bone's body turns it (or moves it with Shift).
        if let Some(bone) = self.scene.pick_bone(origin, input.tolerance) {
            self.selection = vec![bone];
            self.canvas.drag = match self.scene.world_transform(bone) {
                Some(world) if input.shift => Drag::BoneMove { items: vec![(bone, world)], start: origin },
                _ => Drag::BoneAim { id: bone },
            };
            return;
        }
        self.canvas.drag = Drag::BoneCreate { start: origin, parent: None };
    }

    fn update_bone_drag(&mut self, input: &CanvasInput) {
        let Some(pos) = input.pos else { return };
        match std::mem::take(&mut self.canvas.drag) {
            Drag::BoneAim { id } => {
                if let Some((origin, _)) = self.scene.bone_segment(id) {
                    let length = input.alt.then(|| origin.distance(pos));
                    self.scene.set_bone_world(id, origin, (pos - origin).to_angle(), length);
                }
                self.canvas.drag = Drag::BoneAim { id };
            }
            Drag::BoneMove { items, start } => {
                self.apply_world_delta(&items, Affine2::from_translation(pos - start));
                self.canvas.drag = Drag::BoneMove { items, start };
            }
            other => self.canvas.drag = other,
        }
    }

    fn end_bone_drag(&mut self, input: &CanvasInput) {
        if let (Drag::BoneCreate { start, parent }, Some(pos)) = (&self.canvas.drag, input.pos) {
            let offset = pos - *start;
            if offset.length() * self.camera.zoom >= 6.0 {
                let bone = self.scene.add_bone(*parent, *start, offset.to_angle(), offset.length());
                self.selection = vec![bone];
            }
        }
        self.canvas.drag = Drag::None;
    }

    // --- Weights tool --------------------------------------------------------------------------

    pub fn weights_tool(&mut self, input: &CanvasInput) {
        if !(input.drag_started || input.dragging || input.clicked) {
            return;
        }
        let (Some(shape), Some(pos)) = (self.edited_shape(), input.pos) else { return };
        let radius = self.rig.brush_radius / self.camera.zoom;
        let amount = self.rig.brush_strength * if input.alt { -1.0 } else { 1.0 };
        self.scene.paint_skin(shape, pos, radius, self.rig.weight_bone, amount);
    }

    // --- Overlays ------------------------------------------------------------------------------

    pub fn paint_bones(&self, painter: &Painter, screen: &Screen, input: &CanvasInput) {
        for id in self.scene.bone_ids() {
            if let Some((origin, tip)) = self.scene.bone_segment(id) {
                draw_bone(painter, screen, origin, tip, self.selection.contains(&id));
            }
        }
        // Each IK chain is linked to its target by a dashed-looking guide line.
        for ik in &self.scene.ik {
            let tip = self.scene.bone_segment(ik.tip).map(|(_, tip)| tip);
            let target = self.scene.world_transform(ik.target).map(|w| w.translation);
            if let (Some(tip), Some(target)) = (tip, target) {
                painter.line_segment([screen.pt(tip), screen.pt(target)], Stroke::new(1.0, GUIDE));
            }
        }
        if let (Drag::BoneCreate { start, .. }, Some(pos)) = (&self.canvas.drag, input.pos) {
            draw_bone(painter, screen, *start, pos, true);
        }
    }

    /// While painting weights: each anchor is colored by the active bone's influence, plus the brush.
    pub fn paint_weight_heatmap(&self, painter: &Painter, screen: &Screen, input: &CanvasInput) {
        let Some(id) = self.edited_shape() else { return };
        let Some(Node { kind: NodeKind::Shape(shape), .. }) = self.scene.find(id) else { return };
        let Some(skin) = &shape.skin else { return };
        let Some(shown) = skin.deform(&self.scene, &shape.path, Affine2::IDENTITY) else { return };
        let weights = skin.weights_of_bone(self.rig.weight_bone);
        for (anchor, weight) in shown.subpaths.iter().flat_map(|s| &s.anchors).zip(weights) {
            let center = screen.pt(anchor.pos);
            painter.circle_filled(center, 5.0, heat(weight));
            painter.circle_stroke(center, 5.0, Stroke::new(1.0, Color32::from_black_alpha(200)));
        }
        if let Some(pos) = input.pos {
            painter.circle_stroke(screen.pt(pos), self.rig.brush_radius, Stroke::new(1.0, Color32::WHITE));
        }
    }

    // --- Properties panel ----------------------------------------------------------------------

    pub fn rig_properties(&mut self, ui: &mut egui::Ui) {
        self.bone_section(ui);
        self.skin_section(ui);
        self.ik_section(ui);
    }

    fn bone_section(&mut self, ui: &mut egui::Ui) {
        let [id] = self.selection[..] else { return };
        let Some(Node { bone: Some(bone), transform, .. }) = self.scene.find(id) else { return };
        let (position, rotation, scale) = decompose(transform);
        let (mut length, mut degrees) = (bone.length, rotation.to_degrees());

        ui.separator();
        ui.strong("Bone");
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Length");
            changed |= ui.add(egui::DragValue::new(&mut length).speed(0.5).range(1.0..=10_000.0)).changed();
            ui.label("Angle");
            changed |= ui.add(egui::DragValue::new(&mut degrees).speed(0.5).suffix("°")).changed();
        });
        if changed {
            if let Some(mut node) = self.scene.find_mut(id) {
                if let Some(bone) = node.bone_mut() {
                    bone.length = length;
                }
                node.set_transform(compose(position, degrees.to_radians(), scale));
            }
        }
    }

    fn skin_section(&mut self, ui: &mut egui::Ui) {
        let [id] = self.selection[..] else { return };
        let Some(Node { kind: NodeKind::Shape(shape), .. }) = self.scene.find(id) else { return };
        ui.separator();
        ui.strong("Skin");
        let Some(skin) = &shape.skin else {
            ui.add(egui::Label::new(egui::RichText::new("Not skinned. Select this shape plus some bones, then Object > Bind Shape to Selected Bones.").weak()).wrap());
            return;
        };
        let matches = skin.deform(&self.scene, &shape.path, Affine2::IDENTITY).is_some();
        let names: Vec<String> = skin
            .bones
            .iter()
            .map(|&b| self.scene.find(b).map_or("(missing)".to_string(), |n| n.name.clone()))
            .collect();

        if !matches {
            ui.colored_label(Color32::from_rgb(255, 140, 90), "The outline no longer matches the weights. Unbind and bind again.");
        }
        ui.label("Active bone (Weights tool)");
        self.rig.weight_bone = self.rig.weight_bone.min(names.len().saturating_sub(1));
        for (i, name) in names.iter().enumerate() {
            ui.selectable_value(&mut self.rig.weight_bone, i, name);
        }
        ui.add(egui::Slider::new(&mut self.rig.brush_radius, 5.0..=150.0).text("Brush"));
        ui.add(egui::Slider::new(&mut self.rig.brush_strength, 0.01..=1.0).text("Strength"));
        ui.horizontal(|ui| {
            if ui.button("Paint weights").clicked() {
                self.set_tool(Tool::Weights);
            }
            if ui.button("Unbind skin").clicked() {
                self.scene.unbind_skin(id);
            }
        });
    }

    fn ik_section(&mut self, ui: &mut egui::Ui) {
        if self.scene.ik.is_empty() {
            return;
        }
        let name_of = |scene: &evolve_engine::scene::Scene, id: NodeId| scene.find(id).map_or("(missing)".to_string(), |n| n.name.clone());
        let labels: Vec<String> = self
            .scene
            .ik
            .iter()
            .map(|ik| format!("{}: {} to {}", ik.name, name_of(&self.scene, ik.tip), name_of(&self.scene, ik.target)))
            .collect();

        ui.separator();
        ui.strong("IK constraints");
        let mut remove = None;
        for (i, (ik, label)) in self.scene.ik.iter_mut().zip(&labels).enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(label);
                ui.label("Bones");
                ui.add(egui::DragValue::new(&mut ik.chain_length).range(1..=8));
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.scene.ik.remove(i);
        }
    }
}
