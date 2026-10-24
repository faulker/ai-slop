//! Runtime performance: process CPU/memory sampling for the debug overlay, and
//! the auto-quality controller that lowers render cost when FPS drops.
//!
//! Auto quality never edits the saved settings. It picks a degradation level,
//! and `degrade` derives cheaper render settings from the user's own choice,
//! so the user's preset is always the ceiling.

use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::settings::Settings;

/// Deepest degradation level auto quality will go to.
pub const MAX_LEVEL: u8 = 6;

/// FPS below this fraction of the target counts as "too slow".
const LOW_RATIO: f32 = 0.9;
/// FPS at or above this fraction of the target counts as "keeping up".
const HIGH_RATIO: f32 = 0.97;
/// Seconds of sustained low FPS before stepping down.
const DOWN_AFTER: f32 = 1.5;
/// Seconds of sustained good FPS before trying one level up (doubles on each bounce).
const UP_AFTER: f32 = 5.0;
/// Longest wait before retrying a level that was too slow.
const UP_AFTER_MAX: f32 = 120.0;
/// A step down this soon after a step up means the higher level can't hold.
const BOUNCE_WINDOW: f32 = 10.0;
/// Seconds to ignore after any change while the new level settles.
const SETTLE: f32 = 1.5;

/// Picks a degradation level from measured FPS, with hysteresis so it doesn't oscillate.
#[derive(Debug, Clone)]
pub struct AutoQuality {
    level: u8,
    low_secs: f32,
    high_secs: f32,
    settle_secs: f32,
    up_after: f32,
    since_up: Option<f32>,
}

impl Default for AutoQuality {
    /// Full quality, no history.
    fn default() -> Self {
        Self { level: 0, low_secs: 0.0, high_secs: 0.0, settle_secs: 0.0, up_after: UP_AFTER, since_up: None }
    }
}

impl AutoQuality {
    /// Current degradation level (0 = the user's settings unchanged).
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Forget history and go back to full quality.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed one FPS measurement covering `dt` seconds. Returns true if the level changed.
    pub fn update(&mut self, fps: f32, target: f32, dt: f32) -> bool {
        if let Some(t) = self.since_up.as_mut() {
            *t += dt;
        }
        if self.settle_secs > 0.0 {
            self.settle_secs -= dt;
            return false;
        }
        if fps < target * LOW_RATIO {
            self.low_secs += dt;
            self.high_secs = 0.0;
        } else if fps >= target * HIGH_RATIO {
            self.high_secs += dt;
            self.low_secs = 0.0;
        } else {
            self.low_secs = 0.0;
            self.high_secs = 0.0;
        }

        if self.low_secs >= DOWN_AFTER && self.level < MAX_LEVEL {
            if self.since_up.is_some_and(|t| t < BOUNCE_WINDOW) {
                self.up_after = (self.up_after * 2.0).min(UP_AFTER_MAX);
            }
            self.level += 1;
            self.since_up = None;
            self.changed();
            return true;
        }
        if self.high_secs >= self.up_after && self.level > 0 {
            self.level -= 1;
            self.since_up = Some(0.0);
            if self.level == 0 {
                self.up_after = UP_AFTER;
            }
            self.changed();
            return true;
        }
        false
    }

    /// Clear the accumulators and start the settle period after a level change.
    fn changed(&mut self) {
        self.low_secs = 0.0;
        self.high_secs = 0.0;
        self.settle_secs = SETTLE;
    }
}

/// FPS auto quality aims for: the user's target, but never above an FPS cap.
pub fn target_fps(settings: &Settings) -> f32 {
    let target = settings.quality.target_fps as f32;
    match settings.display.fps_cap {
        0 => target,
        cap => target.min(cap as f32),
    }
}

/// Render settings for a degradation level. Each level is cheaper than the last:
/// fewer samples, then fewer iterations and march steps, then lower render scale.
pub fn degrade(settings: &Settings, level: u8) -> Settings {
    let mut s = settings.clone();
    if level == 0 {
        return s;
    }
    let l = level as i32;
    let q = &mut s.quality.params;
    let cost = 0.85f32.powi(l);
    q.supersample = q.supersample.saturating_sub(level as u32).max(1);
    q.shadows = false;
    q.ao &= level < 3;
    q.base_iterations = (q.base_iterations as f32 * cost) as u32;
    q.max_iterations = (q.max_iterations as f32 * cost) as u32;
    q.march_steps = (q.march_steps as f32 * cost) as u32;
    q.fold_iterations = q.fold_iterations.saturating_sub(level as u32 / 2);
    q.clamp();
    let scale = (1.0 - 0.1 * (l - 1) as f32).max(0.5);
    s.display.render_scale = (s.display.render_scale * scale).max(0.25);
    s
}

/// One reading of this process's resource use.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProcessSample {
    /// CPU use in percent of one core (can exceed 100 on multi-core machines).
    pub cpu_percent: f32,
    /// Resident memory in bytes.
    pub memory_bytes: u64,
    /// Logical CPU cores, for putting `cpu_percent` in context.
    pub cores: usize,
}

/// Samples this process's CPU and memory at most every `interval`.
pub struct ProcessMonitor {
    sys: System,
    pid: Option<Pid>,
    interval: Duration,
    last: Option<Instant>,
    sample: ProcessSample,
}

impl ProcessMonitor {
    /// Monitor for the current process. If the pid can't be read, samples stay zero.
    pub fn new(interval: Duration) -> Self {
        let pid = match sysinfo::get_current_pid() {
            Ok(p) => Some(p),
            Err(e) => {
                log::warn!("process stats unavailable: {e}");
                None
            }
        };
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        Self { sys: System::new(), pid, interval, last: None, sample: ProcessSample { cores, ..Default::default() } }
    }

    /// Refresh if the interval has passed, and return the latest sample.
    /// CPU reads 0 until two refreshes have happened.
    pub fn sample(&mut self, now: Instant) -> ProcessSample {
        let Some(pid) = self.pid else { return self.sample };
        if self.last.is_some_and(|t| now - t < self.interval) {
            return self.sample;
        }
        self.last = Some(now);
        self.sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, ProcessRefreshKind::nothing().with_cpu().with_memory());
        if let Some(p) = self.sys.process(pid) {
            self.sample.cpu_percent = p.cpu_usage();
            self.sample.memory_bytes = p.memory();
        }
        self.sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::QualityPreset;

    /// Feed `secs` seconds of a constant FPS in 0.5 s samples; returns how many level changes happened.
    fn run(aq: &mut AutoQuality, fps: f32, secs: f32) -> usize {
        (0..(secs / 0.5) as usize).filter(|_| aq.update(fps, 60.0, 0.5)).count()
    }

    #[test]
    fn steps_down_on_sustained_low_fps_only() {
        let mut aq = AutoQuality::default();
        run(&mut aq, 30.0, 1.0);
        assert_eq!(aq.level(), 0, "one second of low fps is a blip");
        run(&mut aq, 30.0, 1.0);
        assert_eq!(aq.level(), 1);
        run(&mut aq, 30.0, 1.0);
        assert_eq!(aq.level(), 1, "settle period after a change");
    }

    #[test]
    fn saturates_at_max_level() {
        let mut aq = AutoQuality::default();
        run(&mut aq, 5.0, 120.0);
        assert_eq!(aq.level(), MAX_LEVEL);
    }

    #[test]
    fn steps_back_up_when_fps_recovers() {
        let mut aq = AutoQuality::default();
        run(&mut aq, 30.0, 2.0);
        assert_eq!(aq.level(), 1);
        run(&mut aq, 60.0, 8.0);
        assert_eq!(aq.level(), 0);
    }

    #[test]
    fn in_between_fps_holds_level() {
        let mut aq = AutoQuality::default();
        run(&mut aq, 30.0, 2.0);
        run(&mut aq, 56.0, 60.0);
        assert_eq!(aq.level(), 1);
    }

    #[test]
    fn bouncing_backs_off_retries() {
        let mut aq = AutoQuality::default();
        run(&mut aq, 30.0, 2.0);
        run(&mut aq, 30.0, 4.0);
        assert_eq!(aq.level(), 2);
        run(&mut aq, 60.0, 7.0);
        assert_eq!(aq.level(), 1);
        // Level 1 can't hold: drops back within the bounce window, so the next retry waits longer.
        run(&mut aq, 30.0, 2.0);
        assert_eq!(aq.level(), 2);
        assert_eq!(run(&mut aq, 60.0, 7.0), 0, "retry delay doubled");
        run(&mut aq, 60.0, 5.0);
        assert_eq!(aq.level(), 1);
    }

    #[test]
    fn target_respects_fps_cap() {
        let mut s = Settings::default();
        s.quality.target_fps = 60;
        assert_eq!(target_fps(&s), 60.0);
        s.display.fps_cap = 30;
        assert_eq!(target_fps(&s), 30.0);
    }

    #[test]
    fn degrade_level_zero_is_identity() {
        let s = Settings::default();
        assert_eq!(degrade(&s, 0), s);
    }

    #[test]
    fn degrade_gets_cheaper_every_level() {
        let mut s = Settings::default();
        s.set_quality_preset(QualityPreset::Ultra);
        let cost = |s: &Settings| {
            let q = &s.quality.params;
            let px = s.display.render_scale.powi(2) * (q.supersample * q.supersample) as f32;
            px * (q.base_iterations + q.march_steps) as f32 * if q.shadows { 2.0 } else { 1.0 }
        };
        let mut prev = cost(&s);
        for level in 1..=MAX_LEVEL {
            let d = degrade(&s, level);
            let c = cost(&d);
            assert!(c < prev, "level {level} should be cheaper");
            assert!(d.display.render_scale >= 0.25 && d.quality.params.supersample >= 1);
            prev = c;
        }
        assert_eq!(s.quality.preset, QualityPreset::Ultra, "input settings untouched");
    }

    #[test]
    fn process_monitor_reports_memory() {
        let mut m = ProcessMonitor::new(Duration::ZERO);
        let s = m.sample(Instant::now());
        assert!(s.memory_bytes > 0);
        assert!(s.cores >= 1);
    }
}
