//! Tool definitions and the per-frame input handed to them.

use eframe::egui::Key;
use evolve_engine::geometry::{Affine2, Rect, Vec2};
use evolve_engine::path::Anchor;
use evolve_engine::scene::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Node,
    Pen,
    Rectangle,
    Ellipse,
    Pencil,
    Bone,
    Weights,
}

impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::Select,
        Tool::Node,
        Tool::Pen,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Pencil,
        Tool::Bone,
        Tool::Weights,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Node => "Node",
            Tool::Pen => "Pen",
            Tool::Rectangle => "Rect",
            Tool::Ellipse => "Ellipse",
            Tool::Pencil => "Pencil",
            Tool::Bone => "Bone",
            Tool::Weights => "Weights",
        }
    }

    pub fn key(self) -> Key {
        match self {
            Tool::Select => Key::V,
            Tool::Node => Key::A,
            Tool::Pen => Key::P,
            Tool::Rectangle => Key::R,
            Tool::Ellipse => Key::E,
            Tool::Pencil => Key::N,
            Tool::Bone => Key::B,
            Tool::Weights => Key::W,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Click to select, drag to move or box-select. Shift adds. Double-click enters a group.",
            Tool::Node => "Drag anchors and handles. Double-click a segment to add an anchor, an anchor to smooth it. Del removes.",
            Tool::Pen => "Click for corners, drag for curves. Click the first point to close. Enter finishes, Esc cancels.",
            Tool::Rectangle => "Drag to draw. Shift = square, Alt = from center.",
            Tool::Ellipse => "Drag to draw. Shift = circle, Alt = from center.",
            Tool::Pencil => "Drag to draw a freehand curve.",
            Tool::Bone => "Drag in empty space to add a bone, from a bone's tip to add a child. Drag a bone to aim it (Shift moves, Alt stretches).",
            Tool::Weights => "Select a skinned shape and paint the active bone's influence. Alt erases.",
        }
    }
}

/// Everything a tool needs to know about the pointer this frame.
/// Positions are in world space unless named `screen`.
#[derive(Clone, Debug, Default)]
pub struct CanvasInput {
    /// Current pointer position, if it is over the canvas or being dragged.
    pub pos: Option<Vec2>,
    /// Where the primary button went down.
    pub origin: Vec2,
    /// Pointer position relative to the canvas top-left, in screen points.
    pub screen: Vec2,
    pub screen_origin: Vec2,
    pub drag_started: bool,
    pub dragging: bool,
    pub drag_stopped: bool,
    pub clicked: bool,
    pub double_clicked: bool,
    pub shift: bool,
    pub alt: bool,
    /// Pick radius (5 screen points) in world units.
    pub tolerance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformKind {
    Move,
    /// Index into the eight bounding-box handles.
    Scale(usize),
    Rotate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnchorPart {
    Anchor,
    HandleIn,
    HandleOut,
}

/// How the opposite handle follows while one handle is dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mirror {
    Off,
    KeepLength(f32),
    EqualLength,
}

/// Identifies one anchor of one shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorRef {
    pub node: NodeId,
    pub sub: usize,
    pub index: usize,
}

/// The pointer interaction currently in progress.
#[derive(Clone, Debug, Default)]
pub enum Drag {
    #[default]
    None,
    Marquee { start: Vec2 },
    Transform {
        kind: TransformKind,
        start: Vec2,
        bounds: Rect,
        items: Vec<(NodeId, Affine2)>,
        /// Boxes of other objects (and the game screen) that a move can align to.
        targets: Vec<Rect>,
    },
    Anchor { target: AnchorRef, part: AnchorPart, mirror: Mirror },
    Shape { start: Vec2 },
    Freehand { points: Vec<Vec2> },
    PenHandle,
    /// Dragging out a new bone from `start`.
    BoneCreate { start: Vec2, parent: Option<NodeId> },
    /// Turning a bone to point at the pointer.
    BoneAim { id: NodeId },
    BoneMove { items: Vec<(NodeId, Affine2)>, start: Vec2 },
}

/// Pen tool state: the anchors placed so far, in world space.
pub type PenPath = Vec<Anchor>;
