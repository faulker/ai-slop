use crate::hardware::{GpuVendor, HardwareProfile};
use crate::models::ModelSpec;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FitCategory {
    RunsGreat,
    RunsOk,
    RunsButSlow,
    WontFit,
}

impl std::fmt::Display for FitCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FitCategory::RunsGreat => write!(f, "Runs Great"),
            FitCategory::RunsOk => write!(f, "Runs OK"),
            FitCategory::RunsButSlow => write!(f, "Runs But Slow"),
            FitCategory::WontFit => write!(f, "Won't Fit"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelRecommendation {
    pub model: ModelSpec,
    pub fit: FitCategory,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct UpgradeRecommendation {
    pub priority: u8,
    pub component: String,
    pub suggestion: String,
    pub unlocks: String,
}

fn bytes_to_gb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

fn best_vram_gb(hw: &HardwareProfile) -> f64 {
    hw.gpus
        .iter()
        .filter_map(|g| g.vram_bytes)
        .max()
        .map(bytes_to_gb)
        .unwrap_or(0.0)
}

fn has_gpu_acceleration(hw: &HardwareProfile) -> bool {
    hw.gpus.iter().any(|g| {
        matches!(
            g.vendor,
            GpuVendor::Nvidia | GpuVendor::Amd | GpuVendor::AppleSilicon
        )
    })
}

fn is_apple_silicon(hw: &HardwareProfile) -> bool {
    hw.cpu.is_apple_silicon
}

pub fn categorize_model(model: &ModelSpec, hw: &HardwareProfile) -> ModelRecommendation {
    let vram_gb = best_vram_gb(hw);
    let ram_gb = bytes_to_gb(hw.memory.total_ram_bytes);
    let available_ram_gb = bytes_to_gb(hw.memory.available_ram_bytes);
    let has_accel = has_gpu_acceleration(hw);
    let apple_si = is_apple_silicon(hw);

    // Apple Silicon unified memory — treat as GPU memory with Metal acceleration
    if apple_si {
        // On Apple Silicon, ~75% of RAM is usable for model inference
        let usable_gb = ram_gb * 0.75;
        if usable_gb >= model.vram_required_gb * 1.1 {
            let headroom = usable_gb - model.vram_required_gb;
            return ModelRecommendation {
                model: model.clone(),
                fit: FitCategory::RunsGreat,
                reason: format!(
                    "Fits in unified memory with Metal acceleration ({:.1} GB headroom)",
                    headroom
                ),
            };
        } else if usable_gb >= model.vram_required_gb * 0.85 {
            return ModelRecommendation {
                model: model.clone(),
                fit: FitCategory::RunsOk,
                reason: "Tight fit in unified memory, may swap to disk".into(),
            };
        } else {
            return ModelRecommendation {
                model: model.clone(),
                fit: FitCategory::WontFit,
                reason: format!(
                    "Needs {:.1} GB, only {:.1} GB usable unified memory",
                    model.vram_required_gb, usable_gb
                ),
            };
        }
    }

    // Discrete GPU path
    if has_accel && vram_gb > 0.0 {
        if vram_gb >= model.vram_required_gb * 1.1 {
            let headroom = vram_gb - model.vram_required_gb;
            return ModelRecommendation {
                model: model.clone(),
                fit: FitCategory::RunsGreat,
                reason: format!(
                    "Fits in {:.1} GB VRAM with {:.1} GB headroom",
                    vram_gb, headroom
                ),
            };
        } else if vram_gb >= model.vram_required_gb * 0.85 {
            return ModelRecommendation {
                model: model.clone(),
                fit: FitCategory::RunsOk,
                reason: format!(
                    "Tight fit in {:.1} GB VRAM, some layers may offload to RAM",
                    vram_gb
                ),
            };
        }
    }

    // CPU-only fallback
    if available_ram_gb >= model.ram_required_gb {
        return ModelRecommendation {
            model: model.clone(),
            fit: FitCategory::RunsButSlow,
            reason: if has_accel {
                format!(
                    "Too large for {:.1} GB VRAM, will use CPU with {:.1} GB RAM (slow)",
                    vram_gb, available_ram_gb
                )
            } else {
                format!(
                    "CPU-only inference with {:.1} GB available RAM",
                    available_ram_gb
                )
            },
        };
    }

    // Total RAM check (some might free up)
    if ram_gb >= model.ram_required_gb {
        return ModelRecommendation {
            model: model.clone(),
            fit: FitCategory::RunsButSlow,
            reason: format!(
                "System has {:.1} GB total RAM (only {:.1} GB available now), may need to close other apps",
                ram_gb, available_ram_gb
            ),
        };
    }

    ModelRecommendation {
        model: model.clone(),
        fit: FitCategory::WontFit,
        reason: format!(
            "Needs {:.1} GB RAM, system has {:.1} GB total",
            model.ram_required_gb, ram_gb
        ),
    }
}

pub fn analyze_all(hw: &HardwareProfile) -> Vec<ModelRecommendation> {
    let models = crate::models::all_models();
    let mut recs: Vec<ModelRecommendation> = models
        .iter()
        .map(|m| categorize_model(m, hw))
        .collect();

    recs.sort_by(|a, b| a.fit.cmp(&b.fit));
    recs
}

pub fn suggest_upgrades(hw: &HardwareProfile) -> Vec<UpgradeRecommendation> {
    let mut upgrades = Vec::new();
    let vram_gb = best_vram_gb(hw);
    let ram_gb = bytes_to_gb(hw.memory.total_ram_bytes);
    let disk_gb = bytes_to_gb(hw.disk.available_bytes);
    let has_accel = has_gpu_acceleration(hw);
    let apple_si = is_apple_silicon(hw);

    if apple_si {
        // Apple Silicon upgrade suggestions
        let usable_gb = ram_gb * 0.75;
        if usable_gb < 6.0 {
            upgrades.push(UpgradeRecommendation {
                priority: 1,
                component: "Unified Memory".into(),
                suggestion: "Consider a Mac with 16GB+ unified memory".into(),
                unlocks: "Enables 7B models like Llama 3 and Mistral with Metal acceleration".into(),
            });
        } else if usable_gb < 12.0 {
            upgrades.push(UpgradeRecommendation {
                priority: 2,
                component: "Unified Memory".into(),
                suggestion: "Consider a Mac with 24-32GB unified memory".into(),
                unlocks: "Enables 13B+ models and more comfortable 7B inference".into(),
            });
        } else if usable_gb < 24.0 {
            upgrades.push(UpgradeRecommendation {
                priority: 3,
                component: "Unified Memory".into(),
                suggestion: "Consider a Mac with 48-64GB unified memory".into(),
                unlocks: "Enables 30B+ parameter models".into(),
            });
        }
    } else {
        // Linux / Intel Mac upgrade suggestions
        if !has_accel {
            upgrades.push(UpgradeRecommendation {
                priority: 1,
                component: "GPU".into(),
                suggestion: "Add a GPU with 8GB+ VRAM (e.g., RTX 3060 12GB, RTX 4060 8GB)".into(),
                unlocks: "Enables 7B models at full speed (10-50x faster than CPU-only)".into(),
            });
        } else if vram_gb < 8.0 {
            upgrades.push(UpgradeRecommendation {
                priority: 2,
                component: "GPU".into(),
                suggestion: format!("Upgrade GPU from {:.0} GB to 12-16GB VRAM (e.g., RTX 3060 12GB, RTX 4070 12GB)", vram_gb),
                unlocks: "Enables comfortable 7B inference and smaller 13B models".into(),
            });
        } else if vram_gb < 24.0 {
            upgrades.push(UpgradeRecommendation {
                priority: 3,
                component: "GPU".into(),
                suggestion: format!("Upgrade GPU from {:.0} GB to 24GB VRAM (e.g., RTX 3090, RTX 4090)", vram_gb),
                unlocks: "Enables 13B-30B+ parameter models at full GPU speed".into(),
            });
        }

        if ram_gb < 16.0 {
            upgrades.push(UpgradeRecommendation {
                priority: 2,
                component: "RAM".into(),
                suggestion: format!("Upgrade RAM from {:.0} GB to 32GB", ram_gb),
                unlocks: "Enables CPU-only inference for 7B models and better GPU offloading".into(),
            });
        } else if ram_gb < 32.0 && !has_accel {
            upgrades.push(UpgradeRecommendation {
                priority: 2,
                component: "RAM".into(),
                suggestion: format!("Upgrade RAM from {:.0} GB to 64GB", ram_gb),
                unlocks: "Enables CPU-only inference for 13B models".into(),
            });
        }

        if !hw.cpu.has_avx2 && !hw.cpu.is_apple_silicon {
            upgrades.push(UpgradeRecommendation {
                priority: 4,
                component: "CPU".into(),
                suggestion: "Your CPU lacks AVX2 instructions".into(),
                unlocks: "AVX2 is recommended for optimal inference performance; some backends require it".into(),
            });
        }
    }

    // Disk space (applies to all platforms)
    if disk_gb < 20.0 {
        upgrades.push(UpgradeRecommendation {
            priority: 3,
            component: "Disk".into(),
            suggestion: format!("Only {:.1} GB disk space available", disk_gb),
            unlocks: "Most 7B models need 4-5GB, larger models need 10-40GB+".into(),
        });
    }

    upgrades.sort_by_key(|u| u.priority);
    upgrades
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::*;

    fn make_profile(
        total_ram_gb: f64,
        available_ram_gb: f64,
        vram_gb: Option<f64>,
        apple_silicon: bool,
    ) -> HardwareProfile {
        let gb = |g: f64| (g * 1024.0 * 1024.0 * 1024.0) as u64;

        let gpus = if apple_silicon {
            vec![GpuInfo {
                vendor: GpuVendor::AppleSilicon,
                name: "Apple M1 GPU".into(),
                vram_bytes: Some((total_ram_gb * 0.75 * 1024.0 * 1024.0 * 1024.0) as u64),
                driver_version: None,
                cuda_version: None,
                metal_support: true,
            }]
        } else if let Some(vram) = vram_gb {
            vec![GpuInfo {
                vendor: GpuVendor::Nvidia,
                name: "Test GPU".into(),
                vram_bytes: Some(gb(vram)),
                driver_version: Some("535.0".into()),
                cuda_version: Some("12.2".into()),
                metal_support: false,
            }]
        } else {
            vec![]
        };

        HardwareProfile {
            cpu: CpuInfo {
                model_name: "Test CPU".into(),
                architecture: if apple_silicon { "arm64" } else { "x86_64" }.into(),
                cores: 8,
                threads: 16,
                has_avx: !apple_silicon,
                has_avx2: !apple_silicon,
                is_apple_silicon: apple_silicon,
            },
            gpus,
            memory: MemoryInfo {
                total_ram_bytes: gb(total_ram_gb),
                available_ram_bytes: gb(available_ram_gb),
                total_swap_bytes: gb(8.0),
                available_swap_bytes: gb(8.0),
                is_unified: apple_silicon,
            },
            disk: DiskInfo {
                total_bytes: gb(500.0),
                available_bytes: gb(200.0),
                storage_type: StorageType::NVMe,
            },
            os: "linux".into(),
            missing_tools: vec![],
        }
    }

    fn llama3_8b() -> crate::models::ModelSpec {
        crate::models::find_model("llama3:8b").unwrap()
    }

    fn llama3_70b() -> crate::models::ModelSpec {
        crate::models::find_model("llama3:70b").unwrap()
    }

    #[test]
    fn test_runs_great_with_sufficient_vram() {
        let hw = make_profile(32.0, 28.0, Some(12.0), false);
        let rec = categorize_model(&llama3_8b(), &hw);
        assert_eq!(rec.fit, FitCategory::RunsGreat);
    }

    #[test]
    fn test_runs_ok_with_tight_vram() {
        let hw = make_profile(32.0, 28.0, Some(5.0), false);
        let rec = categorize_model(&llama3_8b(), &hw);
        assert_eq!(rec.fit, FitCategory::RunsOk);
    }

    #[test]
    fn test_runs_but_slow_cpu_only() {
        let hw = make_profile(32.0, 28.0, None, false);
        let rec = categorize_model(&llama3_8b(), &hw);
        assert_eq!(rec.fit, FitCategory::RunsButSlow);
    }

    #[test]
    fn test_wont_fit_insufficient_ram() {
        let hw = make_profile(4.0, 3.0, None, false);
        let rec = categorize_model(&llama3_70b(), &hw);
        assert_eq!(rec.fit, FitCategory::WontFit);
    }

    #[test]
    fn test_apple_silicon_unified_memory() {
        let hw = make_profile(16.0, 12.0, None, true);
        let rec = categorize_model(&llama3_8b(), &hw);
        // 16 * 0.75 = 12.0 usable, 5.0 * 1.1 = 5.5, should be RunsGreat
        assert_eq!(rec.fit, FitCategory::RunsGreat);
    }

    #[test]
    fn test_apple_silicon_wont_fit() {
        let hw = make_profile(8.0, 6.0, None, true);
        let rec = categorize_model(&llama3_70b(), &hw);
        assert_eq!(rec.fit, FitCategory::WontFit);
    }

    #[test]
    fn test_upgrade_suggestions_no_gpu() {
        let hw = make_profile(16.0, 12.0, None, false);
        let upgrades = suggest_upgrades(&hw);
        assert!(upgrades.iter().any(|u| u.component == "GPU" && u.priority == 1));
    }

    #[test]
    fn test_upgrade_suggestions_low_ram() {
        let hw = make_profile(8.0, 6.0, Some(8.0), false);
        let upgrades = suggest_upgrades(&hw);
        assert!(upgrades.iter().any(|u| u.component == "RAM"));
    }

    #[test]
    fn test_upgrade_suggestions_apple_silicon_low_ram() {
        let hw = make_profile(8.0, 6.0, None, true);
        let upgrades = suggest_upgrades(&hw);
        assert!(upgrades.iter().any(|u| u.component == "Unified Memory"));
    }

    #[test]
    fn test_analyze_all_returns_sorted() {
        let hw = make_profile(16.0, 12.0, Some(8.0), false);
        let recs = analyze_all(&hw);
        for i in 1..recs.len() {
            assert!(recs[i].fit >= recs[i - 1].fit);
        }
    }
}
