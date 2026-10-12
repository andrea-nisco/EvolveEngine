//! Browser entry point. `index.html` holds a canvas and the project as JSON in a script tag.

use crate::PlayerApp;
use evolve_engine::io;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

fn element(id: &str) -> Option<web_sys::Element> {
    web_sys::window()?.document()?.get_element_by_id(id)
}

/// Runs automatically when the WebAssembly module is loaded.
#[wasm_bindgen(start)]
pub fn start() {
    wasm_bindgen_futures::spawn_local(async {
        let Some(canvas) = element("evolve_canvas").and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            return;
        };
        let project = element("evolve-game").and_then(|e| e.text_content()).unwrap_or_default();
        let app = match io::parse_project(&project) {
            Ok(project) => PlayerApp::new(&project),
            Err(error) => {
                // The browser build has no fonts for egui to draw an error with, so use the page itself.
                if let Some(body) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.body()) {
                    body.set_inner_text(&format!("Could not load the game: {error}"));
                    let _ = body.set_attribute("style", "color:#f88;font-family:sans-serif;padding:1em");
                }
                return;
            }
        };
        let started = eframe::WebRunner::new().start(canvas, eframe::WebOptions::default(), Box::new(move |_cc| Ok(Box::new(app)))).await;
        if let Err(error) = started {
            web_sys::console::error_1(&error);
        }
    });
}
