//! Cut, copy and paste. Copies also go to the system clipboard as text, so they can be pasted
//! into another editor window. Pasting SVG text imports the drawing.

use crate::app::EditorApp;
use eframe::egui;
use evolve_engine::clipboard::Clipboard;
use evolve_engine::geometry::Vec2;
use evolve_engine::io;

impl EditorApp {
    pub fn copy_selection(&mut self, ctx: &egui::Context) -> bool {
        let Some(clipboard) = self.scene.copy_nodes(&self.selection) else {
            self.status = "Nothing selected to copy".into();
            return false;
        };
        if let Some(text) = clipboard.to_text() {
            ctx.copy_text(text);
        }
        self.clipboard = Some(clipboard);
        self.paste_count = 0;
        self.status = "Copied".into();
        true
    }

    pub fn cut_selection(&mut self, ctx: &egui::Context) {
        if self.copy_selection(ctx) && self.remove_selected() {
            self.status = "Cut".into();
        }
    }

    /// Pastes `text` from the system clipboard if it holds copied objects or an SVG drawing,
    /// otherwise the editor's own last copy. `in_place` keeps the original position.
    pub fn paste(&mut self, text: Option<&str>, in_place: bool) {
        if let Some(svg) = text.filter(|t| t.trim_start().starts_with("<svg")) {
            match io::import_svg(&mut self.scene, svg.as_bytes(), "Pasted SVG") {
                Ok(id) => {
                    self.selection = vec![id];
                    self.status = "Pasted SVG drawing".into();
                }
                Err(e) => self.status = format!("Could not paste SVG: {e}"),
            }
            return;
        }
        let clipboard = text.and_then(Clipboard::from_text).or_else(|| self.clipboard.clone());
        let Some(clipboard) = clipboard else {
            self.status = "Nothing to paste".into();
            return;
        };
        let step = self.view.grid_size.max(10.0);
        let offset = if in_place { Vec2::ZERO } else { Vec2::splat(step * (self.paste_count + 1) as f32) };
        self.paste_count += 1;
        let pasted = self.scene.paste_nodes(&clipboard, offset);
        self.canvas.anchor = None;
        self.selection = pasted;
        self.status = match clipboard.skinned_shapes() {
            0 => "Pasted".into(),
            n => format!("Pasted. {n} skinned shape(s) lost their skin: bind the copies to bones again"),
        };
    }

    /// Handles the cut / copy / paste events egui produces for Ctrl+X / C / V.
    pub fn handle_clipboard_events(&mut self, ctx: &egui::Context) {
        let events: Vec<egui::Event> = ctx
            .input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))).cloned().collect());
        let shift = ctx.input(|i| i.modifiers.shift);
        for event in events {
            match event {
                egui::Event::Copy => {
                    self.copy_selection(ctx);
                }
                egui::Event::Cut => self.cut_selection(ctx),
                egui::Event::Paste(text) => self.paste(Some(&text), shift),
                _ => {}
            }
        }
    }
}
