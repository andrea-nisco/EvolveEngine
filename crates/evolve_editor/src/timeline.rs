//! The Timeline tab: clip controls, tracks with keyframes, scrubbing, and the key inspector
//! with an easing curve editor.

use crate::anim::{AnimState, TimelineDrag};
use crate::app::EditorApp;
use crate::inspector::color_button;
use crate::theme;
use eframe::egui::scroll_area::ScrollSource;
use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, Sense, Shape as UiShape, Stroke};
use evolve_engine::animation::{Clip, Easing, Property, Value};
use evolve_engine::scene::Scene;

const LABEL_W: f32 = 170.0;
const ROW_H: f32 = 22.0;
const RULER_H: f32 = 22.0;
const KEY_RADIUS: f32 = 5.0;
const SIDE_W: f32 = 210.0;
const PLAYHEAD: Color32 = Color32::from_rgb(255, 96, 96);

impl EditorApp {
    pub fn timeline_contents(&mut self, ui: &mut egui::Ui) {
        if self.scene.clips.is_empty() {
            ui.horizontal(|ui| {
                ui.weak("No animation yet.");
                if ui.button("Create clip").clicked() {
                    self.new_clip();
                }
                ui.weak("Select an object and use Add keyframe, or turn on Auto-key and edit it.");
            });
            return;
        }
        self.anim.clip = self.anim.clip.min(self.scene.clips.len() - 1);
        self.timeline_toolbar(ui);
        ui.separator();

        // Work on a copy of the clip and write it back only if something changed.
        let index = self.anim.clip;
        let original = self.scene.clips[index].clone();
        let mut clip = original.clone();
        let height = ui.available_height();
        let main_width = (ui.available_width() - SIDE_W - 32.0).max(240.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui(egui::vec2(main_width, height), |ui| {
                ui.set_max_width(main_width);
                ui.vertical(|ui| track_area(ui, &mut self.anim, &mut clip, &self.scene));
            });
            ui.separator();
            ui.allocate_ui(egui::vec2(SIDE_W, height), |ui| {
                ui.set_max_width(SIDE_W);
                ui.vertical(|ui| key_inspector(ui, &self.anim, &mut clip, &self.scene));
            });
        });
        if clip != original {
            self.scene.clips[index] = clip;
        }
    }

    fn timeline_toolbar(&mut self, ui: &mut egui::Ui) {
        let has_selection = !self.selection.is_empty();
        ui.horizontal_wrapped(|ui| {
            let names: Vec<String> = self.scene.clips.iter().map(|c| c.name.clone()).collect();
            let index = self.anim.clip;
            egui::ComboBox::from_id_salt("clip_selector").selected_text(&names[index]).width(110.0).show_ui(ui, |ui| {
                for (i, name) in names.iter().enumerate() {
                    if ui.selectable_value(&mut self.anim.clip, i, name).clicked() {
                        self.anim.selected_key = None;
                    }
                }
            });
            if ui.button("+").on_hover_text("New clip").clicked() {
                self.new_clip();
                return;
            }
            ui.separator();

            if ui.button(if self.anim.playing { "Pause" } else { "Play" }).clicked() {
                self.toggle_play();
            }
            if ui.button("Stop").on_hover_text("Back to the start").clicked() {
                self.stop_playback();
            }
            ui.checkbox(&mut self.anim.looping, "Loop");
            ui.separator();

            {
                let clip = &mut self.scene.clips[index];
                let mut time = self.anim.time;
                ui.label("Time");
                let time_field = egui::DragValue::new(&mut time).speed(0.01).range(0.0..=clip.duration).fixed_decimals(2).suffix(" s");
                if ui.add(time_field).changed() {
                    self.anim.time = clip.snap_time(time);
                    self.anim.playing = false;
                }
                ui.label("Length");
                ui.add(egui::DragValue::new(&mut clip.duration).speed(0.05).range(0.1..=600.0).suffix(" s"));
                ui.label("FPS");
                ui.add(egui::DragValue::new(&mut clip.fps).range(1.0..=240.0));
                self.anim.time = self.anim.time.min(clip.duration);
            }
            ui.separator();

            let record = egui::RichText::new("Auto-key");
            let record = if self.anim.record { record.color(PLAYHEAD).strong() } else { record };
            ui.toggle_value(&mut self.anim.record, record)
                .on_hover_text("Record edits as keyframes at the current time");
            ui.add_enabled_ui(has_selection, |ui| {
                ui.menu_button("Add keyframe", |ui| {
                    let mut chosen = None;
                    if ui.button("Transform (position, rotation, scale)").clicked() {
                        chosen = Some(vec![Property::Position, Property::Rotation, Property::Scale]);
                    }
                    for property in Property::ALL {
                        if ui.button(property.label()).clicked() {
                            chosen = Some(vec![property]);
                        }
                    }
                    if let Some(properties) = chosen {
                        self.add_keyframes(&properties);
                        ui.close();
                    }
                });
            });
            if ui.add_enabled(self.anim.selected_key.is_some(), egui::Button::new("Delete key")).clicked() {
                self.delete_selected_key();
            }
        });
    }
}

/// Ruler step (seconds) that keeps tick labels at least 60 points apart.
fn ruler_step(pixels_per_second: f32) -> f32 {
    [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]
        .into_iter()
        .find(|s| s * pixels_per_second >= 60.0)
        .unwrap_or(60.0)
}

fn diamond(center: Pos2, radius: f32) -> Vec<Pos2> {
    vec![
        Pos2::new(center.x, center.y - radius),
        Pos2::new(center.x + radius, center.y),
        Pos2::new(center.x, center.y + radius),
        Pos2::new(center.x - radius, center.y),
    ]
}

fn track_area(ui: &mut egui::Ui, anim: &mut AnimState, clip: &mut Clip, scene: &Scene) {
    // Only tracks whose node still exists are listed.
    let rows: Vec<usize> = (0..clip.tracks.len()).filter(|&i| scene.find(clip.tracks[i].node).is_some()).collect();
    let content_height = (RULER_H + rows.len() as f32 * ROW_H).max(120.0);

    egui::ScrollArea::vertical()
        .id_salt("track_scroll")
        .scroll_source(ScrollSource::MOUSE_WHEEL | ScrollSource::SCROLL_BAR)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let size = egui::vec2(ui.available_width(), content_height);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            let x0 = rect.left() + LABEL_W;
            let width = (rect.right() - 12.0 - x0).max(50.0);
            let pixels_per_second = width / clip.duration.max(0.01);
            let time_to_x = |t: f32| x0 + t * pixels_per_second;
            let x_to_time = |x: f32| (x - x0) / pixels_per_second;
            let row_top = |r: usize| rect.top() + RULER_H + r as f32 * ROW_H;

            // Ruler.
            let step = ruler_step(pixels_per_second);
            let muted = ui.visuals().weak_text_color();
            let mut tick = 0.0;
            while tick <= clip.duration + 1e-3 {
                let x = time_to_x(tick);
                painter.line_segment([Pos2::new(x, rect.top() + RULER_H - 6.0), Pos2::new(x, rect.top() + RULER_H)], Stroke::new(1.0, muted));
                painter.text(Pos2::new(x + 3.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, format!("{tick:.2}s"), egui::FontId::proportional(10.0), muted);
                tick += step;
            }
            painter.line_segment([Pos2::new(x0, rect.top() + RULER_H), Pos2::new(rect.right(), rect.top() + RULER_H)], Stroke::new(1.0, Color32::from_white_alpha(25)));

            // Rows with their keyframes.
            for (r, &ti) in rows.iter().enumerate() {
                let track = &clip.tracks[ti];
                let row = Rect::from_min_size(Pos2::new(rect.left(), row_top(r)), egui::vec2(rect.width(), ROW_H));
                if r % 2 == 0 {
                    painter.rect_filled(row, 0.0, Color32::from_white_alpha(6));
                }
                let name = scene.find(track.node).map_or("?", |n| n.name.as_str());
                painter.with_clip_rect(Rect::from_min_size(row.min, egui::vec2(LABEL_W - 6.0, ROW_H))).text(
                    Pos2::new(rect.left() + 8.0, row.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{name} · {}", track.property.label()),
                    egui::FontId::proportional(12.0),
                    ui.visuals().text_color(),
                );
                for (ki, key) in track.keys.iter().enumerate() {
                    let selected = anim.selected_key == Some((ti, ki));
                    let points = diamond(Pos2::new(time_to_x(key.time), row.center().y), KEY_RADIUS);
                    let (fill, outline) = if selected { (Color32::WHITE, theme::ACCENT) } else { (theme::ACCENT, Color32::from_black_alpha(160)) };
                    painter.add(UiShape::convex_polygon(points, fill, Stroke::new(1.0, outline)));
                }
            }

            // Playhead.
            let head_x = time_to_x(anim.time.clamp(0.0, clip.duration));
            painter.line_segment([Pos2::new(head_x, rect.top()), Pos2::new(head_x, rect.bottom())], Stroke::new(1.5, PLAYHEAD));
            painter.add(UiShape::convex_polygon(
                vec![Pos2::new(head_x - 5.0, rect.top()), Pos2::new(head_x + 5.0, rect.top()), Pos2::new(head_x, rect.top() + 8.0)],
                PLAYHEAD,
                Stroke::NONE,
            ));

            // Input: press on a key selects (and drags) it, press anywhere else scrubs.
            let pointer = ui.input(|i| i.pointer.latest_pos());
            let started = response.drag_started_by(PointerButton::Primary);
            if started || response.clicked_by(PointerButton::Primary) {
                let press = if started { ui.input(|i| i.pointer.press_origin()) } else { pointer };
                if let Some(p) = press {
                    let hit = rows.iter().enumerate().find_map(|(r, &ti)| {
                        let y = row_top(r) + ROW_H / 2.0;
                        let near = |k: &evolve_engine::animation::Keyframe| (time_to_x(k.time) - p.x).abs() <= KEY_RADIUS + 2.0 && (y - p.y).abs() <= ROW_H / 2.0;
                        clip.tracks[ti].keys.iter().position(near).map(|ki| (ti, ki))
                    });
                    if let Some(key) = hit {
                        anim.selected_key = Some(key);
                        anim.drag = Some(TimelineDrag::Key);
                    } else if p.x >= x0 - 4.0 {
                        anim.selected_key = None;
                        anim.drag = Some(TimelineDrag::Scrub);
                        anim.playing = false;
                        anim.time = clip.snap_time(x_to_time(p.x));
                    }
                }
                if !started {
                    anim.drag = None; // a plain click has no drag phase
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                if let (Some(p), Some(mode)) = (pointer, anim.drag) {
                    let wanted = clip.snap_time(x_to_time(p.x));
                    match (mode, anim.selected_key) {
                        (TimelineDrag::Scrub, _) => anim.time = wanted,
                        (TimelineDrag::Key, Some((ti, ki))) => move_key(clip, ti, ki, wanted),
                        _ => {}
                    }
                }
            }
            if response.drag_stopped() {
                anim.drag = None;
            }
        });
}

/// Moves a key to `time`, keeping it between its neighbours (one frame apart at least).
fn move_key(clip: &mut Clip, track: usize, key: usize, time: f32) {
    let frame = 1.0 / clip.fps;
    let duration = clip.duration;
    let Some(t) = clip.tracks.get_mut(track) else { return };
    let lo = if key > 0 { t.keys[key - 1].time + frame } else { 0.0 };
    let hi = if key + 1 < t.keys.len() { t.keys[key + 1].time - frame } else { duration };
    if let Some(k) = t.keys.get_mut(key) {
        k.time = time.clamp(lo, hi.max(lo));
    }
}

fn key_inspector(ui: &mut egui::Ui, anim: &AnimState, clip: &mut Clip, scene: &Scene) {
    let selected = anim.selected_key.filter(|&(t, k)| clip.tracks.get(t).is_some_and(|tr| k < tr.keys.len()));
    let Some((ti, ki)) = selected else {
        ui.weak("Select a keyframe to edit its time, value and easing.");
        return;
    };
    let (frame, duration) = (1.0 / clip.fps, clip.duration);
    let track = &mut clip.tracks[ti];
    let name = scene.find(track.node).map_or("?", |n| n.name.as_str());
    ui.strong(format!("{name} · {}", track.property.label()));

    let lo = if ki > 0 { track.keys[ki - 1].time + frame } else { 0.0 };
    let hi = if ki + 1 < track.keys.len() { track.keys[ki + 1].time - frame } else { duration };
    let has_next = ki + 1 < track.keys.len();
    let property = track.property;
    let key = &mut track.keys[ki];

    ui.horizontal(|ui| {
        ui.label("Time");
        ui.add(egui::DragValue::new(&mut key.time).speed(0.01).range(lo..=hi.max(lo)).fixed_decimals(2).suffix(" s"));
    });
    value_editor(ui, property, &mut key.value);

    if has_next {
        ui.add_space(4.0);
        ui.label("Easing to next key");
        ui.horizontal_wrapped(|ui| {
            for (label, preset) in Easing::PRESETS {
                let active = key.easing == preset;
                if ui.add(egui::Button::new(label).selected(active).small()).clicked() {
                    key.easing = preset;
                }
            }
        });
        easing_editor(ui, &mut key.easing);
    }
}

fn value_editor(ui: &mut egui::Ui, property: Property, value: &mut Value) {
    match value {
        Value::Vec2(v) => {
            let speed = if property == Property::Scale { 0.01 } else { 0.5 };
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(egui::DragValue::new(&mut v.x).speed(speed));
                ui.label("Y");
                ui.add(egui::DragValue::new(&mut v.y).speed(speed));
            });
        }
        Value::Float(f) if property == Property::Rotation => {
            let mut degrees = f.to_degrees();
            ui.horizontal(|ui| {
                ui.label("Angle");
                if ui.add(egui::DragValue::new(&mut degrees).speed(0.5).suffix("°")).changed() {
                    *f = degrees.to_radians();
                }
            });
        }
        Value::Float(f) => {
            ui.add(egui::Slider::new(f, 0.0..=1.0).text("Opacity"));
        }
        Value::Color(c) => {
            ui.horizontal(|ui| {
                ui.label("Color");
                color_button(ui, c);
            });
        }
        Value::Path(path) => {
            let anchors: usize = path.subpaths.iter().map(|s| s.anchors.len()).sum();
            ui.weak(format!("Outline with {anchors} anchors. It morphs into the next shape key."));
        }
    }
}

/// Cubic-Bézier easing graph: drag the two handles to shape the curve.
fn easing_editor(ui: &mut egui::Ui, easing: &mut Easing) {
    const SIZE: f32 = 150.0;
    const Y_MIN: f32 = -0.3;
    const Y_MAX: f32 = 1.3;
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(SIZE), Sense::hover());
    let painter = ui.painter_at(rect.expand(8.0));
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_white_alpha(30)), egui::StrokeKind::Inside);

    let to_screen = |x: f32, y: f32| Pos2::new(rect.left() + x * SIZE, rect.bottom() - (y - Y_MIN) / (Y_MAX - Y_MIN) * SIZE);
    let (origin, end) = (to_screen(0.0, 0.0), to_screen(1.0, 1.0));
    painter.line_segment([origin, end], Stroke::new(1.0, Color32::from_white_alpha(25)));

    let curve: Vec<Pos2> = (0..=48).map(|i| i as f32 / 48.0).map(|t| to_screen(t, easing.apply(t))).collect();
    painter.add(UiShape::line(curve, Stroke::new(2.0, theme::ACCENT)));

    let handles = [(&mut easing.x1, &mut easing.y1, origin, "h1"), (&mut easing.x2, &mut easing.y2, end, "h2")];
    for (x, y, anchor, id) in handles {
        let point = to_screen(*x, *y);
        painter.line_segment([anchor, point], Stroke::new(1.0, Color32::from_white_alpha(90)));
        let response = ui.interact(Rect::from_center_size(point, egui::Vec2::splat(16.0)), ui.id().with(id), Sense::drag());
        if response.dragged() {
            let delta = response.drag_delta();
            *x = (*x + delta.x / SIZE).clamp(0.0, 1.0);
            *y = (*y - delta.y / SIZE * (Y_MAX - Y_MIN)).clamp(Y_MIN, Y_MAX);
        }
        let hot = response.hovered() || response.dragged();
        painter.circle_filled(point, if hot { 6.0 } else { 5.0 }, Color32::WHITE);
        painter.circle_stroke(point, if hot { 6.0 } else { 5.0 }, Stroke::new(1.5, theme::ACCENT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::animation::{Keyframe, Track};
    use evolve_engine::scene::NodeId;

    #[test]
    fn ruler_labels_stay_readable() {
        assert!(ruler_step(10.0) * 10.0 >= 60.0);
        assert_eq!(ruler_step(1000.0), 0.1);
    }

    #[test]
    fn moving_a_key_cannot_pass_its_neighbours() {
        let key = |time| Keyframe { time, value: Value::Float(0.0), easing: Easing::LINEAR };
        let mut clip = Clip::new("c");
        clip.tracks.push(Track { node: NodeId(1), property: Property::Opacity, keys: vec![key(0.0), key(1.0), key(2.0)] });
        move_key(&mut clip, 0, 1, 5.0);
        assert!(clip.tracks[0].keys[1].time < 2.0);
        move_key(&mut clip, 0, 1, -5.0);
        assert!(clip.tracks[0].keys[1].time > 0.0);
    }
}
