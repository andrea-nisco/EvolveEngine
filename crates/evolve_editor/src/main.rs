// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
#[cfg(test)]
mod anim_tests;
mod app;
mod assets;
mod autosave;
mod bench;
mod bench_scene;
mod clipboard_ui;
mod dialogs;
#[cfg(test)]
mod edit_tests;
mod export_ui;
mod frame_limit;
mod frame_stats;
mod canvas;
mod cli;
mod draw_tools;
mod freehand;
mod game;
mod game_view;
mod guides;
mod history;
#[cfg(test)]
mod gpu_parity_tests;
#[cfg(test)]
mod gpu_view_tests;
#[cfg(test)]
mod idle_tests;
#[cfg(test)]
mod profile_tests;
mod inspector;
mod layout;
mod node_tool;
mod objects;
mod overlay;
mod panels;
mod renderers;
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

    let renderer = match renderers::RendererChoice::from_args(&args) {
        Ok(choice) => choice,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    let quality = match evolve_gpu::profile::Quality::from_args(&args) {
        Ok(quality) => quality,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    let options = native_options();
    // `evolve_editor --play` starts the game as soon as the editor opens.
    let autoplay = args.iter().any(|arg| arg == "--play");
    eframe::run_native(
        "EvolveEngine",
        options,
        Box::new(move |cc| {
            let mut editor = app::EditorApp::new(cc, renderer, quality);
            if autoplay {
                editor.start_game();
            }
            Ok(Box::new(editor))
        }),
    )
}

/// The window and the graphics setup. Windows present with `Fifo` (vsync): the card never draws faster than the screen
/// shows (GPU_SAFETY.md A1); `Mailbox` and `Immediate` are not frame caps.
fn native_options() -> eframe::NativeOptions {
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoVsync;
    eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("EvolveEngine")
            .with_inner_size([1200.0, 680.0])
            .with_min_inner_size([900.0, 520.0]),
        wgpu_options,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_present_with_vsync() {
        let present_mode = native_options().wgpu_options.surface.present_mode;
        assert_eq!(present_mode, eframe::wgpu::PresentMode::AutoVsync);
        assert!(
            !matches!(present_mode, eframe::wgpu::PresentMode::Immediate | eframe::wgpu::PresentMode::Mailbox),
            "those two do not limit the frame rate (GPU_SAFETY.md A1)"
        );
    }
}
