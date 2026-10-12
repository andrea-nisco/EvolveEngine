//! Appearance of a shape: colors, gradients, fill and stroke settings.

use crate::geometry::{Affine2, Vec2};
use serde::{Deserialize, Serialize};

/// 8-bit RGBA color with straight (non-premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn with_alpha_scaled(self, factor: f32) -> Self {
        Self { a: (self.a as f32 * factor.clamp(0.0, 1.0)).round() as u8, ..self }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Position along the gradient, 0..=1.
    pub offset: f32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum GradientKind {
    Linear { start: Vec2, end: Vec2 },
    Radial { center: Vec2, radius: f32 },
}

/// A gradient defined in the shape's local space, optionally further transformed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<GradientStop>,
    pub transform: Affine2,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    Solid(Color),
    Gradient(Gradient),
}

impl Paint {
    /// Two-stop gradient from `a` to `b` across the given local-space line.
    pub fn linear(start: Vec2, end: Vec2, a: Color, b: Color) -> Self {
        Paint::Gradient(Gradient {
            kind: GradientKind::Linear { start, end },
            stops: vec![GradientStop { offset: 0.0, color: a }, GradientStop { offset: 1.0, color: b }],
            transform: Affine2::IDENTITY,
        })
    }

    /// Two-stop gradient from `a` (center) to `b` (edge).
    pub fn radial(center: Vec2, radius: f32, a: Color, b: Color) -> Self {
        Paint::Gradient(Gradient {
            kind: GradientKind::Radial { center, radius },
            stops: vec![GradientStop { offset: 0.0, color: a }, GradientStop { offset: 1.0, color: b }],
            transform: Affine2::IDENTITY,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f32,
    pub join: LineJoin,
    pub cap: LineCap,
    pub miter_limit: f32,
}

impl Stroke {
    pub fn solid(color: Color, width: f32) -> Self {
        Self { paint: Paint::Solid(color), width, join: LineJoin::Miter, cap: LineCap::Butt, miter_limit: 4.0 }
    }
}
