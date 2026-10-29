//! Which dynamic bodies are stepped in a frame, following the project's [`PhysicsSettings`].
//!
//! Kept apart from the solver in [`crate::physics`] so that another solver can use it too: the solver says where its
//! dynamic bodies are and how fast they can go, and is told which of them to step in this frame and by how much time.
//!
//! With `slow_offscreen` off, every body is stepped every frame by the frame's time. With it on, a body is *in view*
//! when its bounds meet the camera's frame grown by the margin, and is stepped every frame. The others wait for their
//! turn, once every `offscreen_interval` frames, and are then stepped by all the time they waited. Turns are spread by
//! id, so about the same number of waiting bodies is stepped in every frame. A waiting body that a stepped body may
//! reach in this step is stepped too, so that the two meet in the same step (and so are the ones it may reach in turn).

use crate::geometry::Rect;
use crate::physics::PhysicsSettings;
use crate::scene::NodeId;
use std::collections::HashMap;

/// A dynamic body as the schedule sees it.
pub(crate) struct Candidate {
    pub id: NodeId,
    /// World bounds of what collides (for a circle, its square).
    pub bounds: Rect,
    /// Units per second it moves now.
    pub speed: f32,
    /// Units per second squared it can speed up by (gravity).
    pub acceleration: f32,
}

impl Candidate {
    /// How far it can move in `seconds`.
    fn reach(&self, seconds: f32) -> f32 {
        (self.speed + self.acceleration * seconds) * seconds
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Schedule {
    /// Seconds each waiting body has not been stepped for.
    waited: HashMap<NodeId, f32>,
    frame: u64,
}

impl Schedule {
    /// For each candidate, in order: the time to step it by in this frame of `dt` seconds, or `None` if it waits.
    /// `view` (the camera's frame in the world) is only asked for when the settings need it.
    pub(crate) fn plan(&mut self, settings: &PhysicsSettings, view: impl FnOnce() -> Rect, candidates: &[Candidate], dt: f32) -> Vec<Option<f32>> {
        let frame = self.frame;
        self.frame += 1;
        let interval = u64::from(settings.offscreen_interval.max(1));
        let owed: Vec<f32> = candidates.iter().map(|c| self.waited.get(&c.id).copied().unwrap_or(0.0) + dt).collect();
        if !slows_bodies(settings) {
            // (A body that was waiting when the rule was switched off gets the time it waited.)
            self.waited.clear();
            return owed.into_iter().map(Some).collect();
        }
        let area = active_area(settings, view());

        let mut stepped: Vec<bool> = candidates.iter().map(|c| c.bounds.intersects(&area) || (frame + c.id.0).is_multiple_of(interval)).collect();
        let mut reaching: Vec<usize> = (0..candidates.len()).filter(|&i| stepped[i]).collect();
        while let Some(i) = reaching.pop() {
            let reach = candidates[i].bounds.expanded(candidates[i].reach(owed[i]));
            for j in 0..candidates.len() {
                if !stepped[j] && reach.intersects(&candidates[j].bounds) {
                    stepped[j] = true;
                    reaching.push(j);
                }
            }
        }

        self.waited = candidates.iter().zip(&owed).zip(&stepped).filter(|(_, &s)| !s).map(|((c, &t), _)| (c.id, t)).collect();
        stepped.iter().zip(owed).map(|(&s, t)| s.then_some(t)).collect()
    }

    /// True if the body was not stepped in the last frame and waits for its turn.
    pub(crate) fn is_waiting(&self, id: NodeId) -> bool {
        self.waited.contains_key(&id)
    }
}

/// Where bodies count as in view: the camera's frame `view` grown by the margin on each side.
pub(crate) fn active_area(settings: &PhysicsSettings, view: Rect) -> Rect {
    let grow = view.size() * settings.margin.max(0.0);
    Rect { min: view.min - grow, max: view.max + grow }
}

/// True if the settings make some bodies wait.
pub(crate) fn slows_bodies(settings: &PhysicsSettings) -> bool {
    settings.slow_offscreen && settings.offscreen_interval > 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Vec2;

    const DT: f32 = 1.0 / 60.0;

    fn view() -> Rect {
        Rect { min: Vec2::ZERO, max: Vec2::new(100.0, 100.0) }
    }

    fn on() -> PhysicsSettings {
        PhysicsSettings { slow_offscreen: true, ..PhysicsSettings::default() }
    }

    /// A still body, 10 units wide, whose top-left corner is at `at`.
    fn body(id: u64, at: Vec2) -> Candidate {
        Candidate { id: NodeId(id), bounds: Rect { min: at, max: at + Vec2::splat(10.0) }, speed: 0.0, acceleration: 0.0 }
    }

    #[test]
    fn with_the_rule_off_every_body_is_stepped_every_frame() {
        let mut schedule = Schedule::default();
        let far = [body(1, Vec2::new(5000.0, 0.0))];
        for _ in 0..8 {
            assert_eq!(schedule.plan(&PhysicsSettings::default(), view, &far, DT), vec![Some(DT)]);
        }
    }

    #[test]
    fn a_body_out_of_view_waits_for_its_turn_and_gets_all_the_time_it_waited() {
        let mut schedule = Schedule::default();
        let bodies = [body(1, Vec2::new(40.0, 40.0)), body(2, Vec2::new(5000.0, 0.0))];
        let mut far = Vec::new();
        for _ in 0..8 {
            let plan = schedule.plan(&on(), view, &bodies, DT);
            assert_eq!(plan[0], Some(DT), "the body in view is stepped every frame");
            far.push(plan[1]);
        }
        let steps: Vec<f32> = far.iter().flatten().copied().collect();
        assert_eq!(steps.len(), 2, "once every 4 frames: {far:?}");
        assert!((steps[1] - 4.0 * DT).abs() < 1e-6, "by the 4 frames it waited: {far:?}");
    }

    #[test]
    fn the_margin_is_a_fraction_of_the_frame_on_each_side() {
        // The frame is 100 wide, so a margin of 0.5 reaches 50 units out on each side.
        let near = body(1, Vec2::new(140.0, 40.0));
        let beyond = body(2, Vec2::new(160.0, 40.0));
        let bodies = [near, beyond];
        let mut schedule = Schedule::default();
        let waits = (0..4).map(|_| schedule.plan(&on(), view, &bodies, DT)).filter(|plan| plan[1].is_none()).count();
        assert_eq!(waits, 3, "beyond the margin it waits 3 frames out of 4");
        let mut schedule = Schedule::default();
        assert!((0..4).all(|_| schedule.plan(&on(), view, &bodies, DT)[0].is_some()), "within the margin it is in view");
        let none = PhysicsSettings { margin: 0.0, ..on() };
        let mut schedule = Schedule::default();
        assert!((0..4).any(|_| schedule.plan(&none, view, &bodies, DT)[0].is_none()), "without a margin it is out of view");
    }

    #[test]
    fn turns_are_spread_over_the_frames() {
        let bodies: Vec<Candidate> = (1..=8).map(|id| body(id, Vec2::new(1000.0 * id as f32, 5000.0))).collect();
        let mut schedule = Schedule::default();
        for _ in 0..8 {
            let stepped = schedule.plan(&on(), view, &bodies, DT).iter().filter(|t| t.is_some()).count();
            assert_eq!(stepped, 2, "8 waiting bodies, a quarter of them in each frame");
        }
    }

    #[test]
    fn a_waiting_body_is_stepped_when_a_stepped_body_may_reach_it_and_so_on() {
        // A fast body in view heads for a waiting one, which touches another waiting one. A fourth is far from all.
        let mut fast = body(1, Vec2::new(90.0, 40.0));
        fast.speed = 3000.0; // 50 units in a frame
        let bodies = [fast, body(2, Vec2::new(150.0, 40.0)), body(3, Vec2::new(160.0, 40.0)), body(4, Vec2::new(900.0, 40.0))];
        let settings = PhysicsSettings { margin: 0.0, ..on() };
        let mut schedule = Schedule::default();
        for _ in 0..4 {
            let plan = schedule.plan(&settings, view, &bodies, DT);
            assert!(plan[1].is_some() && plan[2].is_some(), "{plan:?}");
        }
        let mut schedule = Schedule::default();
        assert!((0..4).any(|_| schedule.plan(&settings, view, &bodies, DT)[3].is_none()), "the far one still waits");
    }

    #[test]
    fn switching_the_rule_off_gives_waiting_bodies_the_time_they_waited() {
        let mut schedule = Schedule::default();
        let far = [body(1, Vec2::new(5000.0, 0.0))];
        // Body 1 has its turn in frame 3; in frames 0 to 2 it waits.
        for _ in 0..3 {
            assert_eq!(schedule.plan(&on(), view, &far, DT), vec![None]);
        }
        let plan = schedule.plan(&PhysicsSettings::default(), view, &far, DT);
        assert!((plan[0].unwrap() - 4.0 * DT).abs() < 1e-6, "{plan:?}");
        assert_eq!(schedule.plan(&PhysicsSettings::default(), view, &far, DT), vec![Some(DT)]);
    }

    #[test]
    fn an_interval_of_one_or_zero_steps_every_body_every_frame() {
        for interval in [0, 1] {
            let settings = PhysicsSettings { offscreen_interval: interval, ..on() };
            let mut schedule = Schedule::default();
            let far = [body(1, Vec2::new(5000.0, 0.0))];
            assert!((0..4).all(|_| schedule.plan(&settings, view, &far, DT) == vec![Some(DT)]));
        }
    }
}
