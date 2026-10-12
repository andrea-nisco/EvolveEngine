//! Exporting pictures: SVG, PNG and PNG sequences.

use crate::app::EditorApp;
use crate::dialogs::{ImageExport, ImageKind};
use evolve_engine::export_image::{frame_count, render_artboard, scene_at, to_svg};
use std::path::Path;

fn stem(file: Option<&Path>) -> String {
    file.and_then(|p| p.file_stem()).map_or("drawing".into(), |s| s.to_string_lossy().into_owned())
}

impl EditorApp {
    pub fn export_svg_dialog(&mut self) {
        let name = format!("{}.svg", stem(self.file.as_deref()));
        let Some(path) = rfd::FileDialog::new().set_title("Export SVG").add_filter("SVG image", &["svg"]).set_file_name(name).save_file() else {
            return;
        };
        self.status = match std::fs::write(&path, to_svg(&self.scene)) {
            Ok(()) => format!("Exported {}", path.display()),
            Err(e) => format!("Could not export SVG: {e}"),
        };
    }

    pub fn begin_image_export(&mut self, kind: ImageKind) {
        if kind == ImageKind::Sequence && self.current_clip().is_none() {
            self.status = "There is no animation clip to export. Create one in the Timeline tab first".into();
            return;
        }
        self.dialogs.image_export = Some(ImageExport { kind, scale: 1.0, transparent: false });
    }

    /// Asks where to save, then renders and writes the pictures.
    pub fn run_image_export(&mut self) {
        let Some(options) = self.dialogs.image_export.as_ref() else { return };
        let (kind, scale, transparent) = (options.kind, options.scale, options.transparent);
        let base = stem(self.file.as_deref());

        let message = match kind {
            ImageKind::Png => {
                let Some(path) = rfd::FileDialog::new().set_title("Export PNG").add_filter("PNG image", &["png"]).set_file_name(format!("{base}.png")).save_file() else {
                    return;
                };
                match render_artboard(&self.scene, scale, transparent).map(|pixmap| pixmap.save_png(&path)) {
                    Some(Ok(())) => format!("Exported {}", path.display()),
                    Some(Err(e)) => format!("Could not write the PNG: {e}"),
                    None => "The picture is too large to render".into(),
                }
            }
            ImageKind::Sequence => {
                let Some(folder) = rfd::FileDialog::new().set_title("Export PNG sequence: choose a folder").pick_folder() else { return };
                let Some(clip) = self.current_clip() else { return };
                let (index, frames, fps) = (self.anim.clip, frame_count(clip), clip.fps);
                let mut written = 0;
                let mut failure = None;
                for frame in 0..frames {
                    let scene = scene_at(&self.scene, index, frame as f32 / fps);
                    let result = render_artboard(&scene, scale, transparent)
                        .ok_or_else(|| "the picture is too large to render".to_string())
                        .and_then(|pixmap| pixmap.save_png(folder.join(format!("{base}_{:04}.png", frame + 1))).map_err(|e| e.to_string()));
                    match result {
                        Ok(()) => written += 1,
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                match failure {
                    None => format!("Exported {written} frames to {}", folder.display()),
                    Some(e) => format!("Stopped after {written} frames: {e}"),
                }
            }
        };
        self.status = message;
        self.dialogs.image_export = None;
    }
}
