//! Dark theme following common editor conventions (neutral grays, one blue accent).

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(76, 154, 255);
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(19, 38, 64, 64);
pub const WORKSPACE: Color32 = Color32::from_rgb(30, 30, 30);
pub const PANEL: Color32 = Color32::from_rgb(37, 37, 38);
pub const HANDLE_FILL: Color32 = Color32::WHITE;
pub const GUIDE: Color32 = Color32::from_rgb(255, 120, 190);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(26, 26, 27);
    visuals.faint_bg_color = Color32::from_rgb(43, 43, 45);
    visuals.selection.bg_fill = Color32::from_rgb(38, 79, 120);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;

    let radius = CornerRadius::same(4);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 60, 62));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(55, 55, 58);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(50, 50, 53);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(68, 68, 72);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(62, 62, 66);
    visuals.widgets.active.bg_fill = Color32::from_rgb(38, 79, 120);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(38, 79, 120);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
    }
    // Always dark, whatever the OS theme is.
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.interact_size.y = 24.0;
    });
}
