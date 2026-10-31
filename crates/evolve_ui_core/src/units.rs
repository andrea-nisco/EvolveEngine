//! Units and screens (EVOLVE_UI_PLAN.md 4.4, ADR 0005).
//!
//! The UI is drawn in *logical* pixels. The size of a logical pixel on the real screen follows from the screen's own size, the
//! reference screen the UI was designed for, a *match mode* that says which side of the reference counts, and the user's own UI
//! scale (accessibility, game settings). The density of the display (DPI) needs no factor of its own: the screen's size is given in
//! physical pixels, which already hold it, so a 1280 x 720 window on a screen of twice the density (2560 x 1440 pixels) gets
//! exactly twice the scale of the same window on an ordinary screen, and the UI keeps its proportions and gets sharper.

/// The screen a UI is designed for, in logical pixels.
pub const REFERENCE_SIZE: (f32, f32) = (1920.0, 1080.0);

/// The smallest and largest scale of a logical pixel: a screen of no size or a user scale that is nonsense must not give a UI of
/// no size or of an unbounded one.
const SCALE_RANGE: (f32, f32) = (0.01, 100.0);
const USER_SCALE_RANGE: (f32, f32) = (0.25, 4.0);

/// Which side of the reference screen decides the scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchMode {
    /// The width decides: the reference width always fits, the height is whatever remains.
    Width,
    /// The height decides.
    Height,
    /// The smaller of the two ratios: the whole reference screen always fits, so nothing of it is ever cut. The default.
    #[default]
    Min,
    /// The larger of the two ratios: the reference screen covers the whole screen and an edge of it may fall outside.
    Expand,
}

/// Distances from the edges of the screen, in the unit of whoever holds them (physical pixels for the screen, logical for the
/// canvas).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Insets {
    pub const ZERO: Insets = Insets { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };

    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self { left, top, right, bottom }
    }

    fn scaled(self, factor: f32) -> Insets {
        Insets::new(self.left * factor, self.top * factor, self.right * factor, self.bottom * factor)
    }
}

/// What the UI needs to know about the screen it is shown on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenMetrics {
    /// The size of the screen (or of the window, or of the view the game is shown in), in physical pixels.
    pub pixels: (f32, f32),
    /// The screen the UI was designed for.
    pub reference: (f32, f32),
    pub mode: MatchMode,
    /// The player's own scale of the UI, 1 being as designed (0.25 to 4).
    pub user_scale: f32,
    /// What the system keeps for itself (a notch, the bars of the system), in physical pixels. A real phone gives it; on a desktop
    /// it is for simulating one.
    pub safe_area: Insets,
}

impl ScreenMetrics {
    /// A screen of `pixels` with the defaults of the plan: a reference of 1920 x 1080, match mode `Min`, no user scale, no safe area.
    pub fn new(pixels: (f32, f32)) -> Self {
        Self { pixels, reference: REFERENCE_SIZE, mode: MatchMode::Min, user_scale: 1.0, safe_area: Insets::ZERO }
    }

    pub fn with_mode(mut self, mode: MatchMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_user_scale(mut self, user_scale: f32) -> Self {
        self.user_scale = user_scale;
        self
    }

    pub fn with_safe_area(mut self, safe_area: Insets) -> Self {
        self.safe_area = safe_area;
        self
    }

    /// Physical pixels in one logical pixel. Always a number in a usable range, whatever the inputs.
    pub fn scale(&self) -> f32 {
        let valid = |v: f32| v.is_finite() && v > 0.0;
        if !(self.pixels.0.is_finite() && self.pixels.1.is_finite()) || !valid(self.reference.0) || !valid(self.reference.1) {
            return 1.0;
        }
        let (across, down) = (self.pixels.0 / self.reference.0, self.pixels.1 / self.reference.1);
        let base = match self.mode {
            MatchMode::Width => across,
            MatchMode::Height => down,
            MatchMode::Min => across.min(down),
            MatchMode::Expand => across.max(down),
        };
        let user = if self.user_scale.is_finite() { self.user_scale.clamp(USER_SCALE_RANGE.0, USER_SCALE_RANGE.1) } else { 1.0 };
        (base * user).clamp(SCALE_RANGE.0, SCALE_RANGE.1)
    }

    /// The size of the screen in logical pixels: what the UI is laid out in.
    pub fn canvas(&self) -> (f32, f32) {
        let scale = self.scale();
        if self.pixels.0.is_finite() && self.pixels.1.is_finite() {
            (self.pixels.0.max(0.0) / scale, self.pixels.1.max(0.0) / scale)
        } else {
            self.reference
        }
    }

    /// The safe area in logical pixels, each edge kept between 0 and half of the canvas, so that the two insets of an axis can
    /// never overlap.
    pub fn safe_area_logical(&self) -> Insets {
        let canvas = self.canvas();
        let scaled = self.safe_area.scaled(1.0 / self.scale());
        let fit = |v: f32, side: f32| if v.is_finite() { v.clamp(0.0, side / 2.0) } else { 0.0 };
        Insets::new(fit(scaled.left, canvas.0), fit(scaled.top, canvas.1), fit(scaled.right, canvas.0), fit(scaled.bottom, canvas.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(w: f32, h: f32, mode: MatchMode) -> f32 {
        ScreenMetrics::new((w, h)).with_mode(mode).scale()
    }

    #[test]
    fn the_reference_screen_has_scale_one_and_the_default_mode_is_min() {
        assert_eq!(ScreenMetrics::new((1920.0, 1080.0)).scale(), 1.0);
        assert_eq!(ScreenMetrics::new((0.0, 0.0)).mode, MatchMode::Min);
        assert_eq!(ScreenMetrics::new((1920.0, 1080.0)).canvas(), (1920.0, 1080.0));
    }

    #[test]
    fn each_match_mode_follows_its_own_side() {
        // A wide screen: 2560 x 1080 is 1.33 across the reference and 1 down.
        assert_eq!(scale(2560.0, 1080.0, MatchMode::Width), 2560.0 / 1920.0);
        assert_eq!(scale(2560.0, 1080.0, MatchMode::Height), 1.0);
        assert_eq!(scale(2560.0, 1080.0, MatchMode::Min), 1.0);
        assert_eq!(scale(2560.0, 1080.0, MatchMode::Expand), 2560.0 / 1920.0);
        // A phone held upright: 1080 x 2400.
        assert_eq!(scale(1080.0, 2400.0, MatchMode::Min), 1080.0 / 1920.0);
        assert_eq!(scale(1080.0, 2400.0, MatchMode::Expand), 2400.0 / 1080.0);
    }

    #[test]
    fn min_keeps_the_whole_reference_screen_inside_the_canvas() {
        for (w, h) in [(1280.0, 720.0), (2560.0, 1080.0), (1080.0, 2400.0), (800.0, 800.0)] {
            let canvas = ScreenMetrics::new((w, h)).canvas();
            assert!(canvas.0 >= 1920.0 - 0.01 && canvas.1 >= 1080.0 - 0.01, "{w} x {h} gives a canvas of {canvas:?}");
        }
    }

    #[test]
    fn a_denser_display_of_the_same_window_keeps_the_proportions() {
        let ordinary = ScreenMetrics::new((1280.0, 720.0));
        let dense = ScreenMetrics::new((2560.0, 1440.0));
        assert_eq!(dense.scale(), 2.0 * ordinary.scale());
        assert_eq!(dense.canvas(), ordinary.canvas(), "the same logical screen, twice the pixels");
    }

    #[test]
    fn the_user_scale_zooms_the_ui_and_shrinks_the_canvas() {
        let big = ScreenMetrics::new((1920.0, 1080.0)).with_user_scale(2.0);
        assert_eq!(big.scale(), 2.0);
        assert_eq!(big.canvas(), (960.0, 540.0));
        assert_eq!(ScreenMetrics::new((1920.0, 1080.0)).with_user_scale(100.0).scale(), 4.0, "clamped");
        assert_eq!(ScreenMetrics::new((1920.0, 1080.0)).with_user_scale(0.0).scale(), 0.25);
        assert_eq!(ScreenMetrics::new((1920.0, 1080.0)).with_user_scale(f32::NAN).scale(), 1.0);
    }

    #[test]
    fn bad_numbers_give_a_usable_scale() {
        assert_eq!(ScreenMetrics::new((0.0, 0.0)).scale(), 0.01);
        assert_eq!(ScreenMetrics::new((f32::NAN, 100.0)).scale(), 1.0);
        assert_eq!(ScreenMetrics::new((f32::INFINITY, f32::INFINITY)).scale(), 1.0);
        assert_eq!(ScreenMetrics::new((f32::NAN, 100.0)).canvas(), REFERENCE_SIZE);
        let mut odd = ScreenMetrics::new((100.0, 100.0));
        odd.reference = (0.0, -5.0);
        assert_eq!(odd.scale(), 1.0);
    }

    #[test]
    fn the_safe_area_is_given_in_pixels_and_used_in_logical_pixels() {
        // A phone: 1080 x 1920 pixels with 60 pixels at the top and 40 at the bottom taken by the system.
        let phone = ScreenMetrics::new((1080.0, 1920.0)).with_safe_area(Insets::new(0.0, 60.0, 0.0, 40.0));
        let logical = phone.safe_area_logical();
        assert_eq!(phone.scale(), 1080.0 / 1920.0);
        assert!((logical.top - 60.0 / phone.scale()).abs() < 1e-3 && (logical.bottom - 40.0 / phone.scale()).abs() < 1e-3);
        assert_eq!((logical.left, logical.right), (0.0, 0.0));
    }

    #[test]
    fn insets_never_overlap_or_go_negative() {
        let screen = ScreenMetrics::new((1920.0, 1080.0)).with_safe_area(Insets::new(5000.0, -10.0, f32::NAN, 5000.0));
        let inset = screen.safe_area_logical();
        assert_eq!((inset.left, inset.top, inset.right, inset.bottom), (960.0, 0.0, 0.0, 540.0));
    }
}
