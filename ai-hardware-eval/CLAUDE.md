# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # Build the project
cargo test                     # Run all tests (20 unit tests across 4 modules)
cargo test analysis::tests     # Run tests for a single module
cargo test test_apple_silicon  # Run tests matching a name pattern
cargo run                      # Run with rich terminal output
cargo run -- --json            # Run with JSON output
cargo run -- --ollama-url http://host:11434  # Custom Ollama endpoint
```

## Architecture

Cross-platform (Linux + macOS) Rust CLI that evaluates system hardware for local LLM inference, recommends Ollama models, and suggests upgrades.

**Data flow:** `main.rs` orchestrates: detect hardware → query Ollama → analyze fit → format output.

### Modules

- **`hardware.rs`** — Detects CPU, GPU, RAM, disk by parsing OS-specific command output (`lscpu`/`sysctl`, `nvidia-smi`/`system_profiler`, `free`/`vm_stat`, etc.). Linux and macOS paths are separate functions selected at runtime via `std::env::consts::OS`. Apple Silicon is treated specially: unified memory, Metal GPU, no AVX. Missing commands produce warnings, not panics.

- **`models.rs`** — Static array of `ModelSpec` entries for ~12 popular Ollama models. **Must stay sorted by `vram_required_gb`** (enforced by test). Each entry has VRAM/RAM/disk requirements at Q4 quantization.

- **`analysis.rs`** — Categorizes each model as RunsGreat/RunsOk/RunsButSlow/WontFit based on hardware. Apple Silicon uses 75% of unified memory as usable VRAM. Upgrade suggestions are rule-based with priority ordering.

- **`ollama.rs`** — Thin HTTP client using `ureq` (blocking, 5s timeout) to hit `GET /api/tags`. Gracefully handles Ollama not running.

### Key Design Decisions

- `ureq` (blocking) instead of `reqwest` (async) — no async runtime needed for 1-2 HTTP calls
- All hardware detection is fault-tolerant: missing commands → defaults, never panics
- Apple Silicon unified memory treated as ~75% available for GPU inference
- Model database is a sorted const array, not fetched from Ollama's registry
