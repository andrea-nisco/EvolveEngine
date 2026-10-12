//! The Objects panel: every object of the scene with an icon for its kind, a "+" menu that
//! creates objects, and a right-click menu on the empty space and on rows.

use crate::app::EditorApp;
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Sense, Stroke, StrokeKind};
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::path::Path;
use evolve_engine::scene::{Node, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Camera,
    Rectangle,
    Ellipse,
    Path,
    Group,
    Symbol,
    Bone,
}

/// Which icon an object gets. Shapes are told apart by their outline: four plain corners make
/// a rectangle, four curved points an ellipse, anything else a path.
pub fn icon_of(node: &Node) -> Icon {
    if node.bone.is_some() {
        return Icon::Bone;
    }
    match &node.kind {
        NodeKind::Camera(_) => Icon::Camera,
        NodeKind::Group(_) => Icon::Group,
        NodeKind::Instance(_) => Icon::Symbol,
        NodeKind::Shape(shape) => match &shape.path.subpaths[..] {
            [sub] if sub.closed && sub.anchors.len() == 4 => {
                let straight = sub.anchors.iter().all(|a| a.handle_in == Vec2::ZERO && a.handle_out == Vec2::ZERO);
                let round = sub.anchors.iter().all(|a| a.handle_in != Vec2::ZERO && a.handle_out != Vec2::ZERO);
                if straight {
                    Icon::Rectangle
                } else if round {
                    Icon::Ellipse
                } else {
                    Icon::Path
                }
            }
            _ => Icon::Path,
        },
    }
}

/// Draws a small line icon inside `rect`.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    // Positions are given in a 16 x 16 box.
    let at = |x: f32, y: f32| Pos2::new(rect.left() + x * rect.width() / 16.0, rect.top() + y * rect.height() / 16.0);
    let boxed = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    match icon {
        Icon::Camera => {
            painter.rect_stroke(boxed(1.0, 4.0, 11.0, 12.0), 1.5, stroke, StrokeKind::Inside);
            painter.add(egui::Shape::closed_line(vec![at(11.5, 7.0), at(15.0, 4.5), at(15.0, 11.5), at(11.5, 9.0)], stroke));
        }
        Icon::Rectangle => {
            painter.rect_stroke(boxed(2.0, 3.0, 14.0, 13.0), 0.5, stroke, StrokeKind::Inside);
        }
        Icon::Ellipse => {
            painter.circle_stroke(at(8.0, 8.0), rect.width() * 0.38, stroke);
        }
        Icon::Path => {
            painter.add(egui::Shape::line(vec![at(2.5, 12.5), at(5.0, 5.0), at(9.0, 4.0), at(11.0, 9.5), at(13.5, 8.0)], stroke));
            for p in [at(2.5, 12.5), at(13.5, 8.0)] {
                painter.rect_filled(Rect::from_center_size(p, egui::vec2(3.5, 3.5)), 0.0, color);
            }
        }
        Icon::Group => {
            painter.rect_stroke(boxed(1.5, 5.0, 10.0, 14.0), 0.5, stroke, StrokeKind::Inside);
            painter.rect_stroke(boxed(6.0, 2.0, 14.5, 10.5), 0.5, stroke, StrokeKind::Inside);
        }
        Icon::Symbol => {
            painter.add(egui::Shape::closed_line(vec![at(8.0, 1.5), at(14.5, 8.0), at(8.0, 14.5), at(1.5, 8.0)], stroke));
        }
        Icon::Bone => {
            painter.line_segment([at(3.5, 12.5), at(12.5, 3.5)], stroke);
            painter.circle_filled(at(3.5, 12.5), 2.2, color);
            painter.circle_stroke(at(12.5, 3.5), 2.2, stroke);
        }
    }
}

/// Things the create menu can add.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NewObject {
    Group,
    Rectangle,
    Ellipse,
    Camera,
}

/// How big a new rectangle or ellipse is, in world units.
const NEW_SHAPE_SIZE: f32 = 200.0;

impl EditorApp {
    /// The Objects panel: a "+" menu above the tree of all objects.
    pub fn objects_pane(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            self.scene_picker(ui);
            ui.menu_button("+ Add", |ui| self.create_menu(ui));
        });
        ui.separator();

        // Clicking the empty space deselects, and right-clicking it opens the create menu.
        // The background is registered first so the rows on top of it win the pointer.
        let background = ui.interact(ui.available_rect_before_wrap(), ui.id().with("objects_background"), Sense::click());
        egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]).show(ui, |ui| {
            self.scene_tree(ui);
        });
        if background.clicked() {
            self.selection.clear();
        }
        background.context_menu(|ui| self.create_menu(ui));
    }

    /// Menu content shared by the "+" button and the right-click menu.
    pub fn create_menu(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        for (label, what) in [("Empty group", NewObject::Group), ("Rectangle", NewObject::Rectangle), ("Ellipse", NewObject::Ellipse)] {
            if ui.button(label).clicked() {
                chosen = Some(what);
            }
        }
        let has_camera = self.scene.main_camera().is_some();
        let camera = ui.add_enabled(!has_camera, egui::Button::new("Camera")).on_disabled_hover_text("The scene already has a camera");
        if camera.clicked() {
            chosen = Some(NewObject::Camera);
        }
        ui.separator();
        if ui.button("Import SVG...").clicked() {
            ui.close();
            self.import_svg(false);
        }
        if ui.button("Import SVG as Symbol...").clicked() {
            ui.close();
            self.import_svg(true);
        }
        if let Some(what) = chosen {
            ui.close();
            self.create_object(what);
        }
    }

    /// Adds a new object in the middle of the editor's view and selects it.
    fn create_object(&mut self, what: NewObject) {
        let center = self.camera.center;
        let half = Vec2::splat(NEW_SHAPE_SIZE * 0.5);
        match what {
            NewObject::Rectangle => {
                self.add_shape_node(Path::rect(center - half, center + half), "Rectangle");
            }
            NewObject::Ellipse => {
                self.add_shape_node(Path::ellipse(center, half), "Ellipse");
            }
            NewObject::Group => {
                let mut node = self.scene.new_node("Empty", NodeKind::Group(Vec::new()));
                node.transform = Affine2::from_translation(center);
                self.selection = vec![self.scene.add(node, None)];
            }
            NewObject::Camera => self.add_camera(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::scene::{Scene, Shape};

    fn icon_for_path(path: Path) -> Icon {
        let mut scene = Scene::default();
        icon_of(&scene.new_node("n", NodeKind::Shape(Shape::new(path))))
    }

    #[test]
    fn every_kind_of_object_gets_its_own_icon() {
        assert_eq!(icon_for_path(Path::rect(Vec2::ZERO, Vec2::splat(10.0))), Icon::Rectangle);
        assert_eq!(icon_for_path(Path::ellipse(Vec2::ZERO, Vec2::splat(10.0))), Icon::Ellipse);
        assert_eq!(icon_for_path(Path::default()), Icon::Path);

        let mut scene = Scene::default();
        assert_eq!(icon_of(&scene.new_node("g", NodeKind::Group(Vec::new()))), Icon::Group);
        assert_eq!(icon_of(&scene.new_camera(Vec2::splat(100.0), Vec2::ZERO)), Icon::Camera);
        let mut bone = scene.new_node("b", NodeKind::Group(Vec::new()));
        bone.bone = Some(evolve_engine::rig::Bone { length: 50.0 });
        assert_eq!(icon_of(&bone), Icon::Bone);
    }
}