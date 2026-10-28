//! Quality profiles (plan D15, tasks 5.8 and 5.9): how much memory the renderer may take, how long a frame may spend making
//! shapes that are not needed at once, and how large the picture is drawn compared with the view.
//!
//! There are three: **High** (a discrete card), **Balanced** (an integrated card, or WebGL2) and **Low** (software rendering, or
//! when nothing else keeps the frame time). The numbers below are proposals from the plan: the T1 card needs far less than High
//! allows, and what T2 and the web need is still to be measured (open point P2), so they are meant to be changed in this one file.
//!
//! The profile is chosen in three ways ([`Quality`]): *Automatic* (from the adapter, and then lowered by the [`Governor`] while
//! frames are slow and raised again when they are fast, never above what the adapter allows), or one of the three by hand. What is
//! kept from sight on purpose is memory: a **budget** (the caps of [`QualityProfile::budgets`]) is only set from the choice and the
//! adapter, never lowered by the governor, because a cap below what is in use makes the next request fail. The governor changes
//! only what costs nothing to change: the render scale and the time for shapes.

use crate::alloc::{Budgets, Category};
use crate::device::{Gpu, Mode};
use std::collections::VecDeque;
use std::fmt;

const MB: u64 = 1 << 20;

/// How much the renderer asks of the card and the processor. Ordered from the best to the least.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum QualityProfile {
    #[default]
    High,
    Balanced,
    Low,
}

impl QualityProfile {
    pub const ALL: [QualityProfile; 3] = [QualityProfile::High, QualityProfile::Balanced, QualityProfile::Low];

    pub fn name(self) -> &'static str {
        match self {
            QualityProfile::High => "High",
            QualityProfile::Balanced => "Balanced",
            QualityProfile::Low => "Low",
        }
    }

    pub fn parse(text: &str) -> Option<QualityProfile> {
        QualityProfile::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(text.trim()))
    }

    /// The most bytes each kind of resource may hold (plan 5.8). The layer pool is capped at 64, 32 and 16 MB; a layer that does not
    /// fit is drawn smaller (D13), never left out. The caps of the others halve with each step down.
    pub fn budgets(self) -> Budgets {
        let (geometry, table, ramps, layers, targets, glyphs, other) = match self {
            QualityProfile::High => (512, 32, 16, 64, 768, 64, 128),
            QualityProfile::Balanced => (256, 16, 8, 32, 384, 32, 64),
            QualityProfile::Low => (128, 8, 4, 16, 192, 16, 32),
        };
        Budgets::high()
            .with(Category::Geometry, geometry * MB)
            .with(Category::ShapeTable, table * MB)
            .with(Category::Ramps, ramps * MB)
            .with(Category::Layers, layers * MB)
            .with(Category::Targets, targets * MB)
            .with(Category::Glyphs, glyphs * MB)
            .with(Category::Other, other * MB)
    }

    /// Milliseconds one frame may spend making the triangles of shapes that are not needed at once (plan 5.4). Halved in the browser,
    /// which has one thread for everything (the caller does that).
    pub fn tessellation_ms(self) -> f64 {
        match self {
            QualityProfile::High => 4.0,
            QualityProfile::Balanced => 3.0,
            QualityProfile::Low => 2.0,
        }
    }

    /// How large the picture is drawn compared with the view (1 is the view's own size): below that it is drawn smaller and shown
    /// stretched, which costs a share of the fill rate that is the square of the scale.
    pub fn render_scale(self) -> f32 {
        match self {
            QualityProfile::High | QualityProfile::Balanced => 1.0,
            QualityProfile::Low => 0.75,
        }
    }

    /// The next profile down, if there is one.
    pub fn lower(self) -> Option<QualityProfile> {
        match self {
            QualityProfile::High => Some(QualityProfile::Balanced),
            QualityProfile::Balanced => Some(QualityProfile::Low),
            QualityProfile::Low => None,
        }
    }

    /// The next profile up, if there is one.
    pub fn higher(self) -> Option<QualityProfile> {
        match self {
            QualityProfile::High => None,
            QualityProfile::Balanced => Some(QualityProfile::High),
            QualityProfile::Low => Some(QualityProfile::Balanced),
        }
    }

    /// What an adapter of this kind deserves before anything is measured: a discrete card High, an integrated one (or a virtual
    /// one, or one that is not known) Balanced, software rendering Low.
    pub fn for_adapter(info: &wgpu::AdapterInfo) -> QualityProfile {
        match info.device_type {
            wgpu::DeviceType::DiscreteGpu => QualityProfile::High,
            wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => QualityProfile::Balanced,
            wgpu::DeviceType::Cpu => QualityProfile::Low,
        }
    }

    /// The profile a device starts with: that of its adapter, and no better than Balanced in WebGL2 mode (a browser's WebGL2 sits
    /// on top of the card's driver and has less to work with; the web canary will say if that is too careful).
    pub fn for_device(gpu: &Gpu) -> QualityProfile {
        QualityProfile::for_adapter(&gpu.info).in_mode(gpu.mode)
    }

    /// This profile as a device in `mode` can have it: WebGL2 mode never gets better than Balanced.
    pub fn in_mode(self, mode: Mode) -> QualityProfile {
        match mode {
            Mode::Native => self,
            Mode::WebGl2 => self.max(QualityProfile::Balanced),
        }
    }
}

impl fmt::Display for QualityProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What the user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Quality {
    /// The profile of the adapter, lowered by the governor while frames are slow.
    #[default]
    Auto,
    /// This profile, whatever the frame time.
    Fixed(QualityProfile),
}

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::Auto, Quality::Fixed(QualityProfile::High), Quality::Fixed(QualityProfile::Balanced), Quality::Fixed(QualityProfile::Low)];

    /// Reads the value of `--quality`: `auto`, `high`, `balanced` or `low`.
    pub fn parse(text: &str) -> Option<Quality> {
        if text.trim().eq_ignore_ascii_case("auto") || text.trim().eq_ignore_ascii_case("automatic") {
            return Some(Quality::Auto);
        }
        QualityProfile::parse(text).map(Quality::Fixed)
    }

    /// Finds `--quality <value>` in a command line. `Err` says what is wrong with it.
    pub fn from_args(args: &[String]) -> Result<Quality, String> {
        let Some(at) = args.iter().position(|a| a == "--quality") else { return Ok(Quality::default()) };
        let value = args.get(at + 1).ok_or("--quality needs a value: auto, high, balanced or low")?;
        Quality::parse(value).ok_or_else(|| format!("--quality {value}: expected auto, high, balanced or low"))
    }

    /// The best profile this choice allows on a device whose adapter calls for `adapter`.
    pub fn ceiling(self, adapter: QualityProfile) -> QualityProfile {
        match self {
            Quality::Auto => adapter,
            Quality::Fixed(profile) => profile,
        }
    }

    /// One byte for a place that is read from several threads.
    #[cfg(any(feature = "egui", test))]
    pub(crate) fn to_byte(self) -> u8 {
        match self {
            Quality::Auto => 0,
            Quality::Fixed(QualityProfile::High) => 1,
            Quality::Fixed(QualityProfile::Balanced) => 2,
            Quality::Fixed(QualityProfile::Low) => 3,
        }
    }

    #[cfg(any(feature = "egui", test))]
    pub(crate) fn from_byte(byte: u8) -> Quality {
        match byte {
            1 => Quality::Fixed(QualityProfile::High),
            2 => Quality::Fixed(QualityProfile::Balanced),
            3 => Quality::Fixed(QualityProfile::Low),
            _ => Quality::Auto,
        }
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quality::Auto => f.write_str("Automatic"),
            Quality::Fixed(profile) => profile.fmt(f),
        }
    }
}

/// Frames looked at together before the profile is lowered, and the time that is too slow for them (their median): 30 frames a
/// second is the least a view that is being worked in should give.
pub const SLOW_WINDOW: usize = 30;
pub const SLOW_MS: f64 = 33.0;
/// Frames in a row at or under [`FAST_MS`] before the profile is raised again (three seconds at 60 frames a second).
pub const FAST_RUN: u32 = 180;
pub const FAST_MS: f64 = 10.0;
/// Frames after a change in which nothing is judged: the new profile has to show what it does.
pub const COOLDOWN: u32 = 90;

/// Chooses the profile of a view from the time its frames take (D15). It lowers the profile one step when the median of the last
/// [`SLOW_WINDOW`] frames is over [`SLOW_MS`], raises it one step after [`FAST_RUN`] frames in a row at or under [`FAST_MS`], never
/// above its ceiling, and judges nothing for [`COOLDOWN`] frames after a change. One slow frame (a zoom that made many shapes) is a
/// spike in the median, not a reason. Times that are not numbers or negative are ignored.
#[derive(Clone, Debug)]
pub struct Governor {
    ceiling: QualityProfile,
    current: QualityProfile,
    recent: VecDeque<f64>,
    fast_run: u32,
    cooldown: u32,
}

impl Governor {
    pub fn new(ceiling: QualityProfile) -> Governor {
        Governor { ceiling, current: ceiling, recent: VecDeque::with_capacity(SLOW_WINDOW), fast_run: 0, cooldown: 0 }
    }

    /// The profile to draw with now.
    pub fn current(&self) -> QualityProfile {
        self.current
    }

    pub fn ceiling(&self) -> QualityProfile {
        self.ceiling
    }

    /// Follows what the user chose (`fixed`: a profile by hand, which is used as it is and never lowered) or the adapter (`Auto`:
    /// the profile it deserves is the best the governor may reach, and it starts from there).
    pub fn follow(&mut self, quality: Quality, adapter: QualityProfile) {
        let ceiling = quality.ceiling(adapter);
        let changed = ceiling != self.ceiling || (matches!(quality, Quality::Fixed(_)) && self.current != ceiling);
        if changed {
            self.ceiling = ceiling;
            self.current = if matches!(quality, Quality::Fixed(_)) { ceiling } else { self.current.max(ceiling) };
            self.restart();
        }
    }

    fn restart(&mut self) {
        self.recent.clear();
        self.fast_run = 0;
        self.cooldown = COOLDOWN;
    }

    /// Notes the time a frame took (milliseconds) and returns the new profile when it changed. For `Auto` only: a view that was
    /// given a profile by hand does not call this.
    pub fn record(&mut self, frame_ms: f64) -> Option<QualityProfile> {
        if !frame_ms.is_finite() || frame_ms < 0.0 {
            return None;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return None;
        }
        if self.recent.len() == SLOW_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(frame_ms);
        self.fast_run = if frame_ms <= FAST_MS { self.fast_run + 1 } else { 0 };
        if self.recent.len() == SLOW_WINDOW && median(&self.recent) > SLOW_MS {
            if let Some(lower) = self.current.lower() {
                self.current = lower;
                self.restart();
                return Some(lower);
            }
        } else if self.fast_run >= FAST_RUN && self.current > self.ceiling {
            if let Some(higher) = self.current.higher() {
                self.current = higher;
                self.restart();
                return Some(higher);
            }
        }
        None
    }
}

fn median(times: &VecDeque<f64>) -> f64 {
    let mut sorted: Vec<f64> = times.iter().copied().collect();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real adapter's description with another kind of device.
    fn adapter(gpu: &Gpu, device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo { device_type, ..gpu.info.clone() }
    }

    #[test]
    fn every_step_down_asks_for_less_of_everything_the_card_has() {
        let profiles = QualityProfile::ALL;
        for pair in profiles.windows(2) {
            let (better, worse) = (pair[0], pair[1]);
            for category in Category::ALL {
                assert!(worse.budgets().get(category) < better.budgets().get(category), "{worse} against {better}: {category:?}");
            }
            assert!(worse.tessellation_ms() < better.tessellation_ms(), "{worse} spends less time on shapes than {better}");
            assert!(worse.render_scale() <= better.render_scale());
            assert_eq!(better.lower(), Some(worse));
            assert_eq!(worse.higher(), Some(better));
        }
        assert_eq!((QualityProfile::High.higher(), QualityProfile::Low.lower()), (None, None));
        assert_eq!(QualityProfile::High.budgets().get(Category::Layers), 64 * MB, "the plan's caps for the layer pool: 64, 32 and 16 MB");
        assert_eq!(QualityProfile::Balanced.budgets().get(Category::Layers), 32 * MB);
        assert_eq!(QualityProfile::Low.budgets().get(Category::Layers), 16 * MB);
        assert!(QualityProfile::Low.render_scale() > 0.0 && QualityProfile::Low.render_scale() < 1.0);
        assert_eq!(QualityProfile::High.render_scale(), 1.0, "the best profile draws at the view's own size");
    }

    #[test]
    fn the_high_profile_is_the_budget_the_renderer_has_always_had() {
        assert_eq!(QualityProfile::High.budgets(), Budgets::high());
    }

    #[test]
    fn an_adapter_gets_the_profile_of_its_kind() {
        let Some(gpu) = Gpu::headless() else { return };
        assert_eq!(QualityProfile::for_adapter(&adapter(&gpu, wgpu::DeviceType::DiscreteGpu)), QualityProfile::High);
        assert_eq!(QualityProfile::for_adapter(&adapter(&gpu, wgpu::DeviceType::IntegratedGpu)), QualityProfile::Balanced);
        assert_eq!(QualityProfile::for_adapter(&adapter(&gpu, wgpu::DeviceType::VirtualGpu)), QualityProfile::Balanced);
        assert_eq!(QualityProfile::for_adapter(&adapter(&gpu, wgpu::DeviceType::Other)), QualityProfile::Balanced);
        assert_eq!(QualityProfile::for_adapter(&adapter(&gpu, wgpu::DeviceType::Cpu)), QualityProfile::Low);
    }

    #[test]
    fn webgl2_mode_is_never_better_than_balanced_and_never_worse_than_its_adapter() {
        for profile in QualityProfile::ALL {
            assert_eq!(profile.in_mode(Mode::Native), profile);
            assert_eq!(profile.in_mode(Mode::WebGl2), profile.max(QualityProfile::Balanced));
        }
        assert_eq!(QualityProfile::High.in_mode(Mode::WebGl2), QualityProfile::Balanced);
        assert_eq!(QualityProfile::Low.in_mode(Mode::WebGl2), QualityProfile::Low);
        Gpu::for_each_test_mode(|gpu| {
            let alone = QualityProfile::for_adapter(&gpu.info);
            let profile = QualityProfile::for_device(gpu);
            match gpu.mode {
                Mode::Native => assert_eq!(profile, alone),
                Mode::WebGl2 => assert_eq!(profile, alone.max(QualityProfile::Balanced)),
            }
        });
    }

    #[test]
    fn the_choice_is_read_from_text_and_the_command_line() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Quality::from_args(&args(&[])), Ok(Quality::Auto), "automatic unless told otherwise");
        assert_eq!(Quality::from_args(&args(&["--quality", "low"])), Ok(Quality::Fixed(QualityProfile::Low)));
        assert_eq!(Quality::from_args(&args(&["--renderer", "gpu", "--quality", "HIGH", "--play"])), Ok(Quality::Fixed(QualityProfile::High)));
        assert_eq!(Quality::from_args(&args(&["--quality", "Auto"])), Ok(Quality::Auto));
        assert!(Quality::from_args(&args(&["--quality"])).is_err());
        assert!(Quality::from_args(&args(&["--quality", "ultra"])).unwrap_err().contains("expected auto, high, balanced or low"));
        for quality in Quality::ALL {
            assert_eq!(Quality::from_byte(quality.to_byte()), quality, "{quality}");
            assert_eq!(Quality::parse(&quality.to_string().to_lowercase()), Some(quality), "{quality}");
        }
        assert_eq!(Quality::from_byte(200), Quality::Auto);
    }

    #[test]
    fn a_profile_by_hand_is_the_ceiling_and_automatic_follows_the_adapter() {
        assert_eq!(Quality::Auto.ceiling(QualityProfile::Balanced), QualityProfile::Balanced);
        assert_eq!(Quality::Fixed(QualityProfile::Low).ceiling(QualityProfile::High), QualityProfile::Low);
        assert_eq!(Quality::Fixed(QualityProfile::High).ceiling(QualityProfile::Balanced), QualityProfile::High, "a choice by hand may ask for more than the adapter would");
    }

    /// Feeds `frames` frames of `ms` each and returns the changes.
    fn feed(governor: &mut Governor, frames: usize, ms: f64) -> Vec<QualityProfile> {
        (0..frames).filter_map(|_| governor.record(ms)).collect()
    }

    #[test]
    fn slow_frames_lower_the_profile_one_step_at_a_time_down_to_low_and_no_further() {
        let mut governor = Governor::new(QualityProfile::High);
        assert_eq!(governor.current(), QualityProfile::High);
        let steps = feed(&mut governor, 2000, 60.0);
        assert_eq!(steps, vec![QualityProfile::Balanced, QualityProfile::Low], "one step each time, then nothing more");
        assert_eq!(governor.current(), QualityProfile::Low);
        // The first step needs a full window of slow frames; none is judged before it.
        let mut governor = Governor::new(QualityProfile::High);
        assert!(feed(&mut governor, SLOW_WINDOW - 1, 60.0).is_empty());
        assert_eq!(governor.record(60.0), Some(QualityProfile::Balanced));
    }

    #[test]
    fn a_few_slow_frames_among_good_ones_are_not_a_reason() {
        let mut governor = Governor::new(QualityProfile::High);
        for round in 0..200 {
            // Nine frames in ten are fine, and the tenth is a zoom that made many shapes (200 ms).
            assert_eq!(governor.record(if round % 10 == 9 { 200.0 } else { 12.0 }), None, "round {round}");
        }
        // Just under the limit, all the time, is fine too.
        assert!(feed(&mut governor, 500, SLOW_MS - 0.5).is_empty());
        assert_eq!(governor.current(), QualityProfile::High);
    }

    #[test]
    fn fast_frames_raise_the_profile_again_but_never_above_the_ceiling() {
        let mut governor = Governor::new(QualityProfile::High);
        feed(&mut governor, 2000, 60.0);
        assert_eq!(governor.current(), QualityProfile::Low);
        let steps = feed(&mut governor, 5000, 5.0);
        assert_eq!(steps, vec![QualityProfile::Balanced, QualityProfile::High], "back up, one step at a time");
        assert!(feed(&mut governor, 1000, 5.0).is_empty(), "and no higher than High");
        // A ceiling of Balanced stops at Balanced.
        let mut governor = Governor::new(QualityProfile::Balanced);
        feed(&mut governor, 2000, 60.0);
        assert_eq!(feed(&mut governor, 5000, 5.0), vec![QualityProfile::Balanced]);
        assert_eq!(governor.current(), QualityProfile::Balanced);
    }

    #[test]
    fn frames_that_are_only_ok_do_not_raise_the_profile_and_a_slow_one_starts_the_count_again() {
        let mut governor = Governor::new(QualityProfile::High);
        feed(&mut governor, 2000, 60.0);
        assert_eq!(governor.current(), QualityProfile::Low);
        // 20 ms frames are neither slow nor fast: the profile stays.
        assert!(feed(&mut governor, 3000, 20.0).is_empty());
        // A slow frame in the middle of a fast run starts the run again: three runs that are almost long enough, and then one that is.
        assert_eq!(governor.record(25.0), None);
        for _ in 0..3 {
            assert!(feed(&mut governor, FAST_RUN as usize - 1, 5.0).is_empty());
            assert_eq!(governor.record(25.0), None);
        }
        assert_eq!(governor.current(), QualityProfile::Low);
        assert_eq!(feed(&mut governor, FAST_RUN as usize, 5.0), vec![QualityProfile::Balanced]);
    }

    #[test]
    fn nothing_is_judged_while_a_new_profile_shows_what_it_does() {
        let mut governor = Governor::new(QualityProfile::High);
        assert_eq!(feed(&mut governor, SLOW_WINDOW, 60.0), vec![QualityProfile::Balanced]);
        // Slow frames right after a change are not counted until the cooldown is over.
        assert!(feed(&mut governor, COOLDOWN as usize + SLOW_WINDOW - 1, 60.0).is_empty());
        assert_eq!(governor.record(60.0), Some(QualityProfile::Low));
    }

    #[test]
    fn times_that_are_not_numbers_or_negative_change_nothing() {
        let mut governor = Governor::new(QualityProfile::High);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(feed(&mut governor, 500, bad), vec![]);
        }
        assert_eq!(governor.current(), QualityProfile::High);
        // ... and they do not count as a fast frame either.
        feed(&mut governor, 2000, 60.0);
        let before = governor.current();
        feed(&mut governor, 2000, f64::NAN);
        assert_eq!(governor.current(), before);
    }

    #[test]
    fn following_the_choice_restarts_from_the_ceiling() {
        let mut governor = Governor::new(QualityProfile::High);
        feed(&mut governor, 2000, 60.0);
        assert_eq!(governor.current(), QualityProfile::Low);
        // A profile by hand is used as it is.
        governor.follow(Quality::Fixed(QualityProfile::Balanced), QualityProfile::High);
        assert_eq!((governor.current(), governor.ceiling()), (QualityProfile::Balanced, QualityProfile::Balanced));
        governor.follow(Quality::Fixed(QualityProfile::High), QualityProfile::High);
        assert_eq!(governor.current(), QualityProfile::High);
        // Back to automatic on an adapter that deserves Balanced: that is the best it may reach, and it keeps what it has when that is lower.
        governor.follow(Quality::Auto, QualityProfile::Balanced);
        assert_eq!((governor.current(), governor.ceiling()), (QualityProfile::Balanced, QualityProfile::Balanced));
        feed(&mut governor, 2000, 60.0);
        assert_eq!(governor.current(), QualityProfile::Low);
        governor.follow(Quality::Auto, QualityProfile::Balanced);
        assert_eq!(governor.current(), QualityProfile::Low, "following the same adapter again changes nothing");
        // The same choice asked again changes nothing either.
        governor.follow(Quality::Fixed(QualityProfile::Low), QualityProfile::Balanced);
        governor.follow(Quality::Fixed(QualityProfile::Low), QualityProfile::Balanced);
        assert_eq!(governor.current(), QualityProfile::Low);
    }
}
