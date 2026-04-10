mod analysis;
mod hardware;
mod models;
mod ollama;

use clap::Parser;
use colored::*;

use analysis::{FitCategory, ModelRecommendation, UpgradeRecommendation};
use hardware::HardwareProfile;

#[derive(Parser)]
#[command(name = "ai-hardware-eval")]
#[command(about = "Evaluate hardware for local AI model inference and recommend Ollama models")]
#[command(version)]
struct Cli {
    /// Evaluate a specific model (e.g., "llama3:8b", "deepseek-r1:14b")
    #[arg(long)]
    model: Option<String>,

    /// Ollama API URL
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_url: String,

    /// Output as JSON
    #[arg(long)]
    json: bool,

    /// Disable colored output
    #[arg(long)]
    no_color: bool,
}

fn format_bytes(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 1.0 {
        format!("{:.1} GB", gb)
    } else {
        let mb = bytes as f64 / (1024.0 * 1024.0);
        format!("{:.0} MB", mb)
    }
}

fn print_banner() {
    println!();
    println!(
        "{}",
        "========================================"
            .bold()
            .cyan()
    );
    println!(
        "{}",
        "    AI Hardware Evaluator v0.1.0"
            .bold()
            .cyan()
    );
    println!(
        "{}",
        "========================================"
            .bold()
            .cyan()
    );
    println!();
}

fn print_missing_tools(profile: &HardwareProfile) {
    if profile.missing_tools.is_empty() {
        return;
    }

    println!(
        "{} {}",
        "!".yellow().bold(),
        "Missing optional tools:".yellow()
    );
    println!();

    for tool in &profile.missing_tools {
        println!(
            "  {} {} - {}",
            "*".yellow(),
            tool.command.bold(),
            tool.purpose
        );
        for (distro, cmd) in &tool.install_instructions {
            println!("    {}: {}", distro.dimmed(), cmd);
        }
    }
    println!();
}

fn print_hardware(profile: &HardwareProfile) {
    println!(
        "{} {}",
        "--".cyan(),
        "Hardware Detection".bold().cyan()
    );
    println!();

    // OS
    let os_display = match profile.os.as_str() {
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    println!("  {:<6}{}", "OS:".bold(), os_display);

    // CPU
    println!(
        "  {:<6}{} ({} cores / {} threads)",
        "CPU:".bold(),
        profile.cpu.model_name,
        profile.cpu.cores,
        profile.cpu.threads
    );
    if profile.cpu.is_apple_silicon {
        println!(
            "  {:<6}Architecture: {} | Apple Silicon (Metal GPU)",
            "",
            profile.cpu.architecture
        );
    } else {
        let avx = if profile.cpu.has_avx { "Y" } else { "N" };
        let avx2 = if profile.cpu.has_avx2 { "Y" } else { "N" };
        println!(
            "  {:<6}Architecture: {} | AVX: {} | AVX2: {}",
            "",
            profile.cpu.architecture,
            avx,
            avx2
        );
    }

    // GPU(s)
    if profile.gpus.is_empty() {
        println!("  {:<6}{}", "GPU:".bold(), "None detected".dimmed());
    } else {
        for (i, gpu) in profile.gpus.iter().enumerate() {
            let label = if i == 0 { "GPU:" } else { "" };
            let vram = gpu
                .vram_bytes
                .map(|v| format!(" ({})", format_bytes(v)))
                .unwrap_or_default();
            println!("  {:<6}{}{}", label.bold(), gpu.name, vram);

            let mut details = Vec::new();
            if let Some(ref driver) = gpu.driver_version {
                details.push(format!("Driver: {}", driver));
            }
            if let Some(ref cuda) = gpu.cuda_version {
                details.push(format!("CUDA: {}", cuda));
            }
            if gpu.metal_support {
                details.push("Metal: supported".to_string());
            }
            if !details.is_empty() {
                println!("  {:<6}{}", "", details.join(" | "));
            }
        }
    }

    // Memory
    let unified_note = if profile.memory.is_unified {
        " (unified)"
    } else {
        ""
    };
    println!(
        "  {:<6}{} total{} ({} available)",
        "RAM:".bold(),
        format_bytes(profile.memory.total_ram_bytes),
        unified_note,
        format_bytes(profile.memory.available_ram_bytes)
    );

    if profile.memory.total_swap_bytes > 0 {
        println!(
            "  {:<6}{} total ({} available)",
            "Swap:".bold(),
            format_bytes(profile.memory.total_swap_bytes),
            format_bytes(profile.memory.available_swap_bytes)
        );
    }

    // Disk
    println!(
        "  {:<6}{} total, {} available ({})",
        "Disk:".bold(),
        format_bytes(profile.disk.total_bytes),
        format_bytes(profile.disk.available_bytes),
        profile.disk.storage_type
    );

    println!();
}

fn print_ollama_models(models: &[ollama::OllamaModel]) {
    println!(
        "{} {}",
        "--".cyan(),
        "Installed Ollama Models".bold().cyan()
    );
    println!();

    if models.is_empty() {
        println!("  {}", "No models installed".dimmed());
    } else {
        for model in models {
            println!(
                "  {} {:<30} ({})",
                "*".green(),
                model.name,
                format_bytes(model.size)
            );
        }
    }
    println!();
}

fn print_recommendations(recs: &[ModelRecommendation]) {
    println!(
        "{} {}",
        "--".cyan(),
        "Model Recommendations".bold().cyan()
    );
    println!();

    let categories = [
        (FitCategory::RunsGreat, "[OK]", Color::Green),
        (FitCategory::RunsOk, "[~~]", Color::Yellow),
        (FitCategory::RunsButSlow, "[!!]", Color::Red),
        (FitCategory::WontFit, "[XX]", Color::BrightBlack),
    ];

    for (cat, icon, color) in &categories {
        let matches: Vec<&ModelRecommendation> = recs.iter().filter(|r| r.fit == *cat).collect();
        if matches.is_empty() {
            continue;
        }

        println!("  {} {}", icon.color(*color).bold(), cat.to_string().color(*color).bold());
        println!();

        for rec in &matches {
            println!(
                "    {:<28} {:>5.1}B params   ~{:.1} GB {}",
                rec.model.name.bold(),
                rec.model.parameter_count_b,
                rec.model.vram_required_gb,
                if *cat == FitCategory::RunsButSlow || *cat == FitCategory::WontFit {
                    "RAM"
                } else {
                    "VRAM"
                }
            );
            println!("    {}", rec.reason.dimmed());
            println!();
        }
    }
}

fn print_upgrades(upgrades: &[UpgradeRecommendation]) {
    if upgrades.is_empty() {
        println!(
            "{} {}",
            "--".cyan(),
            "No upgrade recommendations - your hardware is well-equipped!"
                .bold()
                .green()
        );
        println!();
        return;
    }

    println!(
        "{} {}",
        "--".cyan(),
        "Upgrade Suggestions".bold().cyan()
    );
    println!();

    for (i, upgrade) in upgrades.iter().enumerate() {
        let priority_color = match upgrade.priority {
            1 => Color::Red,
            2 => Color::Yellow,
            3 => Color::Cyan,
            _ => Color::White,
        };
        println!(
            "  {}. {} [{}]",
            (i + 1).to_string().bold(),
            upgrade.suggestion,
            upgrade.component.color(priority_color).bold()
        );
        println!("     -> {}", upgrade.unlocks.dimmed());
        println!();
    }
}

fn print_json(
    profile: &HardwareProfile,
    ollama_models: &Option<Vec<ollama::OllamaModel>>,
    recs: &[ModelRecommendation],
    upgrades: &[UpgradeRecommendation],
) {
    let gpu_json: Vec<serde_json::Value> = profile
        .gpus
        .iter()
        .map(|g| {
            serde_json::json!({
                "name": g.name,
                "vendor": format!("{:?}", g.vendor),
                "vram_bytes": g.vram_bytes,
                "driver_version": g.driver_version,
                "cuda_version": g.cuda_version,
                "metal_support": g.metal_support,
            })
        })
        .collect();

    let rec_json: Vec<serde_json::Value> = recs
        .iter()
        .map(|r| {
            serde_json::json!({
                "model": r.model.name,
                "category": r.model.category.to_string(),
                "parameter_count_b": r.model.parameter_count_b,
                "vram_required_gb": r.model.vram_required_gb,
                "fit": r.fit.to_string(),
                "reason": r.reason,
            })
        })
        .collect();

    let upgrade_json: Vec<serde_json::Value> = upgrades
        .iter()
        .map(|u| {
            serde_json::json!({
                "priority": u.priority,
                "component": u.component,
                "suggestion": u.suggestion,
                "unlocks": u.unlocks,
            })
        })
        .collect();

    let ollama_json: Vec<serde_json::Value> = ollama_models
        .as_ref()
        .map(|models| {
            models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.name,
                        "size": m.size,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let output = serde_json::json!({
        "hardware": {
            "os": profile.os,
            "cpu": {
                "model": profile.cpu.model_name,
                "architecture": profile.cpu.architecture,
                "cores": profile.cpu.cores,
                "threads": profile.cpu.threads,
                "avx": profile.cpu.has_avx,
                "avx2": profile.cpu.has_avx2,
                "apple_silicon": profile.cpu.is_apple_silicon,
            },
            "gpus": gpu_json,
            "memory": {
                "total_ram_bytes": profile.memory.total_ram_bytes,
                "available_ram_bytes": profile.memory.available_ram_bytes,
                "total_swap_bytes": profile.memory.total_swap_bytes,
                "unified": profile.memory.is_unified,
            },
            "disk": {
                "total_bytes": profile.disk.total_bytes,
                "available_bytes": profile.disk.available_bytes,
                "storage_type": format!("{}", profile.disk.storage_type),
            },
        },
        "ollama_models": ollama_json,
        "recommendations": rec_json,
        "upgrades": upgrade_json,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

/// Try to resolve a model name into a ModelSpec.
/// 1. Check built-in database
/// 2. Query Ollama /api/show for parameter details
/// 3. Try to parse size from the model name tag (e.g., "deepseek-r1:14b")
fn resolve_model(
    name: &str,
    ollama_client: &ollama::OllamaClient,
    ollama_available: bool,
) -> Result<models::ModelSpec, String> {
    // Check built-in database first
    if let Some(spec) = models::find_model(name) {
        return Ok(spec);
    }

    // Try Ollama API
    if ollama_available {
        if let Ok(show) = ollama_client.show_model(name) {
            // Try to get parameter size from details
            if let Some(ref details) = show.details {
                if let Some(ref param_size) = details.parameter_size {
                    if let Some(params_b) = ollama::parse_parameter_size(param_size) {
                        // Get disk size from local models list if available
                        let disk_size = ollama_client
                            .list_models()
                            .ok()
                            .and_then(|models| {
                                models
                                    .iter()
                                    .find(|m| m.name == name || m.name.starts_with(&format!("{}:", name.split(':').next().unwrap_or(name))))
                                    .map(|m| m.size as f64 / (1024.0 * 1024.0 * 1024.0))
                            })
                            .unwrap_or(params_b * 0.55); // estimate disk from params

                        return Ok(models::estimate_from_params(name, params_b, disk_size));
                    }
                }
            }

            // If we got a response but no parameter size, check model_info
            if let Some(ref info) = show.model_info {
                // Some models report parameter count in model_info
                if let Some(params) = info.get("general.parameter_count") {
                    if let Some(count) = params.as_f64() {
                        let params_b = count / 1_000_000_000.0;
                        return Ok(models::estimate_from_params(name, params_b, params_b * 0.55));
                    }
                }
            }

            return Err(format!(
                "Found '{}' on Ollama but couldn't determine parameter count. \
                 Try specifying the size in the name (e.g., '{}:7b').",
                name, name.split(':').next().unwrap_or(name)
            ));
        }
    }

    // Try to parse size from the name tag (e.g., "some-model:14b" or "model:7b-q4")
    let tag = name.split(':').nth(1).unwrap_or("");
    let size_part = tag.split('-').next().unwrap_or("");
    if let Some(params_b) = ollama::parse_parameter_size(size_part) {
        return Ok(models::estimate_from_params(name, params_b, params_b * 0.55));
    }

    Err(format!(
        "Could not find '{}'. Try:\n  \
         - An exact Ollama model name (e.g., 'llama3:8b', 'deepseek-r1:14b')\n  \
         - Include the size tag so requirements can be estimated (e.g., 'mymodel:13b')\n  \
         - Make sure Ollama is running for registry lookups",
        name
    ))
}

fn print_model_evaluation(rec: &ModelRecommendation, profile: &HardwareProfile) {
    println!(
        "{} {}",
        "--".cyan(),
        format!("Evaluation: {}", rec.model.name).bold().cyan()
    );
    println!();

    // Model details
    println!("  {:<14}{}", "Model:".bold(), rec.model.name);
    println!(
        "  {:<14}{:.1}B parameters",
        "Parameters:".bold(),
        rec.model.parameter_count_b
    );
    println!(
        "  {:<14}{:.1} GB (GPU/unified memory)",
        "VRAM needed:".bold(),
        rec.model.vram_required_gb
    );
    println!(
        "  {:<14}{:.1} GB (CPU-only inference)",
        "RAM needed:".bold(),
        rec.model.ram_required_gb
    );
    println!(
        "  {:<14}{:.1} GB",
        "Disk size:".bold(),
        rec.model.disk_size_gb
    );
    println!(
        "  {:<14}{}",
        "Category:".bold(),
        rec.model.category
    );
    if rec.model.default_quant.contains("estimated") {
        println!(
            "  {:<14}{}",
            "Note:".bold(),
            "Requirements estimated from parameter count".dimmed()
        );
    }
    println!();

    // Verdict
    let (icon, color) = match rec.fit {
        FitCategory::RunsGreat => ("[OK]", Color::Green),
        FitCategory::RunsOk => ("[~~]", Color::Yellow),
        FitCategory::RunsButSlow => ("[!!]", Color::Red),
        FitCategory::WontFit => ("[XX]", Color::BrightBlack),
    };

    println!(
        "  {} {} {}",
        "Verdict:".bold(),
        icon.color(color).bold(),
        rec.fit.to_string().color(color).bold()
    );
    println!("  {}", rec.reason.dimmed());
    println!();

    // Quick hardware context
    let ram_gb = profile.memory.total_ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let best_vram = profile
        .gpus
        .iter()
        .filter_map(|g| g.vram_bytes)
        .max()
        .map(|v| v as f64 / (1024.0 * 1024.0 * 1024.0));

    println!("  {}", "Your hardware:".bold());
    if let Some(vram) = best_vram {
        if profile.memory.is_unified {
            println!(
                "    {:.0} GB unified memory ({:.1} GB usable for inference)",
                ram_gb,
                ram_gb * 0.75
            );
        } else {
            println!("    {:.1} GB VRAM | {:.0} GB RAM", vram, ram_gb);
        }
    } else {
        println!("    No GPU | {:.0} GB RAM (CPU-only)", ram_gb);
    }
    println!();
}

fn main() {
    let cli = Cli::parse();

    if cli.no_color {
        colored::control::set_override(false);
    }

    // Detect hardware
    let profile = hardware::detect_all();
    let ollama_client = ollama::OllamaClient::new(Some(&cli.ollama_url));
    let ollama_available = ollama_client.ping();

    // Single model evaluation mode
    if let Some(ref model_name) = cli.model {
        if !cli.json {
            print_banner();
            print_hardware(&profile);
        }

        match resolve_model(model_name, &ollama_client, ollama_available) {
            Ok(spec) => {
                let rec = analysis::categorize_model(&spec, &profile);
                if cli.json {
                    let output = serde_json::json!({
                        "model": {
                            "name": rec.model.name,
                            "parameter_count_b": rec.model.parameter_count_b,
                            "vram_required_gb": rec.model.vram_required_gb,
                            "ram_required_gb": rec.model.ram_required_gb,
                            "disk_size_gb": rec.model.disk_size_gb,
                            "category": rec.model.category.to_string(),
                        },
                        "fit": rec.fit.to_string(),
                        "reason": rec.reason,
                    });
                    println!("{}", serde_json::to_string_pretty(&output).unwrap());
                } else {
                    print_model_evaluation(&rec, &profile);
                }
            }
            Err(e) => {
                if cli.json {
                    let output = serde_json::json!({ "error": e });
                    println!("{}", serde_json::to_string_pretty(&output).unwrap());
                } else {
                    println!("  {} {}", "Error:".red().bold(), e);
                    println!();
                }
                std::process::exit(1);
            }
        }
        return;
    }

    // Full evaluation mode
    if cli.json {
        let ollama_models = if ollama_available {
            ollama_client.list_models().ok()
        } else {
            None
        };
        let recs = analysis::analyze_all(&profile);
        let upgrades = analysis::suggest_upgrades(&profile);
        print_json(&profile, &ollama_models, &recs, &upgrades);
        return;
    }

    // Rich terminal output
    print_banner();
    print_missing_tools(&profile);
    print_hardware(&profile);

    // Ollama
    if ollama_available {
        match ollama_client.list_models() {
            Ok(models) => print_ollama_models(&models),
            Err(e) => {
                println!(
                    "  {} {}",
                    "!".yellow(),
                    format!("Could not list Ollama models: {}", e).yellow()
                );
                println!();
            }
        }
    } else {
        println!(
            "{} {}",
            "--".cyan(),
            "Ollama".bold().cyan()
        );
        println!();
        println!(
            "  {} {}",
            "!".yellow(),
            "Ollama not detected. Install from https://ollama.com to manage local models."
                .dimmed()
        );
        println!();
    }

    // Analysis
    let recs = analysis::analyze_all(&profile);
    print_recommendations(&recs);

    let upgrades = analysis::suggest_upgrades(&profile);
    print_upgrades(&upgrades);
}
