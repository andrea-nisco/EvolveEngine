//! Comparing two pictures, for checking the GPU renderer against the CPU renderer (the reference).
//!
//! Two correct renderers never agree exactly on anti-aliased edges: tiny-skia computes exact coverage, the GPU samples
//! 4 points per pixel. So the rule treats two kinds of pixel differently. Pixels near a color change in the
//! reference ("edge" pixels) may differ a lot; all others ("interior" pixels) must be almost identical. This catches a
//! wrong color or a shape in the wrong place (an interior or whole-edge error) without failing on honest MSAA edges.

/// What counts as "the same picture". The numbers are the proposal of the plan (section 6), to be tuned on real
/// results in phase 3.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    /// A pixel is on a color change when a neighbor (8 around it) differs from it by more than this in any channel.
    /// Smooth gradients change by a step or two per pixel, so they stay interior.
    pub change_step: u8,
    /// An interior pixel exceeds its limit when any channel is off by more than this.
    pub interior_limit: u8,
    /// An edge pixel (within 1 pixel of a change) exceeds its limit when any channel is off by more than this.
    pub edge_limit: u8,
    /// Share (0 to 1) of interior pixels that may exceed their limit (stray pixels, for example tiny details).
    pub interior_exceeding: f32,
    /// Share (0 to 1) of edge pixels that may exceed their limit. Measured on real pictures (2026-10-15): honest 4x MSAA
    /// against tiny-skia puts a handful of edge pixels, at thin curves and sharp corners, a little past 96 (97 and 113
    /// were seen), well under 0.2% of the edge pixels. A mistake in how pixels are placed puts nearly all of them past it
    /// (a shift of half a pixel is 127 on every straight edge), so 0.5% separates the two.
    pub edge_exceeding: f32,
    /// Upper bound of the average difference over the whole picture, over all channels (0 to 255).
    pub mean_difference: f32,
}

impl Default for Rule {
    fn default() -> Self {
        Self { change_step: 12, interior_limit: 3, edge_limit: 96, interior_exceeding: 0.001, edge_exceeding: 0.005, mean_difference: 1.0 }
    }
}

/// How two pictures differ, and the rule they were judged by.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub pixels: usize,
    /// Pixels within 1 pixel of a color change in the reference.
    pub edge_pixels: usize,
    pub interior_exceeding: usize,
    pub edge_exceeding: usize,
    /// The largest channel difference among interior pixels, and where.
    pub worst_interior: (u8, (u32, u32)),
    /// The largest channel difference among edge pixels, and where.
    pub worst_edge: (u8, (u32, u32)),
    /// Average difference over all channels of all pixels.
    pub mean_difference: f32,
    pub rule: Rule,
}

impl Comparison {
    pub fn interior_pixels(&self) -> usize {
        self.pixels - self.edge_pixels
    }

    pub fn interior_ok(&self) -> bool {
        self.interior_exceeding as f32 <= self.rule.interior_exceeding * self.interior_pixels() as f32
    }

    pub fn edge_ok(&self) -> bool {
        self.edge_exceeding as f32 <= self.rule.edge_exceeding * self.edge_pixels as f32
    }

    pub fn mean_ok(&self) -> bool {
        self.mean_difference <= self.rule.mean_difference
    }

    pub fn passes(&self) -> bool {
        self.interior_ok() && self.edge_ok() && self.mean_ok()
    }
}

impl std::fmt::Display for Comparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mark = |ok: bool| if ok { "ok" } else { "FAILS" };
        write!(
            f,
            "interior {}: {} of {} pixels beyond {} (allowed {:.2}%), worst {} at {:?}; edge {}: {} of {} pixels beyond {} (allowed {:.2}%), worst {} at {:?}; average {}: {:.3} (allowed {})",
            mark(self.interior_ok()),
            self.interior_exceeding,
            self.interior_pixels(),
            self.rule.interior_limit,
            self.rule.interior_exceeding * 100.0,
            self.worst_interior.0,
            self.worst_interior.1,
            mark(self.edge_ok()),
            self.edge_exceeding,
            self.edge_pixels,
            self.rule.edge_limit,
            self.rule.edge_exceeding * 100.0,
            self.worst_edge.0,
            self.worst_edge.1,
            mark(self.mean_ok()),
            self.mean_difference,
            self.rule.mean_difference,
        )
    }
}

fn channel_difference(a: &[u8; 4], b: &[u8; 4]) -> u8 {
    a.iter().zip(b).map(|(&x, &y)| x.abs_diff(y)).max().unwrap_or(0)
}

/// Marks the pixels within 1 pixel of a color change in `picture` (`width * height * 4` bytes). A pixel is on a change
/// when one of its 8 neighbors differs from it by more than `change_step` in any channel; the mask is that set grown
/// by one pixel, so both sides of every change, and the pixel next to each side, count as edge.
pub fn edge_mask(picture: &[u8], width: u32, change_step: u8) -> Vec<bool> {
    let pixels = pixel_slice(picture, width);
    let (w, h) = (width as usize, pixels.len() / width as usize);
    let mut change = vec![false; pixels.len()];
    for y in 0..h {
        for x in 0..w {
            let here = &pixels[y * w + x];
            'neighbors: for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    if channel_difference(here, &pixels[ny * w + nx]) > change_step {
                        change[y * w + x] = true;
                        break 'neighbors;
                    }
                }
            }
        }
    }
    let mut mask = vec![false; pixels.len()];
    for y in 0..h {
        for x in 0..w {
            mask[y * w + x] = (y.saturating_sub(1)..=(y + 1).min(h - 1))
                .any(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|nx| change[ny * w + nx]));
        }
    }
    mask
}

fn pixel_slice(picture: &[u8], width: u32) -> &[[u8; 4]] {
    assert_eq!(picture.len() % 4, 0, "pixels are 4 bytes");
    let pixels = picture.as_chunks::<4>().0;
    assert!(width > 0 && pixels.len().is_multiple_of(width as usize), "{} pixels do not fill rows {width} wide", pixels.len());
    pixels
}

/// Judges `candidate` (the GPU picture) against `reference` (the CPU picture), both `width * height * 4` bytes.
/// Panics if the two have different sizes.
pub fn compare_pictures(reference: &[u8], candidate: &[u8], width: u32, rule: Rule) -> Comparison {
    assert_eq!(reference.len(), candidate.len(), "the pictures have different sizes");
    let (reference_pixels, candidate_pixels) = (pixel_slice(reference, width), pixel_slice(candidate, width));
    let mask = edge_mask(reference, width, rule.change_step);

    let mut result = Comparison {
        pixels: reference_pixels.len(),
        edge_pixels: mask.iter().filter(|&&edge| edge).count(),
        interior_exceeding: 0,
        edge_exceeding: 0,
        worst_interior: (0, (0, 0)),
        worst_edge: (0, (0, 0)),
        mean_difference: 0.0,
        rule,
    };
    let mut total = 0u64;
    for (index, (a, b)) in reference_pixels.iter().zip(candidate_pixels).enumerate() {
        total += a.iter().zip(b).map(|(&x, &y)| x.abs_diff(y) as u64).sum::<u64>();
        let difference = channel_difference(a, b);
        let at = (index as u32 % width, index as u32 / width);
        let (limit, exceeding, worst) = if mask[index] {
            (rule.edge_limit, &mut result.edge_exceeding, &mut result.worst_edge)
        } else {
            (rule.interior_limit, &mut result.interior_exceeding, &mut result.worst_interior)
        };
        if difference > limit {
            *exceeding += 1;
        }
        if difference > worst.0 {
            *worst = (difference, at);
        }
    }
    result.mean_difference = total as f32 / (reference.len().max(1) as f32);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::tiny_skia as sk;

    fn picture(width: usize, height: usize, color: [u8; 4]) -> Vec<u8> {
        color.repeat(width * height)
    }

    fn set(picture: &mut [u8], width: usize, x: usize, y: usize, color: [u8; 4]) {
        picture[(y * width + x) * 4..][..4].copy_from_slice(&color);
    }

    /// A 100 x 100 picture with a hard-edged white square (x and y from 40 to 59) on black.
    fn square() -> Vec<u8> {
        let mut p = picture(100, 100, [0, 0, 0, 255]);
        for y in 40..60 {
            for x in 40..60 {
                set(&mut p, 100, x, y, [255, 255, 255, 255]);
            }
        }
        p
    }

    #[test]
    fn identical_pictures_pass_with_no_difference() {
        let a = square();
        let result = compare_pictures(&a, &a, 100, Rule::default());
        assert!(result.passes(), "{result}");
        assert_eq!((result.worst_interior.0, result.worst_edge.0, result.mean_difference), (0, 0, 0.0));
    }

    #[test]
    fn the_edge_mask_is_the_pixels_within_one_pixel_of_a_color_change() {
        let mask = edge_mask(&square(), 100, 12);
        let at = |x: usize, y: usize| mask[y * 100 + x];
        // Columns 39 and 40 touch the change on the left side; 38 is the pixel next to those.
        assert!(at(39, 50) && at(40, 50), "both sides of the change");
        assert!(at(38, 50) && at(41, 50), "the pixel next to each side");
        assert!(!at(36, 50) && !at(43, 50), "farther away is interior");
        assert!(!at(50, 50) && !at(5, 5), "flat areas are interior");
        assert!(at(39, 39) && at(60, 60), "corners");
    }

    #[test]
    fn a_smooth_gradient_has_no_edges() {
        let mut p = picture(256, 4, [0, 0, 0, 255]);
        for y in 0..4 {
            for x in 0..256 {
                set(&mut p, 256, x, y, [x as u8, x as u8, x as u8, 255]);
            }
        }
        assert!(edge_mask(&p, 256, 12).iter().all(|&edge| !edge), "one step per pixel is not a color change");
    }

    #[test]
    fn interior_pixels_must_be_nearly_identical() {
        let a = picture(100, 100, [100, 100, 100, 255]);
        let mut b = a.clone();
        b.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[0] = 103);
        let result = compare_pictures(&a, &b, 100, Rule::default());
        assert!(result.interior_ok(), "3 off is within the limit: {result}");
        b.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[0] = 104);
        let result = compare_pictures(&a, &b, 100, Rule::default());
        assert!(!result.interior_ok() && !result.passes(), "4 off is beyond it: {result}");
    }

    #[test]
    fn a_few_stray_interior_pixels_are_allowed_but_not_many() {
        let a = picture(100, 100, [100, 100, 100, 255]);
        let mut few = a.clone();
        for x in 0..10 {
            set(&mut few, 100, x, 0, [200, 100, 100, 255]); // 10 of 10 000 = 0.1%, the limit
        }
        let rule = Rule { mean_difference: 255.0, ..Rule::default() };
        assert!(compare_pictures(&a, &few, 100, rule).interior_ok());
        set(&mut few, 100, 10, 0, [200, 100, 100, 255]); // 0.11%
        assert!(!compare_pictures(&a, &few, 100, rule).interior_ok());
    }

    #[test]
    fn edge_pixels_may_differ_a_lot_but_not_completely() {
        let a = square();
        let mut b = a.clone();
        let rule = Rule { mean_difference: 255.0, ..Rule::default() };
        // Edge pixels of the square's left side (columns 39 and 40), 96 off: at the limit, all of them pass.
        for y in 40..60 {
            set(&mut b, 100, 40, y, [255 - 96, 255 - 96, 255 - 96, 255]);
        }
        assert!(compare_pictures(&a, &b, 100, rule).passes());
        // 97 off on all of them: past the limit.
        for y in 40..60 {
            set(&mut b, 100, 40, y, [255 - 97, 255, 255, 255]);
        }
        let result = compare_pictures(&a, &b, 100, rule);
        assert!(!result.edge_ok() && result.interior_ok(), "{result}");
        assert_eq!(result.worst_edge.0, 97);
        // A single pixel past the limit among the 320 edge pixels is within the 0.5% that is allowed, two are not.
        let mut few = a.clone();
        set(&mut few, 100, 40, 50, [255 - 97, 255, 255, 255]);
        assert!(compare_pictures(&a, &few, 100, rule).edge_ok());
        set(&mut few, 100, 40, 51, [255 - 97, 255, 255, 255]);
        let result = compare_pictures(&a, &few, 100, rule);
        assert!(!result.edge_ok(), "{result}");
    }

    #[test]
    fn the_average_difference_is_bounded() {
        let a = square();
        let mut b = a.clone();
        // Spread small, allowed-per-pixel differences over the edge pixels only: each pixel passes, the average does not.
        let mask = edge_mask(&a, 100, 12);
        for (index, edge) in mask.iter().enumerate() {
            if *edge {
                for channel in &mut b[index * 4..index * 4 + 3] {
                    *channel = if *channel >= 96 { *channel - 96 } else { *channel + 96 }; // off by exactly 96, the edge limit
                }
            }
        }
        let strict = compare_pictures(&a, &b, 100, Rule::default());
        assert!(strict.interior_ok() && strict.edge_ok(), "{strict}");
        assert!(!strict.mean_ok() && !strict.passes(), "{strict}");
        let loose = Rule { mean_difference: 255.0, ..Rule::default() };
        assert!(compare_pictures(&a, &b, 100, loose).passes());
    }

    /// Mutation check of the rule itself: the same rectangle moved by one pixel must fail.
    #[test]
    fn a_rectangle_moved_by_one_pixel_fails() {
        let draw = |dx: f32| {
            let mut pixmap = sk::Pixmap::new(100, 80).unwrap();
            let mut paint = sk::Paint::default();
            paint.set_color_rgba8(200, 60, 30, 255);
            let rect = sk::Rect::from_ltrb(20.3 + dx, 15.6, 70.8 + dx, 55.4).unwrap();
            pixmap.fill_rect(rect, &paint, sk::Transform::identity(), None);
            pixmap.take()
        };
        let reference = draw(0.0);
        let result = compare_pictures(&reference, &draw(1.0), 100, Rule::default());
        assert!(!result.passes(), "a shift of one pixel went unnoticed: {result}");
        let result = compare_pictures(&reference, &draw(0.0), 100, Rule::default());
        assert!(result.passes(), "{result}");
    }
}
