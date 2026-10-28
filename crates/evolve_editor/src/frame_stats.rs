//! What the editor's frames really cost, shown in the status bar: how long one frame of the editor takes (all panels, the
//! views and the undo bookkeeping), how often frames come, and where the time went. Measured with the wall clock around the
//! real work, not estimated: the numbers of a benchmark only cover the renderer, and the editor does a lot more.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many recent frames the "worst" figure looks at.
const RECENT: usize = 120;

/// Frames further apart than this are not a frame rate: nothing was asking for a new frame (the editor was idle).
const IDLE_AFTER_MS: f64 = 250.0;

#[derive(Default)]
pub struct FrameStats {
    began: Option<Instant>,
    previous_began: Option<Instant>,
    /// Milliseconds from the start of the previous frame to the start of this one.
    interval: Option<f64>,
    /// The work of the recent frames, in milliseconds.
    recent: VecDeque<f64>,
    /// This frame's parts so far, and the parts of the last finished frame.
    parts: Vec<(&'static str, f64)>,
    last_parts: Vec<(&'static str, f64)>,
    /// Frames started so far.
    frames: u64,
}

impl FrameStats {
    /// A frame of the editor starts.
    pub fn begin(&mut self) {
        self.begin_at(Instant::now());
    }

    fn begin_at(&mut self, now: Instant) {
        self.interval = self.previous_began.map(|p| now.duration_since(p).as_secs_f64() * 1000.0);
        self.previous_began = Some(now);
        self.began = Some(now);
        self.parts.clear();
        self.frames += 1;
    }

    /// Adds `took` to the part called `name` of this frame (a part can be timed in several places).
    pub fn add(&mut self, name: &'static str, took: Duration) {
        let ms = took.as_secs_f64() * 1000.0;
        match self.parts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, total)) => *total += ms,
            None => self.parts.push((name, ms)),
        }
    }

    /// The frame is finished.
    pub fn end(&mut self) {
        let work = self.began.take().map_or(0.0, |b| b.elapsed().as_secs_f64() * 1000.0);
        self.finish(work);
    }

    fn finish(&mut self, work_ms: f64) {
        self.recent.push_back(work_ms);
        if self.recent.len() > RECENT {
            self.recent.pop_front();
        }
        self.last_parts = std::mem::take(&mut self.parts);
    }

    /// The work of the last finished frame.
    #[cfg(test)]
    pub fn last_work(&self) -> Option<f64> {
        self.recent.back().copied()
    }

    /// The average and the worst work of the recent frames.
    pub fn average_and_worst(&self) -> Option<(f64, f64)> {
        if self.recent.is_empty() {
            return None;
        }
        let sum: f64 = self.recent.iter().sum();
        let worst = self.recent.iter().copied().fold(0.0, f64::max);
        Some((sum / self.recent.len() as f64, worst))
    }

    /// Frames per second, when frames are following each other (None when the editor was idle).
    pub fn rate(&self) -> Option<f64> {
        self.interval.filter(|ms| *ms > 0.0 && *ms < IDLE_AFTER_MS).map(|ms| 1000.0 / ms)
    }

    /// One line for the status bar.
    pub fn line(&self) -> String {
        let Some((average, worst)) = self.average_and_worst() else { return String::new() };
        let rate = self.rate().map_or("idle".to_string(), |r| format!("{r:.0} fps"));
        format!("{rate}  ·  frame {:.1} ms (worst {worst:.0})", average)
    }

    /// Where the time of the last frame went, for the hover text.
    pub fn breakdown(&self) -> String {
        let mut text = String::from("Work of the last frame of the editor (the card and the window system come on top):\n");
        for (name, ms) in &self.last_parts {
            text.push_str(&format!("  {name}: {ms:.1} ms\n"));
        }
        if let Some((average, worst)) = self.average_and_worst() {
            text.push_str(&format!("Last {} frames: average {average:.1} ms, worst {worst:.1} ms\n", self.recent.len()));
        }
        text.push_str("fps counts frames that follow each other; an idle editor draws nothing and shows \"idle\".");
        text
    }

    #[cfg(test)]
    pub fn frames(&self) -> u64 {
        self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn frames_that_follow_each_other_give_a_rate_and_a_pause_gives_idle() {
        let mut stats = FrameStats::default();
        let start = Instant::now();
        stats.begin_at(start);
        assert_eq!(stats.rate(), None, "one frame is no rate");
        stats.finish(2.0);
        stats.begin_at(start + ms(20));
        assert_eq!(stats.rate().map(|r| r.round()), Some(50.0));
        stats.finish(2.0);
        stats.begin_at(start + ms(20) + ms(2000));
        assert_eq!(stats.rate(), None, "two seconds apart is not a frame rate");
        assert!(stats.line().contains("idle"));
    }

    #[test]
    fn the_average_and_the_worst_cover_only_the_recent_frames() {
        let mut stats = FrameStats::default();
        stats.finish(500.0); // an old spike
        for _ in 0..RECENT {
            stats.finish(10.0);
        }
        let (average, worst) = stats.average_and_worst().unwrap();
        assert_eq!((average, worst), (10.0, 10.0), "the spike is out of the window");
        stats.finish(90.0);
        assert_eq!(stats.average_and_worst().unwrap().1, 90.0);
        assert_eq!(stats.last_work(), Some(90.0));
    }

    #[test]
    fn a_part_timed_in_several_places_adds_up_and_the_breakdown_names_it() {
        let mut stats = FrameStats::default();
        stats.begin_at(Instant::now());
        stats.add("Scene view: drawing", ms(3));
        stats.add("Game view: drawing", ms(1));
        stats.add("Scene view: drawing", ms(4));
        stats.finish(9.0);
        let text = stats.breakdown();
        assert!(text.contains("Scene view: drawing: 7.0 ms"), "{text}");
        assert!(text.contains("Game view: drawing: 1.0 ms"), "{text}");
        stats.begin_at(Instant::now());
        assert!(stats.breakdown().contains("Scene view: drawing"), "the last finished frame stays readable while the next runs");
        assert_eq!(stats.frames(), 2);
    }

    #[test]
    fn nothing_is_shown_before_the_first_frame_has_finished() {
        assert_eq!(FrameStats::default().line(), "");
    }
}
