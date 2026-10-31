//! Soft shadows without a blur: a few rounded rectangles, each a little larger and fainter than the next.
//!
//! A real shadow is a rectangle blurred with a Gaussian of deviation `sigma`; seen across its edge, its opacity is
//! `0.5 * erfc(d / (sigma * sqrt 2))` where `d` is the distance outside the edge (negative inside). Rectangles stacked from the
//! outermost to the innermost give the same opacity in steps: each layer is given the alpha that makes the layers drawn so far
//! add up to the profile in the band where it is the top one. The steps are 2 * `blur` / [`LAYERS`] wide, a fifth of a pixel to a
//! few pixels for any usual shadow; the corners get the right rounding by growing the radii with the layer.

use evolve_ui_core::{Point, Rect};

/// How many rectangles make a shadow. More is smoother and costs more shapes; at 10 the steps stay under one level of 255 in
/// opacity for the blurs of a normal UI (4 to 32 pixels).
pub(crate) const LAYERS: usize = 10;

/// One rectangle of a shadow and the opacity it is drawn with (0 to 1, to be multiplied by the shadow color's own alpha).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layer {
    pub rect: Rect,
    pub radii: [f32; 4],
    pub alpha: f32,
}

/// The complementary error function (Abramowitz and Stegun 7.1.26, error under 1.5e-7).
fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t * (0.254_829_6 + t * (-0.284_496_74 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let erfc_positive = poly * (-z * z).exp();
    if x >= 0.0 {
        erfc_positive
    } else {
        2.0 - erfc_positive
    }
}

/// The opacity of a Gaussian-blurred edge at signed distance `d` outside it, for a blur whose reach is `blur` (sigma = blur / 2).
pub(crate) fn profile(d: f32, blur: f32) -> f32 {
    let sigma = (blur / 2.0).max(1e-4);
    0.5 * erfc(d / (sigma * std::f32::consts::SQRT_2))
}

/// The layers of a shadow, outermost first. Numbers that are not finite give no shadow; a blur of under half a pixel gives one
/// hard-edged layer.
pub(crate) fn layers(rect: Rect, radii: [f32; 4], offset: Point, blur: f32, spread: f32) -> Vec<Layer> {
    let finite = [rect.x, rect.y, rect.w, rect.h, offset.x, offset.y, blur, spread].iter().all(|v| v.is_finite()) && radii.iter().all(|r| r.is_finite());
    if !finite || rect.w < 0.0 || rect.h < 0.0 {
        return Vec::new();
    }
    let blur = blur.clamp(0.0, 1000.0);
    let base = Rect::new(rect.x + offset.x, rect.y + offset.y, rect.w, rect.h);
    let layer_at = |grow: f32, alpha: f32| -> Option<Layer> {
        let r = Rect::new(base.x - grow, base.y - grow, base.w + 2.0 * grow, base.h + 2.0 * grow);
        (r.w > 0.0 && r.h > 0.0 && alpha > 0.0).then(|| Layer { rect: r, radii: radii.map(|v| (v.max(0.0) + grow).max(0.0)), alpha })
    };
    if blur < 0.5 {
        return layer_at(spread, 1.0).into_iter().collect();
    }
    let step = 2.0 * blur / LAYERS as f32;
    let mut layers = Vec::with_capacity(LAYERS);
    let mut before = 0.0f32; // the opacity the layers drawn so far give
    for k in 0..LAYERS {
        let edge = blur - k as f32 * step;
        // The opacity wanted in the band where this layer is the topmost; the innermost one reaches full strength.
        let wanted = if k + 1 == LAYERS { 1.0 } else { profile(edge - step / 2.0, blur) };
        let wanted = wanted.max(before);
        let alpha = if before >= 1.0 - 1e-6 { 0.0 } else { 1.0 - (1.0 - wanted) / (1.0 - before) };
        before = wanted;
        layers.extend(layer_at(edge + spread, alpha.clamp(0.0, 1.0)));
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_the_gaussian_edge() {
        assert!((profile(0.0, 10.0) - 0.5).abs() < 1e-4, "half way at the edge");
        assert!(profile(-10.0, 10.0) > 0.97 && profile(10.0, 10.0) < 0.03, "two sigma (the blur's reach) inside is nearly full, outside nearly nothing");
        assert!(profile(-15.0, 10.0) > 0.998 && profile(15.0, 10.0) < 0.002);
        // Known value: one sigma outside (d = 5 for a blur of 10) is 0.1587.
        assert!((profile(5.0, 10.0) - 0.158_655).abs() < 1e-3);
        assert!((profile(-5.0, 10.0) + profile(5.0, 10.0) - 1.0).abs() < 1e-4, "symmetric around the edge");
    }

    fn stacked_opacity(layers: &[Layer], d: f32, rect: Rect) -> f32 {
        // The opacity at distance `d` outside the straight middle of the top edge: layers whose rectangle contains the point.
        let y = rect.y - d;
        let x = rect.x + rect.w / 2.0;
        let mut open = 1.0f32;
        for l in layers {
            if x >= l.rect.x && x < l.rect.right() && y >= l.rect.y && y < l.rect.bottom() {
                open *= 1.0 - l.alpha;
            }
        }
        1.0 - open
    }

    #[test]
    fn stacked_layers_follow_the_profile_within_a_few_percent() {
        let rect = Rect::new(100.0, 100.0, 200.0, 120.0);
        for blur in [6.0f32, 16.0, 32.0] {
            let layers = layers(rect, [0.0; 4], Point::ZERO, blur, 0.0);
            assert_eq!(layers.len(), LAYERS);
            for i in -20..=20 {
                let d = i as f32 * blur / 20.0;
                let (got, want) = (stacked_opacity(&layers, d, rect), profile(d, blur));
                assert!((got - want).abs() < 0.08, "blur {blur}, d {d}: {got} against {want}");
            }
            assert!(stacked_opacity(&layers, -blur * 1.01, rect) > 0.999, "full strength inside");
            assert!(stacked_opacity(&layers, blur * 1.01, rect) == 0.0, "nothing past the reach");
        }
    }

    #[test]
    fn layers_get_larger_outwards_and_the_corners_get_rounder() {
        let layers = layers(Rect::new(0.0, 0.0, 100.0, 60.0), [4.0; 4], Point::ZERO, 20.0, 0.0);
        assert!(layers.windows(2).all(|w| w[0].rect.w > w[1].rect.w && w[0].radii[0] >= w[1].radii[0]), "each layer is inside the one before");
        assert!(layers[0].radii[0] > 4.0, "even a corner with a radius gets rounder as it blurs");
        assert!(layers[layers.len() - 1].radii[0] < 4.0, "and sharper again inside the edge");
    }

    #[test]
    fn offset_moves_and_spread_grows_the_shadow() {
        let plain = layers(Rect::new(0.0, 0.0, 100.0, 60.0), [0.0; 4], Point::ZERO, 8.0, 0.0);
        let moved = layers(Rect::new(0.0, 0.0, 100.0, 60.0), [0.0; 4], Point::new(3.0, 5.0), 8.0, 0.0);
        let grown = layers(Rect::new(0.0, 0.0, 100.0, 60.0), [0.0; 4], Point::ZERO, 8.0, 2.0);
        assert_eq!((moved[0].rect.x - plain[0].rect.x, moved[0].rect.y - plain[0].rect.y), (3.0, 5.0));
        assert_eq!(grown[0].rect.w - plain[0].rect.w, 4.0);
    }

    #[test]
    fn a_tiny_blur_is_one_hard_layer_and_bad_numbers_give_nothing() {
        let hard = layers(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4], Point::new(1.0, 1.0), 0.0, 0.0);
        assert_eq!(hard.len(), 1);
        assert_eq!(hard[0].alpha, 1.0);
        assert!(layers(Rect::new(0.0, 0.0, f32::NAN, 10.0), [0.0; 4], Point::ZERO, 8.0, 0.0).is_empty());
        assert!(layers(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4], Point::ZERO, f32::INFINITY, 0.0).is_empty());
        assert!(layers(Rect::new(0.0, 0.0, -5.0, 10.0), [0.0; 4], Point::ZERO, 8.0, 0.0).is_empty());
    }

    #[test]
    fn a_huge_blur_is_clamped() {
        let layers = layers(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4], Point::ZERO, 1.0e9, 0.0);
        assert!(layers.len() <= LAYERS && layers.iter().all(|l| l.rect.w <= 2100.0), "reach limited to 1000 pixels");
    }
}
