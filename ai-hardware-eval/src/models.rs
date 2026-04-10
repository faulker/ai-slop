#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModelCategory {
    General,
    Code,
    Small,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ModelSpec {
    pub name: String,
    pub family: String,
    pub parameter_count_b: f64,
    pub default_quant: String,
    pub vram_required_gb: f64,
    pub ram_required_gb: f64,
    pub disk_size_gb: f64,
    pub description: String,
    pub category: ModelCategory,
}

impl std::fmt::Display for ModelCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelCategory::General => write!(f, "General"),
            ModelCategory::Code => write!(f, "Code"),
            ModelCategory::Small => write!(f, "Small"),
        }
    }
}

struct ModelEntry {
    name: &'static str,
    family: &'static str,
    parameter_count_b: f64,
    default_quant: &'static str,
    vram_required_gb: f64,
    ram_required_gb: f64,
    disk_size_gb: f64,
    description: &'static str,
    category: ModelCategory,
}

impl ModelEntry {
    fn to_spec(&self) -> ModelSpec {
        ModelSpec {
            name: self.name.to_string(),
            family: self.family.to_string(),
            parameter_count_b: self.parameter_count_b,
            default_quant: self.default_quant.to_string(),
            vram_required_gb: self.vram_required_gb,
            ram_required_gb: self.ram_required_gb,
            disk_size_gb: self.disk_size_gb,
            description: self.description.to_string(),
            category: self.category,
        }
    }
}

const MODEL_ENTRIES: &[ModelEntry] = &[
    ModelEntry {
        name: "tinyllama:1.1b",
        family: "tinyllama",
        parameter_count_b: 1.1,
        default_quant: "Q4_K_M",
        vram_required_gb: 0.8,
        ram_required_gb: 1.5,
        disk_size_gb: 0.6,
        description: "Tiny model, good for testing and very constrained hardware",
        category: ModelCategory::Small,
    },
    ModelEntry {
        name: "gemma2:2b",
        family: "gemma2",
        parameter_count_b: 2.6,
        default_quant: "Q4_K_M",
        vram_required_gb: 1.8,
        ram_required_gb: 3.0,
        disk_size_gb: 1.6,
        description: "Google's small model, good quality for its size",
        category: ModelCategory::Small,
    },
    ModelEntry {
        name: "llama3.2:3b",
        family: "llama3.2",
        parameter_count_b: 3.2,
        default_quant: "Q4_K_M",
        vram_required_gb: 2.1,
        ram_required_gb: 4.0,
        disk_size_gb: 2.0,
        description: "Meta's latest small model, great general-purpose",
        category: ModelCategory::General,
    },
    ModelEntry {
        name: "phi3:mini",
        family: "phi3",
        parameter_count_b: 3.8,
        default_quant: "Q4_K_M",
        vram_required_gb: 2.5,
        ram_required_gb: 4.5,
        disk_size_gb: 2.3,
        description: "Microsoft's compact model with strong reasoning",
        category: ModelCategory::Small,
    },
    ModelEntry {
        name: "codellama:7b",
        family: "codellama",
        parameter_count_b: 6.7,
        default_quant: "Q4_K_M",
        vram_required_gb: 4.2,
        ram_required_gb: 7.5,
        disk_size_gb: 3.8,
        description: "Meta's code-specialized model",
        category: ModelCategory::Code,
    },
    ModelEntry {
        name: "mistral:7b",
        family: "mistral",
        parameter_count_b: 7.2,
        default_quant: "Q4_K_M",
        vram_required_gb: 4.5,
        ram_required_gb: 8.0,
        disk_size_gb: 4.1,
        description: "Excellent 7B model, strong all-around performance",
        category: ModelCategory::General,
    },
    ModelEntry {
        name: "qwen2.5:7b",
        family: "qwen2.5",
        parameter_count_b: 7.6,
        default_quant: "Q4_K_M",
        vram_required_gb: 4.8,
        ram_required_gb: 8.5,
        disk_size_gb: 4.4,
        description: "Alibaba's strong multilingual model",
        category: ModelCategory::General,
    },
    ModelEntry {
        name: "qwen2.5-coder:7b",
        family: "qwen2.5-coder",
        parameter_count_b: 7.6,
        default_quant: "Q4_K_M",
        vram_required_gb: 4.8,
        ram_required_gb: 8.5,
        disk_size_gb: 4.4,
        description: "Alibaba's code model, strong at code generation",
        category: ModelCategory::Code,
    },
    ModelEntry {
        name: "llama3:8b",
        family: "llama3",
        parameter_count_b: 8.0,
        default_quant: "Q4_K_M",
        vram_required_gb: 5.0,
        ram_required_gb: 9.0,
        disk_size_gb: 4.7,
        description: "Meta's flagship 8B model, top-tier at this size",
        category: ModelCategory::General,
    },
    ModelEntry {
        name: "gemma2:9b",
        family: "gemma2",
        parameter_count_b: 9.2,
        default_quant: "Q4_K_M",
        vram_required_gb: 5.7,
        ram_required_gb: 10.0,
        disk_size_gb: 5.4,
        description: "Google's 9B model, competitive with larger models",
        category: ModelCategory::General,
    },
    ModelEntry {
        name: "deepseek-coder-v2:16b",
        family: "deepseek-coder-v2",
        parameter_count_b: 15.7,
        default_quant: "Q4_K_M",
        vram_required_gb: 9.5,
        ram_required_gb: 17.0,
        disk_size_gb: 8.9,
        description: "DeepSeek's large code model, excellent for programming",
        category: ModelCategory::Code,
    },
    ModelEntry {
        name: "llama3:70b",
        family: "llama3",
        parameter_count_b: 70.6,
        default_quant: "Q4_K_M",
        vram_required_gb: 40.0,
        ram_required_gb: 75.0,
        disk_size_gb: 39.0,
        description: "Meta's largest open model, near GPT-4 quality",
        category: ModelCategory::General,
    },
];

pub fn all_models() -> Vec<ModelSpec> {
    MODEL_ENTRIES.iter().map(|e| e.to_spec()).collect()
}

pub fn find_model(name: &str) -> Option<ModelSpec> {
    let name_lower = name.to_lowercase();
    MODEL_ENTRIES.iter().find(|e| {
        let entry_name = e.name.to_lowercase();
        // Exact match
        entry_name == name_lower
            // Match without tag (e.g., "mistral" matches "mistral:7b")
            || entry_name.split(':').next().unwrap_or("") == name_lower
            // Match with "latest" tag (e.g., "mistral:latest" matches "mistral:7b")
            || (name_lower.ends_with(":latest")
                && name_lower.trim_end_matches(":latest") == entry_name.split(':').next().unwrap_or(""))
    }).map(|e| e.to_spec())
}

/// Estimate requirements from parameter count (at Q4 quantization).
/// ~0.65 GB VRAM per billion params, ~1.1 GB RAM per billion params.
pub fn estimate_from_params(name: &str, parameter_count_b: f64, disk_size_gb: f64) -> ModelSpec {
    let vram = parameter_count_b * 0.65;
    let ram = parameter_count_b * 1.1;
    let category = if name.contains("code") || name.contains("coder") {
        ModelCategory::Code
    } else if parameter_count_b < 4.0 {
        ModelCategory::Small
    } else {
        ModelCategory::General
    };

    ModelSpec {
        name: name.to_string(),
        family: name.split(':').next().unwrap_or(name).to_string(),
        parameter_count_b,
        default_quant: "Q4 (estimated)".to_string(),
        vram_required_gb: vram,
        ram_required_gb: ram,
        disk_size_gb,
        description: format!("Estimated from {:.1}B parameters", parameter_count_b),
        category,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_database_not_empty() {
        assert!(!all_models().is_empty());
    }

    #[test]
    fn test_models_sorted_by_size() {
        let models = all_models();
        for i in 1..models.len() {
            assert!(
                models[i].vram_required_gb >= models[i - 1].vram_required_gb,
                "Models should be sorted by VRAM requirement: {} ({}) < {} ({})",
                models[i].name,
                models[i].vram_required_gb,
                models[i - 1].name,
                models[i - 1].vram_required_gb
            );
        }
    }

    #[test]
    fn test_ram_exceeds_vram() {
        for model in all_models() {
            assert!(
                model.ram_required_gb > model.vram_required_gb,
                "RAM requirement should exceed VRAM for {}: ram={} vram={}",
                model.name,
                model.ram_required_gb,
                model.vram_required_gb
            );
        }
    }

    #[test]
    fn test_find_model_exact() {
        let m = find_model("llama3:8b");
        assert!(m.is_some());
        assert_eq!(m.unwrap().name, "llama3:8b");
    }

    #[test]
    fn test_find_model_without_tag() {
        let m = find_model("mistral");
        assert!(m.is_some());
        assert_eq!(m.unwrap().name, "mistral:7b");
    }

    #[test]
    fn test_find_model_with_latest() {
        let m = find_model("mistral:latest");
        assert!(m.is_some());
        assert_eq!(m.unwrap().name, "mistral:7b");
    }

    #[test]
    fn test_find_model_not_found() {
        assert!(find_model("nonexistent-model:99b").is_none());
    }

    #[test]
    fn test_estimate_from_params() {
        let spec = estimate_from_params("test-model:13b", 13.0, 7.0);
        assert!((spec.vram_required_gb - 8.45).abs() < 0.01);
        assert!((spec.ram_required_gb - 14.3).abs() < 0.01);
        assert_eq!(spec.category, ModelCategory::General);
    }

    #[test]
    fn test_estimate_code_model() {
        let spec = estimate_from_params("deepseek-coder:6.7b", 6.7, 4.0);
        assert_eq!(spec.category, ModelCategory::Code);
    }
}
