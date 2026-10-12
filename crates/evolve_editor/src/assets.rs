//! Asset browser: the project's symbols (prefabs) with thumbnails.
//! Cards can be dragged onto the canvas, or placed with a double click.

use crate::app::EditorApp;
use crate::theme;
use eframe::egui::{self, Sense};
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::render;
use evolve_engine::scene::NodeId;
use evolve_engine::symbol::{Symbol, SymbolId};
use std::collections::HashMap;

/// Thumbnail resolution in pixels (shown at half that size in points, for sharp HiDPI).
const THUMB_PIXELS: u32 = 128;
const CARD_SIZE: egui::Vec2 = egui::vec2(88.0, 100.0);

#[derive(Default)]
pub struct AssetsState {
    pub selected: Option<SymbolId>,
    thumbs: HashMap<SymbolId, egui::TextureHandle>,
    /// The symbols the thumbnails were rendered from; they are redrawn when this differs from the scene.
    thumb_source: Vec<Symbol>,
}

impl AssetsState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

impl EditorApp {
    // --- Commands ------------------------------------------------------------------------------

    pub fn create_symbol_from_selection(&mut self) {
        let name = match self.selection[..] {
            [id] => self.scene.find(id).map(|n| n.name.clone()),
            _ => None,
        }
        .unwrap_or_else(|| format!("Symbol {}", self.scene.symbols.len() + 1));
        match self.scene.make_symbol(&self.selection, &name) {
            Some((symbol, instance)) => {
                self.selection = vec![instance];
                self.assets.selected = Some(symbol);
                self.status = format!("Created symbol \"{name}\"; the selection is now an instance of it");
            }
            None => self.status = "Select something to turn into a symbol".into(),
        }
    }

    pub fn place_symbol(&mut self, symbol: SymbolId, at: Vec2) {
        if let Some(instance) = self.scene.place_instance(symbol, at) {
            self.selection = vec![instance];
        }
    }

    fn update_symbol_from_selection(&mut self, symbol: SymbolId) {
        match self.scene.update_symbol(symbol, &self.selection) {
            Some(instance) => {
                self.selection = vec![instance];
                self.status = "Symbol updated; every instance now shows the new artwork".into();
            }
            None => self.status = "Select the artwork to use (a symbol can't contain itself)".into(),
        }
    }

    /// Puts an editable copy of the master on the canvas. Edit it, then use "Update from selection".
    pub fn edit_master(&mut self, symbol: SymbolId) {
        let Some(mut copy) = self.scene.symbol_copy(symbol) else { return };
        copy.transform = Affine2::from_translation(self.camera.center);
        let id = self.scene.add(copy, None);
        self.selection = vec![id];
        self.assets.selected = Some(symbol);
        self.status = "Edit the copy, keep it selected, then choose \"Update from selection\" in Assets".into();
    }

    pub fn detach_selected_instances(&mut self) {
        let detached: Vec<NodeId> = self.selection.iter().filter_map(|&id| self.scene.detach_instance(id)).collect();
        if !detached.is_empty() {
            self.selection = detached;
        }
    }

    // --- Thumbnails ----------------------------------------------------------------------------

    fn refresh_thumbnails(&mut self, ctx: &egui::Context) {
        if self.assets.thumb_source == self.scene.symbols {
            return;
        }
        self.assets.thumb_source = self.scene.symbols.clone();
        self.assets.thumbs.clear();
        for symbol in &self.scene.symbols {
            if let Some(pixmap) = render::thumbnail(&self.scene, symbol, THUMB_PIXELS) {
                let size = [THUMB_PIXELS as usize; 2];
                let image = egui::ColorImage::from_rgba_premultiplied(size, pixmap.data());
                let texture = ctx.load_texture(format!("symbol-{}", symbol.id.0), image, egui::TextureOptions::LINEAR);
                self.assets.thumbs.insert(symbol.id, texture);
            }
        }
    }

    // --- Panel ---------------------------------------------------------------------------------

    /// The Assets tab of the bottom panel.
    pub fn assets_contents(&mut self, ui: &mut egui::Ui) {
        self.refresh_thumbnails(ui.ctx());
        if self.assets.selected.is_some_and(|id| self.scene.symbol(id).is_none()) {
            self.assets.selected = None;
        }
        self.assets_toolbar(ui);
        ui.separator();
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let symbols: Vec<(SymbolId, String)> = self.scene.symbols.iter().map(|s| (s.id, s.name.clone())).collect();
                if symbols.is_empty() {
                    ui.weak("No symbols yet. Select artwork and choose Object > Create Symbol from Selection.");
                }
                for (id, name) in symbols {
                    self.asset_card(ui, id, &name);
                }
            });
        });
    }

    fn assets_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.selection.is_empty(), egui::Button::new("New symbol from selection")).clicked() {
                self.create_symbol_from_selection();
            }
            if ui.button("Import SVG as symbol...").clicked() {
                self.import_svg(true);
            }
            let Some(id) = self.assets.selected else { return };
            ui.separator();
            let uses = self.scene.symbol_usage(id);
            if ui.button("Place").clicked() {
                self.place_symbol(id, self.camera.center);
            }
            if ui.add_enabled(!self.selection.is_empty(), egui::Button::new("Update from selection")).clicked() {
                self.update_symbol_from_selection(id);
            }
            if ui.button("Edit master").clicked() {
                self.edit_master(id);
            }
            let delete = ui.add_enabled(uses == 0, egui::Button::new("Delete"));
            if delete.on_disabled_hover_text("Still used by instances").clicked() {
                self.scene.delete_symbol(id);
            }
            if let Some(symbol) = self.scene.symbols.iter_mut().find(|s| s.id == id) {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut symbol.name).desired_width(120.0));
            }
            ui.weak(format!("{uses} instance{}", if uses == 1 { "" } else { "s" }));
        });
    }

    fn asset_card(&mut self, ui: &mut egui::Ui, id: SymbolId, name: &str) {
        let (rect, response) = ui.allocate_exact_size(CARD_SIZE, Sense::click_and_drag());
        response.dnd_set_drag_payload(id);
        let selected = self.assets.selected == Some(id);
        if response.clicked() {
            self.assets.selected = Some(id);
        }
        if response.double_clicked() {
            self.place_symbol(id, self.camera.center);
        }

        let painter = ui.painter();
        let fill = if selected {
            theme::ACCENT_SOFT
        } else if response.hovered() {
            ui.visuals().widgets.hovered.weak_bg_fill
        } else {
            ui.visuals().faint_bg_color
        };
        painter.rect_filled(rect, 4.0, fill);
        if selected {
            painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
        }
        let thumb = egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 6.0), egui::Vec2::splat(THUMB_PIXELS as f32 / 2.0));
        if let Some(texture) = self.assets.thumbs.get(&id) {
            painter.image(texture.id(), thumb, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        let label = painter.layout_no_wrap(name.to_string(), egui::FontId::proportional(12.0), ui.visuals().text_color());
        let label_pos = egui::pos2(rect.center().x - label.size().x.min(CARD_SIZE.x - 8.0) / 2.0, rect.bottom() - 22.0);
        painter.with_clip_rect(rect.shrink(4.0)).galley(label_pos, label, ui.visuals().text_color());
        response.on_hover_text("Drag onto the canvas to place, or double-click");
    }
}
