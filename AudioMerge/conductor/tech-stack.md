# Technology Stack

## Core Language & Runtime
- **RustLang**: Chosen for performance, memory safety, and its excellent ecosystem for building robust CLI utilities.

## Primary Libraries (Crates)
- **Audio Processing**: `symphonia`. A pure Rust library for media decoding and handling. This allows for a portable, self-contained binary without external system dependencies like FFmpeg for standard MP3 processing.
- **CLI Parsing**: `clap` (v4). Used for defining a feature-rich CLI with subcommands, arguments (like `--on-error` and `--dry-run`), and generated help messages.
- **Logging**: `tracing` and `tracing-subscriber` (or `env_logger`). Provides structured, level-based logging to fulfill the "Informative & Descriptive" communication guideline.

## Infrastructure & Tools
- **Cargo**: Rust's build system and package manager.
- **GitHub Actions (Optional)**: For automated building and testing of the Rust binary across platforms.
