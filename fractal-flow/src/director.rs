//! Scene timing: when to prepare the next scene, when to crossfade, and how far
//! along the crossfade is. Pure state machine, no GPU or threads.

/// What the app should do after a `Director::update`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectorEvent {
    None,
    /// Start preparing the next scene now (it may take a while, e.g. deep zoom targets).
    RequestNext,
    /// The prepared scene becomes the incoming scene; start blending it in.
    BeginTransition,
    /// Blend finished; the incoming scene is now the current scene.
    FinishTransition,
}

/// Scene scheduler.
#[derive(Clone, Debug)]
pub struct Director {
    duration: f32,
    crossfade: f32,
    scene_time: f32,
    transition: Option<f32>,
    skip: bool,
    requested: bool,
}

impl Director {
    /// New director; the first scene fades in as soon as it is ready.
    pub fn new(duration: f32, crossfade: f32) -> Self {
        let mut d = Self { duration, crossfade, scene_time: 0.0, transition: None, skip: true, requested: false };
        d.set_timing(duration, crossfade);
        d
    }

    /// Change scene length and crossfade length (takes effect immediately).
    pub fn set_timing(&mut self, duration: f32, crossfade: f32) {
        self.duration = duration.max(0.1);
        self.crossfade = crossfade.clamp(0.0, self.duration);
    }

    /// Ask to move on to the next scene as soon as it is ready.
    /// During a transition this finishes the transition immediately instead.
    pub fn skip(&mut self) {
        if let Some(p) = self.transition.as_mut() {
            *p = self.crossfade;
        } else {
            self.skip = true;
        }
    }

    /// Advance by `dt` seconds. `next_ready` says whether a prepared scene is waiting.
    pub fn update(&mut self, dt: f32, next_ready: bool) -> DirectorEvent {
        self.scene_time += dt;
        if let Some(p) = self.transition.as_mut() {
            *p += dt;
            if *p >= self.crossfade {
                self.scene_time = *p;
                self.transition = None;
                self.requested = false;
                return DirectorEvent::FinishTransition;
            }
            return DirectorEvent::None;
        }
        if !self.requested {
            self.requested = true;
            return DirectorEvent::RequestNext;
        }
        if (self.skip || self.scene_time >= self.duration) && next_ready {
            self.skip = false;
            self.transition = Some(0.0);
            return DirectorEvent::BeginTransition;
        }
        DirectorEvent::None
    }

    /// Crossfade weight of the incoming scene, 0..1 (smoothstepped). 0 when not transitioning.
    pub fn blend(&self) -> f32 {
        match self.transition {
            None => 0.0,
            Some(_) if self.crossfade <= 0.0 => 1.0,
            Some(p) => {
                let x = (p / self.crossfade).clamp(0.0, 1.0);
                x * x * (3.0 - 2.0 * x)
            }
        }
    }

    /// True while two scenes are being blended.
    pub fn in_transition(&self) -> bool {
        self.transition.is_some()
    }

    /// Seconds since the current scene started.
    pub fn scene_time(&self) -> f32 {
        self.scene_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run updates until `event` happens, returning the elapsed time.
    fn run_until(d: &mut Director, event: DirectorEvent, ready: bool, max: f32) -> f32 {
        let mut t = 0.0;
        while t < max {
            t += 0.1;
            if d.update(0.1, ready) == event {
                return t;
            }
        }
        panic!("{event:?} did not happen within {max}s");
    }

    #[test]
    fn requests_next_immediately_and_fades_in_first_scene() {
        let mut d = Director::new(10.0, 2.0);
        assert_eq!(d.update(0.1, false), DirectorEvent::RequestNext);
        // Nothing happens until the scene is ready.
        for _ in 0..50 {
            assert_eq!(d.update(0.1, false), DirectorEvent::None);
        }
        assert_eq!(d.update(0.1, true), DirectorEvent::BeginTransition);
    }

    #[test]
    fn transition_starts_at_interval() {
        let mut d = Director::new(10.0, 2.0);
        d.update(0.1, false);
        run_until(&mut d, DirectorEvent::BeginTransition, true, 1.0);
        run_until(&mut d, DirectorEvent::FinishTransition, true, 5.0);
        assert_eq!(d.update(0.1, true), DirectorEvent::RequestNext);
        let t = run_until(&mut d, DirectorEvent::BeginTransition, true, 20.0);
        assert!((t - 7.9).abs() < 0.25, "began after {t}s (scene clock includes the 2s fade)");
    }

    #[test]
    fn blend_is_monotonic_zero_to_one() {
        let mut d = Director::new(10.0, 2.0);
        d.update(0.1, false);
        run_until(&mut d, DirectorEvent::BeginTransition, true, 1.0);
        let mut last = d.blend();
        assert_eq!(last, 0.0);
        while d.in_transition() {
            d.update(0.05, true);
            let b = if d.in_transition() { d.blend() } else { 1.0 };
            assert!(b >= last, "blend went backwards: {last} -> {b}");
            last = b;
        }
        assert_eq!(last, 1.0);
        assert_eq!(d.blend(), 0.0);
    }

    #[test]
    fn waits_for_next_scene_past_interval() {
        let mut d = Director::new(1.0, 0.5);
        d.update(0.1, false);
        run_until(&mut d, DirectorEvent::BeginTransition, true, 1.0);
        run_until(&mut d, DirectorEvent::FinishTransition, true, 2.0);
        d.update(0.1, false);
        for _ in 0..100 {
            assert_eq!(d.update(0.1, false), DirectorEvent::None);
        }
        assert_eq!(d.update(0.1, true), DirectorEvent::BeginTransition);
    }

    #[test]
    fn skip_mid_scene_and_mid_transition() {
        let mut d = Director::new(100.0, 4.0);
        d.update(0.1, false);
        run_until(&mut d, DirectorEvent::BeginTransition, true, 1.0);
        run_until(&mut d, DirectorEvent::FinishTransition, true, 10.0);
        d.update(0.1, true);
        d.skip();
        assert_eq!(d.update(0.1, true), DirectorEvent::BeginTransition);
        d.skip();
        assert_eq!(d.update(0.01, true), DirectorEvent::FinishTransition);
    }

    #[test]
    fn zero_crossfade_cuts() {
        let mut d = Director::new(5.0, 0.0);
        d.update(0.1, false);
        assert_eq!(d.update(0.1, true), DirectorEvent::BeginTransition);
        assert_eq!(d.blend(), 1.0);
        assert_eq!(d.update(0.1, true), DirectorEvent::FinishTransition);
    }
}
