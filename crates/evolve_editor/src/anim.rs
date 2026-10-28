//! Animation state and commands: playback, auto-keying, and showing the pose of the current time.
//!
//! The scene always holds the *pose at the current time*: scrubbing writes the animated values
//! into the nodes, so the usual tools edit what you see. With Auto-key on, edits are recorded
//! as keyframes at the current time.

use crate::app::EditorApp;
use eframe::egui;
use evolve_engine::animation::{Clip, Property};
use evolve_engine::revision::Revs;
use evolve_engine::rig::Ik;
use evolve_engine::scene::Scene;
use std::sync::Arc;

/// What the pointer is doing in the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineDrag {
    Scrub,
    Key,
}

pub struct AnimState {
    /// The Timeline panel was on screen in the previous frame (Delete then removes the selected key).
    pub timeline_visible: bool,
    /// Index into `scene.clips`.
    pub clip: usize,
    /// Seconds.
    pub time: f32,
    pub playing: bool,
    pub looping: bool,
    /// Auto-key: record edits as keyframes.
    pub record: bool,
    /// `(track, key)` indices in the current clip.
    pub selected_key: Option<(usize, usize)>,
    pub drag: Option<TimelineDrag>,
    /// What was last written into the scene, to know when to do it again.
    applied: Option<(usize, f32, Clip)>,
    /// The scene as posed at the playhead while a clip plays. One copy of the scene is made when playback starts (and again when the
    /// scene, the clip or the IK constraints change) and posed anew in every frame; it is shared by the Scene and Game views. (A
    /// copy for each frame cost 12 to 14 ms on the big scenes, and to the GPU renderer every copy was another scene, to be walked
    /// from the start.)
    pub preview: Option<Arc<Scene>>,
    /// What `preview` was copied from.
    preview_of: Option<PreviewSource>,
}

/// What a preview copy stands for: the stamps of the scene, the clip that plays (its index and its content) and the IK constraints.
#[derive(Clone, PartialEq)]
struct PreviewSource {
    revs: Revs,
    clip: usize,
    content: Clip,
    ik: Vec<Ik>,
}

impl Default for AnimState {
    fn default() -> Self {
        Self {
            timeline_visible: false,
            clip: 0,
            time: 0.0,
            playing: false,
            looping: true,
            record: false,
            selected_key: None,
            drag: None,
            applied: None,
            preview: None,
            preview_of: None,
        }
    }
}

impl AnimState {
    /// Forget everything tied to the current document.
    pub fn reset(&mut self) {
        *self = Self { timeline_visible: self.timeline_visible, ..Self::default() };
    }
}

impl EditorApp {
    pub fn current_clip(&self) -> Option<&Clip> {
        self.scene.clips.get(self.anim.clip)
    }

    /// Index of the current clip, creating the first clip if there is none.
    pub fn ensure_clip(&mut self) -> usize {
        if self.scene.clips.is_empty() {
            self.scene.clips.push(Clip::new("Clip 1"));
        }
        self.anim.clip = self.anim.clip.min(self.scene.clips.len() - 1);
        self.anim.clip
    }

    pub fn new_clip(&mut self) {
        let name = format!("Clip {}", self.scene.clips.len() + 1);
        self.scene.clips.push(Clip::new(name));
        self.anim.clip = self.scene.clips.len() - 1;
        self.anim.time = 0.0;
        self.anim.selected_key = None;
    }

    /// Keys the given properties of the selected nodes at the current time.
    pub fn add_keyframes(&mut self, properties: &[Property]) {
        let clip = self.ensure_clip();
        let time = self.scene.clips[clip].snap_time(self.anim.time);
        let mut keys = Vec::new();
        for &id in &self.selection {
            let Some(node) = self.scene.find(id) else { continue };
            for &property in properties {
                if let Some(value) = property.read(node) {
                    keys.push((id, property, value));
                }
            }
        }
        if keys.is_empty() {
            self.status = "Nothing to key: select an object (only solid fills and strokes can be animated)".into();
            return;
        }
        for (id, property, value) in keys {
            self.scene.clips[clip].set_key(id, property, time, value.clone(), Some(value));
        }
        self.anim.selected_key = None;
    }

    pub fn delete_selected_key(&mut self) -> bool {
        let Some((track, key)) = self.anim.selected_key.take() else { return false };
        let Some(clip) = self.scene.clips.get_mut(self.anim.clip) else { return false };
        let Some(t) = clip.tracks.get_mut(track) else { return false };
        if key < t.keys.len() {
            t.keys.remove(key);
        }
        if t.keys.is_empty() {
            clip.tracks.remove(track);
        }
        true
    }

    /// Delete while the Timeline is on screen removes the selected keyframe instead of scene objects.
    pub fn delete_key_if_timeline(&mut self) -> bool {
        self.anim.timeline_visible && self.delete_selected_key()
    }

    /// A new copy of the scene showing the animation at the playhead (with IK solved). Playback draws a copy, so the real scene is never
    /// modified by it. (`update_preview` keeps one copy and poses it again; this is what that copy must look like.)
    #[cfg(test)]
    pub fn playback_scene(&self) -> Option<Scene> {
        if !self.anim.playing {
            return None;
        }
        let pose = self.current_clip()?.pose_at(&self.scene, self.anim.time);
        let mut posed = self.scene.clone();
        posed.apply_pose(&pose);
        posed.solve_ik();
        Some(posed)
    }

    /// Makes the preview for this frame (nothing unless a clip is playing): the copy of the scene is posed at the playhead again. The
    /// copy is made anew only when what it was made from changed. Every pose it is given sets the same properties of the same nodes,
    /// so nothing of an earlier pose is left in it, except the bones of IK chains, which are given back their transforms from the
    /// scene before the chains are solved again (the solver starts from where the bones are).
    pub fn update_preview(&mut self) {
        let Some(clip) = self.current_clip().filter(|_| self.anim.playing) else {
            self.anim.preview = None;
            self.anim.preview_of = None;
            return;
        };
        let source = PreviewSource { revs: self.scene.revision(), clip: self.anim.clip, content: clip.clone(), ik: self.scene.ik.clone() };
        let pose = clip.pose_at(&self.scene, self.anim.time);
        match self.anim.preview.as_mut() {
            Some(preview) if self.anim.preview_of.as_ref() == Some(&source) => {
                // The IK bones start from where the scene has them, as in a copy.
                self.scene.copy_ik_bones_to(Arc::make_mut(preview));
            }
            _ => {
                self.anim.preview = Some(Arc::new(self.scene.clone()));
                self.anim.preview_of = Some(source);
            }
        }
        let Some(preview) = self.anim.preview.as_mut() else { return };
        let preview = Arc::make_mut(preview);
        preview.apply_pose(&pose);
        preview.solve_ik();
    }

    pub fn toggle_play(&mut self) {
        let Some(clip) = self.current_clip() else { return };
        let duration = clip.duration;
        if self.anim.playing {
            self.anim.playing = false;
        } else {
            if self.anim.time >= duration {
                self.anim.time = 0.0;
            }
            self.anim.playing = true;
        }
    }

    pub fn stop_playback(&mut self) {
        self.anim.playing = false;
        self.anim.time = 0.0;
    }

    /// Moves the playhead while playing.
    pub fn advance_playback(&mut self, ctx: &egui::Context) {
        if !self.anim.playing {
            return;
        }
        let Some(duration) = self.current_clip().map(|c| c.duration.max(0.01)) else {
            self.anim.playing = false;
            return;
        };
        self.anim.time += ctx.input(|i| i.stable_dt);
        if self.anim.time >= duration {
            if self.anim.looping {
                self.anim.time %= duration;
            } else {
                self.anim.time = duration;
                self.anim.playing = false;
            }
        }
        ctx.request_repaint();
    }

    /// Records the edits made since the last undo step as keyframes (Auto-key).
    pub fn auto_key(&mut self) {
        if !self.anim.record || self.anim.playing {
            return;
        }
        let changes = self.scene.changes_since(self.history.committed());
        if changes.is_empty() {
            return;
        }
        let clip = self.ensure_clip();
        let time = self.scene.clips[clip].snap_time(self.anim.time);
        self.scene.clips[clip].record(&changes, time);
    }

    /// Writes the pose of the current time into the scene when the time or the clip changed.
    pub fn sync_pose(&mut self) {
        if self.anim.playing {
            return;
        }
        let Some(clip) = self.scene.clips.get(self.anim.clip).filter(|c| !c.tracks.is_empty()) else {
            self.anim.applied = None;
            return;
        };
        let up_to_date = self.anim.applied.as_ref().is_some_and(|(i, t, c)| *i == self.anim.clip && *t == self.anim.time && c == clip);
        if up_to_date {
            return;
        }
        let pose = clip.pose_at(&self.scene, self.anim.time);
        let snapshot = (self.anim.clip, self.anim.time, clip.clone());
        self.scene.apply_pose(&pose);
        // Showing another time is not an edit, so it must not show up in the undo history.
        self.history.edit_committed(|committed| committed.apply_pose(&pose));
        self.anim.applied = Some(snapshot);
    }
}
