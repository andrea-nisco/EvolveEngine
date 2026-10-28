//! A cap on the frame rate of the editor while it draws continuously (a running game, a playing clip). Idle, the editor draws only
//! when something changed (GPU_SAFETY A3) and this does nothing. The cap is for looking at how a game feels at a lower rate, and for
//! sparing the card and the battery; the picture is still shown through vsync (GPU_SAFETY A1), so with a display of 240 Hz a cap of
//! 140 lands on the display's steps (120 Hz, mostly), as any limiter on top of vsync does.

use std::fmt;
use std::time::{Duration, Instant};

/// How often the editor may draw while it draws continuously.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameLimit {
    /// As often as the display shows frames (vsync decides).
    Display,
    /// At most this many frames in a second.
    Fps(u32),
}

impl FrameLimit {
    pub const ALL: [FrameLimit; 5] = [FrameLimit::Display, FrameLimit::Fps(10), FrameLimit::Fps(30), FrameLimit::Fps(60), FrameLimit::Fps(140)];

    /// The shortest time between two frames, if there is a cap.
    pub fn interval(self) -> Option<Duration> {
        match self {
            FrameLimit::Display => None,
            FrameLimit::Fps(fps) => Some(Duration::from_secs_f64(1.0 / fps.max(1) as f64)),
        }
    }
}

impl fmt::Display for FrameLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameLimit::Display => write!(f, "Display rate (vsync)"),
            FrameLimit::Fps(fps) => write!(f, "{fps} fps"),
        }
    }
}

/// Holds frames back so that they are not closer together than the limit allows.
#[derive(Default)]
pub struct Pacer {
    last: Option<Instant>,
}

/// How long to wait, `since` the previous frame, to keep `interval` between frames.
fn wait_for(since: Duration, interval: Duration) -> Duration {
    interval.saturating_sub(since)
}

impl Pacer {
    /// Called at the end of a frame. Waits, if the frame came early, when the limit applies: `continuous` says that the editor is
    /// drawing every frame anyway (a game runs). An idle editor, or no limit, forgets the last frame and never waits.
    pub fn end_of_frame(&mut self, limit: FrameLimit, continuous: bool) {
        let Some(interval) = limit.interval().filter(|_| continuous) else {
            self.last = None;
            return;
        };
        if let Some(last) = self.last {
            let wait = wait_for(last.elapsed(), interval);
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
        self.last = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limits_are_the_rates_offered_and_a_frame_rate_has_its_interval() {
        assert_eq!(FrameLimit::ALL.iter().filter_map(|l| if let FrameLimit::Fps(f) = l { Some(*f) } else { None }).collect::<Vec<_>>(), [10, 30, 60, 140]);
        assert_eq!(FrameLimit::Display.interval(), None);
        assert_eq!(FrameLimit::Fps(10).interval(), Some(Duration::from_millis(100)));
        assert_eq!(FrameLimit::Fps(0).interval(), Some(Duration::from_secs(1)), "a rate of nothing is one a second, not a division by zero");
        assert_eq!(FrameLimit::Fps(30).to_string(), "30 fps");
    }

    #[test]
    fn the_editor_draws_continuously_while_a_game_runs_or_a_clip_plays_and_not_otherwise() {
        let mut app = crate::app::EditorApp::from_project(evolve_engine::project::Project::single(evolve_engine::scene::Scene::with_camera(evolve_engine::Vec2::new(100.0, 100.0))), 0);
        assert!(!app.draws_continuously());
        app.start_game();
        assert!(app.draws_continuously());
        app.stop_game();
        app.anim.playing = true;
        assert!(app.draws_continuously());
        app.anim.playing = false;
        assert!(!app.draws_continuously());
        assert_eq!(app.frame_limit, FrameLimit::Display, "no cap unless it is asked for");
    }

    #[test]
    fn a_frame_waits_for_what_is_left_of_its_interval_and_never_for_a_negative_time() {
        let interval = Duration::from_millis(33);
        assert_eq!(wait_for(Duration::from_millis(10), interval), Duration::from_millis(23));
        assert_eq!(wait_for(Duration::from_millis(33), interval), Duration::ZERO);
        assert_eq!(wait_for(Duration::from_millis(500), interval), Duration::ZERO);
    }

    #[test]
    fn frames_with_a_limit_come_no_faster_than_it_and_without_one_or_when_idle_nothing_waits() {
        let mut pacer = Pacer::default();
        let started = Instant::now();
        for _ in 0..6 {
            pacer.end_of_frame(FrameLimit::Fps(20), true); // 50 ms apart: the first sets the time, five wait
        }
        assert!(started.elapsed() >= Duration::from_millis(240), "{:?}", started.elapsed());

        for (limit, continuous) in [(FrameLimit::Display, true), (FrameLimit::Fps(10), false)] {
            let mut pacer = Pacer::default();
            let started = Instant::now();
            for _ in 0..6 {
                pacer.end_of_frame(limit, continuous);
            }
            assert!(started.elapsed() < Duration::from_millis(50), "{limit:?} {continuous}: {:?}", started.elapsed());
        }
        // After idle frames the first continuous frame does not wait for time that was spent doing nothing.
        let mut pacer = Pacer::default();
        pacer.end_of_frame(FrameLimit::Fps(10), true);
        pacer.end_of_frame(FrameLimit::Fps(10), false);
        let started = Instant::now();
        pacer.end_of_frame(FrameLimit::Fps(10), true);
        assert!(started.elapsed() < Duration::from_millis(50));
    }
}
