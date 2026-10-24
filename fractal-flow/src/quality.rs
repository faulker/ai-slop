//! Quality presets: how much GPU work each frame gets.
//!
//! Detail comes from iteration counts, supersampling and raymarch steps, so
//! these are the knobs that trade sharpness against frame rate.

use serde::{Deserialize, Serialize};

/// Named quality level. `Custom` means the user edited individual values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum QualityPreset {
    Low,
    Medium,
    High,
    Ultra,
    Custom,
}

/// Individual render-cost parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QualityParams {
    /// Samples per pixel axis (N means N×N jittered samples).
    pub supersample: u32,
    /// Escape-time iterations at the start of a zoom.
    pub base_iterations: u32,
    /// Extra iterations per decade of zoom, as a fraction of `base_iterations`.
    pub iteration_growth: f32,
    /// Hard cap on iterations, however deep the zoom gets.
    pub max_iterations: u32,
    /// Raymarch step budget for 3D scenes.
    pub march_steps: u32,
    /// Fractal fold iterations for 3D distance estimators.
    pub fold_iterations: u32,
    /// Ambient occlusion in 3D scenes.
    pub ao: bool,
    /// Soft shadows in 3D scenes (expensive).
    pub shadows: bool,
}

impl Default for QualityParams {
    /// Defaults to the High preset.
    fn default() -> Self {
        QualityPreset::High.params().expect("High is a concrete preset")
    }
}

impl QualityParams {
    /// Iteration budget at a given zoom depth (decades below the start view).
    pub fn iterations_at_depth(&self, decades: f64) -> u32 {
        let n = self.base_iterations as f64 * (1.0 + self.iteration_growth as f64 * decades.max(0.0));
        (n as u32).clamp(16, self.max_iterations.max(16))
    }

    /// Clamp every field to a sane range.
    pub fn clamp(&mut self) {
        self.supersample = self.supersample.clamp(1, 4);
        self.base_iterations = self.base_iterations.clamp(64, 20_000);
        self.iteration_growth = self.iteration_growth.clamp(0.0, 2.0);
        self.max_iterations = self.max_iterations.clamp(self.base_iterations, 100_000);
        self.march_steps = self.march_steps.clamp(32, 1_000);
        self.fold_iterations = self.fold_iterations.clamp(4, 30);
    }
}

impl QualityPreset {
    /// Concrete parameters for a preset; `None` for `Custom`.
    pub fn params(self) -> Option<QualityParams> {
        let p = |supersample, base_iterations, iteration_growth, max_iterations, march_steps, fold_iterations, ao, shadows| QualityParams {
            supersample,
            base_iterations,
            iteration_growth,
            max_iterations,
            march_steps,
            fold_iterations,
            ao,
            shadows,
        };
        match self {
            Self::Low => Some(p(1, 400, 0.25, 4_000, 120, 10, false, false)),
            Self::Medium => Some(p(1, 600, 0.3, 8_000, 200, 12, true, false)),
            Self::High => Some(p(2, 800, 0.35, 20_000, 300, 14, true, false)),
            Self::Ultra => Some(p(3, 1_200, 0.45, 50_000, 450, 18, true, true)),
            Self::Custom => None,
        }
    }

    /// Next preset up (`up = true`) or down; `Custom` steps from High.
    pub fn step(self, up: bool) -> Self {
        const ORDER: [QualityPreset; 4] = [QualityPreset::Low, QualityPreset::Medium, QualityPreset::High, QualityPreset::Ultra];
        let i = ORDER.iter().position(|&q| q == self).unwrap_or(2);
        let j = if up { (i + 1).min(3) } else { i.saturating_sub(1) };
        ORDER[j]
    }

    /// Human-readable label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
            Self::Custom => "Custom",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterations_grow_with_depth_and_respect_cap() {
        let q = QualityPreset::High.params().unwrap();
        assert_eq!(q.iterations_at_depth(0.0), 800);
        assert!(q.iterations_at_depth(10.0) > q.iterations_at_depth(5.0));
        assert_eq!(q.iterations_at_depth(10_000.0), q.max_iterations);
    }

    #[test]
    fn presets_get_more_expensive() {
        let all: Vec<_> = [QualityPreset::Low, QualityPreset::Medium, QualityPreset::High, QualityPreset::Ultra]
            .iter()
            .map(|p| p.params().unwrap())
            .collect();
        for w in all.windows(2) {
            assert!(w[1].base_iterations > w[0].base_iterations);
            assert!(w[1].march_steps > w[0].march_steps);
            assert!(w[1].supersample >= w[0].supersample);
        }
        assert!(QualityPreset::Custom.params().is_none());
    }

    #[test]
    fn step_moves_and_saturates() {
        assert_eq!(QualityPreset::Low.step(false), QualityPreset::Low);
        assert_eq!(QualityPreset::Low.step(true), QualityPreset::Medium);
        assert_eq!(QualityPreset::Ultra.step(true), QualityPreset::Ultra);
        assert_eq!(QualityPreset::Custom.step(true), QualityPreset::Ultra);
    }

    #[test]
    fn clamp_fixes_bad_values() {
        let mut q = QualityParams { supersample: 99, base_iterations: 1, iteration_growth: -1.0, max_iterations: 0, march_steps: 0, fold_iterations: 100, ao: true, shadows: true };
        q.clamp();
        assert_eq!(q.supersample, 4);
        assert_eq!(q.base_iterations, 64);
        assert_eq!(q.iteration_growth, 0.0);
        assert!(q.max_iterations >= q.base_iterations);
        assert_eq!(q.march_steps, 32);
        assert_eq!(q.fold_iterations, 30);
    }
}
