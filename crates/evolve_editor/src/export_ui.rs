//! Exporting pictures: SVG, PNG and PNG sequences. A sequence is rendered on a worker thread, with
//! a progress window and a Cancel button, so the editor stays responsive.

use crate::app::EditorApp;
use crate::dialogs::{ImageExport, ImageKind};
use eframe::egui::{self, Align2};
use evolve_engine::export_image::{frame_count, render_artboard, scene_at, to_svg};
use evolve_engine::scene::Scene;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

fn stem(file: Option<&Path>) -> String {
    file.and_then(|p| p.file_stem()).map_or("drawing".into(), |s| s.to_string_lossy().into_owned())
}

/// How a sequence export ended.
struct SequenceResult {
    written: usize,
    cancelled: bool,
    failure: Option<String>,
}

/// A PNG sequence being rendered in the background.
pub struct SequenceJob {
    total: usize,
    folder: PathBuf,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<SequenceResult>,
}

/// What a sequence export needs, gathered before the thread starts.
struct SequenceSpec {
    scene: Scene,
    clip: usize,
    frames: usize,
    fps: f32,
    folder: PathBuf,
    base: String,
    scale: f32,
    transparent: bool,
}

fn render_sequence(spec: SequenceSpec, done: &AtomicUsize, cancel: &AtomicBool) -> SequenceResult {
    let mut result = SequenceResult { written: 0, cancelled: false, failure: None };
    for frame in 0..spec.frames {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }
        let scene = scene_at(&spec.scene, spec.clip, frame as f32 / spec.fps);
        let written = render_artboard(&scene, spec.scale, spec.transparent)
            .ok_or_else(|| "the picture is too large to render".to_string())
            .and_then(|pixmap| pixmap.save_png(spec.folder.join(format!("{}_{:04}.png", spec.base, frame + 1))).map_err(|e| e.to_string()));
        match written {
            Ok(()) => {
                result.written += 1;
                done.store(result.written, Ordering::Relaxed);
            }
            Err(e) => {
                result.failure = Some(e);
                break;
            }
        }
    }
    result
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
        if kind == ImageKind::Sequence {
            if self.dialogs.sequence_job.is_some() {
                self.status = "A PNG sequence is already being exported".into();
                return;
            }
            if self.current_clip().is_none() {
                self.status = "There is no animation clip to export. Create one in the Timeline tab first".into();
                return;
            }
        }
        self.dialogs.image_export = Some(ImageExport { kind, scale: 1.0, transparent: false });
    }

    /// Asks where to save, then renders and writes the pictures.
    pub fn run_image_export(&mut self) {
        let Some(options) = self.dialogs.image_export.as_ref() else { return };
        let (kind, scale, transparent) = (options.kind, options.scale, options.transparent);
        let base = stem(self.file.as_deref());

        match kind {
            ImageKind::Png => {
                let Some(path) = rfd::FileDialog::new().set_title("Export PNG").add_filter("PNG image", &["png"]).set_file_name(format!("{base}.png")).save_file() else {
                    return;
                };
                self.status = match render_artboard(&self.scene, scale, transparent).map(|pixmap| pixmap.save_png(&path)) {
                    Some(Ok(())) => format!("Exported {}", path.display()),
                    Some(Err(e)) => format!("Could not write the PNG: {e}"),
                    None => "The picture is too large to render".into(),
                };
            }
            ImageKind::Sequence => {
                let Some(folder) = rfd::FileDialog::new().set_title("Export PNG sequence: choose a folder").pick_folder() else { return };
                let Some(clip) = self.current_clip() else { return };
                let (frames, fps) = (frame_count(clip), clip.fps);
                let spec = SequenceSpec { scene: self.scene.clone(), clip: self.anim.clip, frames, fps, folder: folder.clone(), base, scale, transparent };
                let (done, cancel) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
                let (thread_done, thread_cancel) = (done.clone(), cancel.clone());
                let handle = std::thread::spawn(move || render_sequence(spec, &thread_done, &thread_cancel));
                self.dialogs.sequence_job = Some(SequenceJob { total: frames, folder, done, cancel, handle });
                self.status = format!("Exporting {frames} frames...");
            }
        }
        self.dialogs.image_export = None;
    }

    /// Shows the progress of a running sequence export and reports when it ends.
    pub fn sequence_export_dialog(&mut self, ctx: &egui::Context) {
        let Some(job) = self.dialogs.sequence_job.as_ref() else { return };
        if job.handle.is_finished() {
            let Some(job) = self.dialogs.sequence_job.take() else { return };
            let folder = job.folder.display().to_string();
            self.status = match job.handle.join() {
                Ok(SequenceResult { failure: Some(e), written, .. }) => format!("Stopped after {written} frames: {e}"),
                Ok(SequenceResult { cancelled: true, written, .. }) => format!("Cancelled after {written} frames (already written to {folder})"),
                Ok(SequenceResult { written, .. }) => format!("Exported {written} frames to {folder}"),
                Err(_) => "The PNG sequence export stopped unexpectedly".into(),
            };
            return;
        }
        let done = job.done.load(Ordering::Relaxed);
        let fraction = done as f32 / job.total.max(1) as f32;
        let text = format!("{done} / {} frames", job.total);
        let mut cancel = false;
        egui::Window::new("Exporting PNG sequence").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.add(egui::ProgressBar::new(fraction).text(text).desired_width(260.0));
            ui.add_space(8.0);
            cancel = ui.button("Cancel").clicked();
        });
        if cancel {
            job.cancel.store(true, Ordering::Relaxed);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
