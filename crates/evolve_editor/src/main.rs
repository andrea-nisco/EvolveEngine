// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod app;
mod assets;
mod autosave;
mod clipboard_ui;
mod dialogs;
mod export_ui;
mod canvas;
mod cli;
mod draw_tools;
mod freehand;
mod game;
mod game_view;
mod guides;
mod history;
mod inspector;
mod layout;
mod node_tool;
mod objects;
mod overlay;
mod panels;
mod rig_ui;
mod sample;
mod scenes_ui;
mod select_tool;
mod theme;
mod timeline;
mod tools;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cli::run(&args) {
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("EvolveEngine")
            .with_inner_size([1200.0, 680.0])
            .with_min_inner_size([900.0, 520.0]),
        ..Default::default()
    };
    // `evolve_editor --play` starts the game as soon as the editor opens.
    let autoplay = args.iter().any(|arg| arg == "--play");
    eframe::run_native(
        "EvolveEngine",
        options,
        Box::new(move |cc| {
            let mut editor = app::EditorApp::new(cc);
            if autoplay {
                editor.start_game();
            }
            Ok(Box::new(editor))
        }),
    )
}
