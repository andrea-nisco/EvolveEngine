//! Editor state, commands and the top-level frame layout.

use crate::anim::AnimState;
use crate::assets::AssetsState;
use crate::canvas::RenderTarget;
use crate::renderers::{RendererChoice, RendererSettings};
use evolve_gpu::profile::Quality;
use crate::autosave::{self, Recovery};
use crate::dialogs::{DialogState, Dialogs, Pending};
use crate::game::{GameView, PlayState};
use crate::guides::Guide;
use crate::history::History;
use crate::layout::{self, Pane};
use crate::rig_ui::RigState;
use crate::scenes_ui::ScenesState;
use crate::tools::{AnchorRef, Drag, PenPath, Tool};
use crate::{sample, theme};
use eframe::egui::{self, Key, Modifiers};
use egui_dock::DockState;
use evolve_engine::boolean::BoolOp;
use evolve_engine::camera::Camera;
use evolve_engine::clipboard::Clipboard;
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::io;
use evolve_engine::path::Path;
use evolve_engine::project::Project;
use evolve_engine::runtime::LogLine;
use evolve_player_core::bundle;
use evolve_engine::scene::{NodeId, NodeKind, Scene, Shape};
use std::collections::HashSet;
use std::path::PathBuf;

/// The least of the scene (as a share of its largest side; about 5 km on a map of the world, where the median border is 2 km long, a province some 200 km and 74% of the objects are under 26 km) that the pin of the Objects panel shows around an object.
pub(crate) const LOCATE_CONTEXT: f32 = 0.0005;

pub struct ViewSettings {
    pub show_grid: bool,
    pub snap_to_grid: bool,
    pub smart_guides: bool,
    pub grid_size: f32,
    /// The frame times in the status bar.
    pub show_frame_times: bool,
}

/// State owned by the viewports: cached render targets and the interaction in progress.
pub struct CanvasState {
    /// Where the Scene view and the Game view are rasterized.
    pub scene_target: RenderTarget,
    pub game_target: RenderTarget,
    /// Pointer position in world space while it is over the Game view (what the game sees as its mouse), and while a drag with the
    /// right button that began over it goes on outside it.
    pub game_cursor: Option<Vec2>,
    /// The right button went down over the Game view and is still down.
    pub game_drag: bool,
    pub drag: Drag,
    pub pen: PenPath,
    pub anchor: Option<AnchorRef>,
    pub cursor: Option<Vec2>,
    pub guides: Vec<Guide>,
    pub needs_frame: bool,
    /// The last scroll input came from a trackpad (or other pixel-precise device), not a mouse wheel.
    pub trackpad_scroll: bool,
}

#[derive(Default)]
pub struct TreeState {
    pub collapsed: HashSet<NodeId>,
    pub renaming: Option<NodeId>,
}

pub struct EditorApp {
    /// The scene being edited. The project below holds all scenes, but its entry for this one is
    /// out of date until sync_open_scene copies it back.
    pub scene: Scene,
    pub project: Project,
    /// Index of the scene being edited in project.
    pub open: usize,
    pub scenes_ui: ScenesState,
    pub camera: Camera,
    pub selection: Vec<NodeId>,
    pub tool: Tool,
    pub history: History,
    /// Which renderer draws the Scene and Game views, and the graphics device the GPU renderer would use.
    pub renderers: RendererSettings,
    pub frame_stats: crate::frame_stats::FrameStats,
    /// The cap on the frame rate while a game runs or a clip plays (View -> Frame rate).
    pub frame_limit: crate::frame_limit::FrameLimit,
    pub pacer: crate::frame_limit::Pacer,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    pub view: ViewSettings,
    pub canvas: CanvasState,
    pub tree: TreeState,
    pub assets: AssetsState,
    pub anim: AnimState,
    pub rig: RigState,
    /// The running game, if Play mode is on.
    pub play: Option<PlayState>,
    pub game_view: GameView,
    /// Console output of the last game run.
    pub last_log: Vec<LogLine>,
    /// Modal dialogs currently open.
    pub dialogs: Dialogs,
    pub dialog_state: DialogState,
    /// The last thing copied inside the editor (it survives New / Open, so it can be pasted into another document).
    pub clipboard: Option<Clipboard>,
    /// How many times the current copy was pasted (each paste shifts a little further).
    pub paste_count: usize,
    pub status: String,
    /// The dockable panels. Taken out while they are drawn (see `show_panes`).
    pub dock: Option<DockState<Pane>>,
    /// The layout as last written to disk.
    pub saved_layout: String,
    name_counter: u32,
    shown_title: String,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, renderer: RendererChoice, quality: Quality) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut app = Self::from_project(sample::project(), sample::OPEN_AT_START);
        app.renderers = RendererSettings::new(renderer, cc.wgpu_render_state.as_ref());
        app.renderers.set_quality(quality);
        if renderer != RendererChoice::Cpu {
            app.status = match app.renderers.adapter() {
                Some(adapter) => format!("{renderer} renderer requested; graphics card: {adapter}"),
                None => format!("{renderer} renderer requested, but there is no usable graphics card: using the CPU renderer"),
            };
        }
        app
    }

    /// An editor with `project` loaded and scene `open` being edited. (Separate from `new` so that tests can build
    /// one without a window.)
    pub(crate) fn from_project(project: Project, open: usize) -> Self {
        let scene = project.scenes[open].clone();
        Self {
            history: History::new(&scene),
            renderers: RendererSettings::without_gpu(RendererChoice::Cpu),
            frame_stats: Default::default(),
            frame_limit: crate::frame_limit::FrameLimit::Display,
            pacer: Default::default(),
            scene,
            project,
            open,
            scenes_ui: ScenesState::default(),
            camera: Camera::new(Vec2::new(1000.0, 700.0)),
            selection: Vec::new(),
            tool: Tool::Select,
            file: None,
            dirty: false,
            view: ViewSettings { show_grid: true, snap_to_grid: false, smart_guides: true, grid_size: 20.0, show_frame_times: true },
            canvas: CanvasState {
                scene_target: RenderTarget::default(),
                game_target: RenderTarget::default(),
                game_cursor: None,
                game_drag: false,
                drag: Drag::None,
                pen: Vec::new(),
                anchor: None,
                cursor: None,
                guides: Vec::new(),
                needs_frame: true,
                trackpad_scroll: false,
            },
            tree: TreeState::default(),
            assets: AssetsState::default(),
            anim: AnimState::default(),
            rig: RigState::default(),
            play: None,
            game_view: GameView::default(),
            last_log: Vec::new(),
            // A recovery copy left behind by a crash is offered at startup.
            dialogs: Dialogs { recovery: autosave::read(), ..Dialogs::default() },
            dialog_state: DialogState::default(),
            clipboard: None,
            paste_count: 0,
            status: "Ready".into(),
            dock: Some(layout::load()),
            saved_layout: String::new(),
            name_counter: 0,
            shown_title: String::new(),
        }
    }

    // --- Helpers -------------------------------------------------------------------------------

    pub fn snap(&self, p: Vec2) -> Vec2 {
        if self.view.snap_to_grid {
            (p / self.view.grid_size).round() * self.view.grid_size
        } else {
            p
        }
    }

    fn new_name(&mut self, prefix: &str) -> String {
        self.name_counter += 1;
        format!("{prefix} {}", self.name_counter)
    }

    /// Adds a shape built from `path` (world coordinates) on top of the scene and selects it.
    /// Open paths get no fill, since a fill on an open line is rarely wanted.
    pub fn add_shape_node(&mut self, path: Path, prefix: &str) -> NodeId {
        let mut shape = Shape::new(path);
        if shape.path.subpaths.iter().any(|s| !s.closed) {
            shape.fill = None;
        }
        let name = self.new_name(prefix);
        let node = self.scene.new_node(name, NodeKind::Shape(shape));
        let id = self.scene.add(node, None);
        // Rotation and scale (and their animation) pivot around the origin, so put it in the middle.
        self.scene.center_origin(id);
        self.selection = vec![id];
        self.canvas.anchor = None;
        id
    }

    pub fn center_origin_selection(&mut self) {
        for id in self.scene.top_level_selection(&self.selection) {
            self.scene.center_origin(id);
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        if self.tool == Tool::Pen && tool != Tool::Pen {
            self.finish_pen(false);
        }
        self.tool = tool;
        self.canvas.drag = Drag::None;
        self.canvas.anchor = None;
    }

    // --- Document commands ---------------------------------------------------------------------

    /// Replaces the whole document, opening the project's first scene.
    fn load_project(&mut self, project: Project, file: Option<PathBuf>) {
        self.scene = project.scenes[0].clone();
        self.open = 0;
        self.project = project;
        self.file = file;
        self.reset_editing_state();
        self.dirty = false;
    }

    /// Forgets everything tied to the scene that was open: selection, undo history, tool state.
    pub fn reset_editing_state(&mut self) {
        self.history.reset(&self.scene);
        self.selection.clear();
        self.canvas.drag = Drag::None;
        self.canvas.pen.clear();
        self.canvas.anchor = None;
        self.canvas.needs_frame = true;
        self.assets.reset();
        self.anim.reset();
    }

    pub fn new_document(&mut self) {
        self.load_project(Project::new(), None);
        autosave::clear();
        self.status = "New document".into();
    }

    /// Continues work from a crash-recovery copy. It counts as unsaved.
    pub fn recover(&mut self, recovery: Recovery) {
        self.load_project(recovery.project, recovery.original);
        self.dirty = true;
        self.status = "Recovered your unsaved work. Save it to keep it".into();
    }

    pub fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new().add_filter("Evolve project", &["evolve"]).pick_file();
        if let Some(path) = picked {
            match io::load_project(&path) {
                Ok(project) => {
                    self.status = format!("Opened {}", path.display());
                    self.load_project(project, Some(path));
                    autosave::clear();
                }
                Err(e) => self.status = format!("Could not open file: {e}"),
            }
        }
    }

    pub fn save(&mut self) {
        match self.file.clone() {
            Some(path) => self.save_to(path),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let picked = rfd::FileDialog::new().add_filter("Evolve project", &["evolve"]).set_file_name("untitled.evolve").save_file();
        if let Some(path) = picked {
            self.save_to(path.with_extension("evolve"));
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        self.sync_open_scene();
        match io::save_project(&self.project, &path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.file = Some(path);
                self.dirty = false;
                autosave::clear();
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    /// Imports an SVG as a group, or (with `as_symbol`) as a reusable symbol with one instance placed.
    pub fn import_svg(&mut self, as_symbol: bool) {
        let Some(path) = rfd::FileDialog::new().add_filter("SVG image", &["svg"]).pick_file() else { return };
        let name = path.file_stem().map_or("SVG".into(), |s| s.to_string_lossy().into_owned());
        let result = std::fs::read(&path).map_err(Into::into).and_then(|data| io::import_svg(&mut self.scene, &data, &name));
        match result {
            Ok(id) => {
                self.selection = vec![id];
                self.status = format!("Imported {}", path.display());
                if as_symbol {
                    self.create_symbol_from_selection();
                }
            }
            Err(e) => self.status = format!("Could not import SVG: {e}"),
        }
    }

    // --- Export --------------------------------------------------------------------------------

    /// Saves a standalone game: the player program with this project built into it.
    pub fn export_desktop_dialog(&mut self) {
        let Some(player) = bundle::find_player_exe() else {
            self.status = "The player is missing next to the editor. Build it with: cargo build --release -p evolve_player".into();
            return;
        };
        let default_name = if cfg!(windows) {
            "game.exe"
        } else if cfg!(target_os = "macos") {
            "game.app"
        } else {
            "game"
        };
        let Some(output) = rfd::FileDialog::new().set_title("Export game").set_file_name(default_name).save_file() else { return };
        self.sync_open_scene();
        let result = io::project_to_string(&self.project).and_then(|json| bundle::export_desktop(&player, &json, &output));
        self.status = match result {
            Ok(()) => format!("Exported {}. It runs on its own: no editor or project file needed", output.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    /// Saves a folder with `index.html` that plays the game in a browser.
    pub fn export_web_dialog(&mut self) {
        let Some(player_dir) = bundle::find_web_player_dir() else {
            self.status = "The web player is not built yet. Run tools/build-web-player.ps1 (Windows) or .sh (macOS, Linux) once".into();
            return;
        };
        let Some(folder) = rfd::FileDialog::new().set_title("Export for web: choose a folder").pick_folder() else { return };
        self.sync_open_scene();
        let result = io::project_to_string(&self.project).and_then(|json| bundle::export_web(&player_dir, &json, &folder));
        self.status = match result {
            Ok(()) => format!("Exported to {}. Serve the folder over http to play (browsers block file:// modules)", folder.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    // --- Edit commands -------------------------------------------------------------------------

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.scene) {
            self.dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.scene) {
            self.dirty = true;
        }
    }

    pub fn select_all(&mut self) {
        let pickable = |n: &&evolve_engine::scene::Node| n.visible && !n.locked && !matches!(n.kind, NodeKind::Camera(_));
        self.selection = self.scene.nodes.iter().filter(pickable).map(|n| n.id).collect();
    }

    pub fn delete_selection(&mut self) {
        if self.tool == Tool::Node && self.delete_selected_anchor() {
            return;
        }
        self.remove_selected();
    }

    /// Removes the selected objects, except the scene's only camera (which stays selected).
    /// Returns false if something was kept for that reason.
    pub fn remove_selected(&mut self) -> bool {
        let removable = self.scene.deletable(&self.selection);
        let kept_camera = removable.len() < self.scene.top_level_selection(&self.selection).len();
        self.scene.remove_all(&removable);
        self.scene.retain_existing(&mut self.selection);
        if kept_camera {
            self.notify("A scene needs a camera, so it was not deleted");
        }
        !kept_camera
    }

    pub fn duplicate_selection(&mut self) {
        let offset = Affine2::from_translation(Vec2::splat(self.view.grid_size.max(10.0)));
        let sources = self.scene.top_level_selection(&self.selection);
        let mut copies = Vec::new();
        for id in sources {
            if let Some(copy) = self.scene.duplicate(id) {
                if let Some(mut node) = self.scene.find_mut(copy) {
                    let name = format!("{} copy", node.name);
                    *node.name_mut() = name;
                }
                let start = self.scene.world_transform(copy).unwrap_or_default();
                self.apply_world_delta(&[(copy, start)], offset);
                copies.push(copy);
            }
        }
        if !copies.is_empty() {
            self.selection = copies;
        }
    }

    pub fn group_selection(&mut self) {
        if let Some(group) = self.scene.group(&self.selection) {
            self.selection = vec![group];
        }
    }

    pub fn ungroup_selection(&mut self) {
        let mut freed = Vec::new();
        for id in self.scene.top_level_selection(&self.selection) {
            freed.extend(self.scene.ungroup(id));
        }
        if !freed.is_empty() {
            self.selection = freed;
        }
    }

    pub fn combine_selection(&mut self) {
        if let Some(id) = self.scene.combine(&self.selection) {
            self.selection = vec![id];
        }
    }

    pub fn boolean_selection(&mut self, op: BoolOp) {
        match self.scene.boolean(&self.selection, op) {
            Some(id) => self.selection = vec![id],
            None => self.status = "Select two or more shapes first".into(),
        }
    }

    pub fn release_compound_selection(&mut self) {
        let released: Vec<NodeId> =
            self.scene.top_level_selection(&self.selection).into_iter().flat_map(|id| self.scene.release_compound(id)).collect();
        if !released.is_empty() {
            self.selection = released;
        }
    }

    pub fn make_clip_selection(&mut self) {
        match self.scene.make_clip_group(&self.selection) {
            Some(id) => self.selection = vec![id],
            None => self.status = "Select two or more objects; the top-most one must be a shape".into(),
        }
    }

    pub fn release_clip_selection(&mut self) {
        for &id in &self.selection {
            if let Some(mut node) = self.scene.find_mut(id) {
                node.set_clip(false);
            }
        }
    }

    pub fn shift_selection_order(&mut self, up: bool) {
        let mut ids = self.scene.top_level_selection(&self.selection);
        if up {
            ids.reverse(); // move the topmost first so neighbours do not block each other
        }
        for id in ids {
            self.scene.shift_order(id, up);
        }
    }

    // --- View commands -------------------------------------------------------------------------

    /// Fits the game camera's frame into the view.
    pub fn frame_camera(&mut self) {
        let frame = self.scene.camera_view().bounds();
        self.camera.frame(frame.center(), frame.size());
    }

    pub fn frame_selection(&mut self) {
        let bounds = self.selection_bounds();
        match bounds {
            Some(b) => self.camera.frame(b.center(), b.size().max(Vec2::splat(1.0)) * 1.2),
            None => self.frame_camera(),
        }
    }

    /// Takes the Scene view to the object `id` (the pin in the Objects panel) and selects it. The view shows it with some of what is
    /// around it: never less than half a thousandth of the scene's own size, or a thin border or a speck would fill the view with nothing
    /// that can be recognised.
    pub fn locate(&mut self, id: NodeId) {
        self.canvas.anchor = None;
        self.selection = vec![id];
        let Some(bounds) = self.scene.selection_bounds(&self.selection) else { return self.frame_camera() };
        let whole = self.scene.selection_bounds(&self.scene.all_ids()).map_or(0.0, |all| all.size().max_element());
        let size = (bounds.size() * 1.2).max(Vec2::splat((whole * LOCATE_CONTEXT).max(1.0)));
        self.camera.frame(bounds.center(), size);
        // What it did, in words, so that a surprising view can be told about exactly.
        let name = self.scene.find(id).map_or(String::new(), |node| node.name.clone());
        let shown = self.camera.viewport / self.camera.zoom;
        self.status = format!(
            "Pin: {name}: {:.1} x {:.1}, view {:.0} x {:.0} (at least {:.0}), zoom {:.3}",
            bounds.size().x, bounds.size().y, shown.x, shown.y, whole * LOCATE_CONTEXT, self.camera.zoom
        );
    }

    // --- Frame ---------------------------------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Keys belong to a text field being edited, or to a question waiting for an answer.
        if ctx.egui_wants_keyboard_input() || self.scenes_ui.confirm_delete.is_some() {
            return;
        }
        let (cmd, cmd_shift) = (Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT);
        let pressed = |mods: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(mods, key));

        if pressed(Modifiers::SHIFT, Key::F5) {
            if self.play.is_none() {
                self.start_game_from_start();
            }
            return;
        }
        if pressed(Modifiers::NONE, Key::F5) {
            self.toggle_game();
            return;
        }
        if pressed(Modifiers::NONE, Key::F6) {
            self.toggle_game_pause();
            return;
        }
        if self.play.is_some() {
            return; // keys belong to the game
        }

        self.handle_clipboard_events(ctx);

        if pressed(cmd, Key::N) {
            self.request(Pending::New);
        } else if pressed(cmd, Key::O) {
            self.request(Pending::Open);
        } else if pressed(cmd_shift, Key::S) {
            self.save_as();
        } else if pressed(cmd, Key::S) {
            self.save();
        } else if pressed(cmd_shift, Key::Z) || pressed(cmd, Key::Y) {
            self.redo();
        } else if pressed(cmd, Key::Z) {
            self.undo();
        } else if pressed(cmd_shift, Key::G) {
            self.ungroup_selection();
        } else if pressed(cmd, Key::G) {
            self.group_selection();
        } else if pressed(cmd, Key::D) {
            self.duplicate_selection();
        } else if pressed(cmd_shift, Key::J) {
            self.release_compound_selection();
        } else if pressed(cmd, Key::J) {
            self.combine_selection();
        } else if pressed(Modifiers::COMMAND | Modifiers::ALT, Key::Num7) {
            self.release_clip_selection();
        } else if pressed(cmd, Key::Num7) {
            self.make_clip_selection();
        } else if pressed(Modifiers::NONE, Key::X) {
            self.split_selected_anchor();
        } else if pressed(cmd, Key::A) {
            self.select_all();
        } else if pressed(Modifiers::NONE, Key::Delete) || pressed(Modifiers::NONE, Key::Backspace) {
            if !self.delete_key_if_timeline() {
                self.delete_selection();
            }
        } else if pressed(Modifiers::NONE, Key::OpenBracket) {
            self.shift_selection_order(false);
        } else if pressed(Modifiers::NONE, Key::CloseBracket) {
            self.shift_selection_order(true);
        } else if pressed(Modifiers::NONE, Key::Enter) {
            self.finish_pen(false);
        } else if pressed(Modifiers::NONE, Key::Escape) {
            self.cancel_pen();
            self.canvas.drag = Drag::None;
        } else if pressed(Modifiers::NONE, Key::Home) {
            self.frame_camera();
        } else if pressed(Modifiers::NONE, Key::F) {
            self.frame_selection();
        } else if let Some(tool) = Tool::ALL.into_iter().find(|t| pressed(Modifiers::NONE, t.key())) {
            self.set_tool(tool);
        }
    }

    /// Whether the editor asks for a new frame as soon as one is done: a game runs, or a clip plays.
    pub(crate) fn draws_continuously(&self) -> bool {
        self.play.is_some() || self.anim.playing
    }

    /// Bookkeeping at the end of each frame: undo history, stale selection, window title.
    pub(crate) fn after_frame(&mut self, ctx: &egui::Context) {
        // An edit in progress (mouse held, or typing into a text field) is committed when it ends.
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.egui_wants_keyboard_input();
        // Derived state first (animation pose, then IK), so only the user's own edits remain as
        // differences from the last undo step.
        self.sync_pose();
        self.solve_rig();
        if !busy {
            self.auto_key(); // keys are part of the same undo step as the edit that caused them
        }
        if self.history.update(&self.scene, busy) {
            self.dirty = true;
        }
        self.autosave_tick(ctx, busy);
        self.save_layout_if_changed(busy);
        self.scene.retain_existing(&mut self.selection);
        if self.canvas.anchor.is_some_and(|a| !self.anchor_exists(a)) {
            self.canvas.anchor = None;
        }

        let name = self.file.as_ref().and_then(|p| p.file_name()).map_or("Untitled".into(), |n| n.to_string_lossy());
        let title = format!("{}{} - EvolveEngine", name, if self.dirty { "*" } else { "" });
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_stats.begin();
        if let Some(message) = self.renderers.poll() {
            self.status = message; // a lost device or a reported card error: the views go on with the CPU renderer
        }
        self.handle_shortcuts(&ctx);
        self.anim.timeline_visible = false; // the Timeline panel sets it again while it is drawn
        self.advance_playback(&ctx);
        self.update_preview();
        self.update_game(&ctx);

        self.menu_bar(ui);
        self.status_bar(ui);
        self.toolbar(ui);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::WORKSPACE)).show(ui, |ui| self.show_panes(ui));

        self.show_dialogs(&ctx);
        self.watch_window_close(&ctx);
        self.after_frame(&ctx);
        self.frame_stats.end();
        // Drawing every frame anyway: a running game, a playing clip. Then the cap holds the next frame back.
        self.pacer.end_of_frame(self.frame_limit, self.draws_continuously());
    }
}
