//! Desktop player. Starts the game appended to this executable, or the `.evolve` project given on
//! the command line, or asks for a project file.

#[cfg(target_arch = "wasm32")]
fn main() {} // the browser build uses the library's `start` function instead

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use evolve_engine::io;
    use evolve_player_core::{bundle, PlayerApp};
    use std::path::PathBuf;

    let loaded = std::env::current_exe()
        .ok()
        .and_then(|exe| bundle::read_embedded(&exe))
        .map(|json| io::parse_project(&json))
        .or_else(|| {
            let path = std::env::args().nth(1).map(PathBuf::from).or_else(|| {
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
        Some(Ok(project)) => PlayerApp::new(&project),
        Some(Err(error)) => PlayerApp::with_error(format!("Could not load the game: {error}")),
        None => PlayerApp::with_error("No game to play. Pass a .evolve file, or export one from the editor."),
    };
    eframe::run_native("Evolve Game", options, Box::new(move |_cc| Ok(Box::new(app))))
}
