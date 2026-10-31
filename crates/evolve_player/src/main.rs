//! Desktop player. Starts the game appended to this executable (or stored in its macOS `.app` bundle), or the
//! `.evolve` project given on the command line, or asks for a project file.

#[cfg(target_arch = "wasm32")]
fn main() {} // the browser build uses the library's `start` function instead

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use evolve_engine::io;
    use evolve_player_core::{bundle, PlayerApp};
    use std::path::PathBuf;

    // Arguments that are not options: the project to play.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let project_arg = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from);
    let ui_demo = args.iter().any(|a| a == "--ui-demo");
    #[cfg(not(feature = "ui-demo"))]
    if ui_demo {
        eprintln!("--ui-demo needs a player built with the `ui-demo` feature: cargo run -p evolve_player --features ui-demo -- <project> --ui-demo");
    }

    let loaded = std::env::current_exe()
        .ok()
        .and_then(|exe| bundle::read_project_for(&exe))
        .map(|json| io::parse_project(&json))
        .or_else(|| {
            let path = project_arg.or_else(|| {
                rfd::FileDialog::new().set_title("Open a Evolve project").add_filter("Evolve project", &["evolve"]).pick_file()
            })?;
            Some(io::load_project(&path))
        });

    // Open at the game's own size, shrunk to fit a typical screen.
    let (width, height) = match &loaded {
        Some(Ok(project)) => {
            let size = project.scenes[project.start].camera_view().size;
            let shrink = (1280.0 / size.x).min(800.0 / size.y).min(1.0);
            (size.x * shrink, size.y * shrink)
        }
        _ => (640.0, 360.0),
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title("Evolve Game").with_inner_size([width, height]),
        ..Default::default()
    };
    let app = match loaded {
        Some(Ok(project)) => {
            let app = PlayerApp::new(&project);
            #[cfg(feature = "ui-demo")]
            let app = if ui_demo { app.with_ui(Box::new(evolve_ui_scene::demo::UiDemo::new())) } else { app };
            #[cfg(not(feature = "ui-demo"))]
            let _ = ui_demo;
            app
        }
        Some(Err(error)) => PlayerApp::with_error(format!("Could not load the game: {error}")),
        None => PlayerApp::with_error("No game to play. Pass a .evolve file, or export one from the editor."),
    };
    eframe::run_native("Evolve Game", options, Box::new(move |_cc| Ok(Box::new(app))))
}
