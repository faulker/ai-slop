# ai-hardware-eval

A cross-platform CLI tool that evaluates your system hardware for running AI models locally with [Ollama](https://ollama.com). It detects your hardware, identifies bottlenecks, recommends models that will run well, and suggests the most impactful upgrades.

## Features

- **Hardware detection** — CPU, GPU (NVIDIA/AMD/Apple Silicon), RAM, disk type and space
- **Cross-platform** — Linux and macOS, with Apple Silicon unified memory support
- **Single model evaluation** — Check any Ollama model with `--model`, even ones not in the built-in database
- **Model recommendations** — Categorizes 12 popular Ollama models by how well they'll run on your hardware
- **Upgrade suggestions** — Prioritized list of what hardware changes would unlock better models
- **Ollama integration** — Lists your installed models if Ollama is running; queries API for model details
- **Graceful degradation** — Works without a GPU, without Ollama, and with missing system tools

## Install

Requires [Rust](https://rustup.rs/) toolchain.

```bash
git clone <repo-url>
cd ai-hardware-eval
cargo build --release
# Binary at target/release/ai-hardware-eval
```

## Usage

```bash
# Run with rich terminal output (full hardware scan + all model recommendations)
ai-hardware-eval

# Evaluate a specific model
ai-hardware-eval --model llama3:8b

# Evaluate any Ollama model (queries Ollama API for details, or estimates from name)
ai-hardware-eval --model deepseek-r1:14b

# JSON output (for scripting)
ai-hardware-eval --json

# Combine flags
ai-hardware-eval --model mistral:7b --json

# Custom Ollama endpoint
ai-hardware-eval --ollama-url http://192.168.1.100:11434

# Disable colors (for piping)
ai-hardware-eval --no-color
```

## Example Output

```
========================================
    AI Hardware Evaluator v0.1.0
========================================

-- Hardware Detection

  OS:   macOS
  CPU:  Apple M4 Pro (12 cores / 12 threads)
        Architecture: arm64 | Apple Silicon (Metal GPU)
  GPU:  Apple M4 Pro GPU (18.0 GB)
        Metal: supported
  RAM:  24.0 GB total (unified) (8.0 GB available)
  Disk: 93.5 GB available (SSD)

-- Model Recommendations

  [OK] Runs Great

    llama3:8b                      8.0B params   ~5.0 GB VRAM
    Fits in unified memory with Metal acceleration (13.0 GB headroom)

    mistral:7b                     7.2B params   ~4.5 GB VRAM
    Fits in unified memory with Metal acceleration (13.5 GB headroom)

  [XX] Won't Fit

    llama3:70b                    70.6B params   ~40.0 GB RAM
    Needs 40.0 GB, only 18.0 GB usable unified memory

-- Upgrade Suggestions

  1. Consider a Mac with 48-64GB unified memory [Unified Memory]
     -> Enables 30B+ parameter models
```

## Single Model Evaluation

Use `--model` to check how well a specific model will run on your hardware:

```
$ ai-hardware-eval --model deepseek-r1:14b

-- Evaluation: deepseek-r1:14b

  Model:        deepseek-r1:14b
  Parameters:   14.0B parameters
  VRAM needed:  9.1 GB (GPU/unified memory)
  RAM needed:   15.4 GB (CPU-only inference)
  Disk size:    7.7 GB
  Category:     General
  Note:         Requirements estimated from parameter count

  Verdict: [OK] Runs Great
  Fits in unified memory with Metal acceleration (8.9 GB headroom)
```

The model is resolved in this order:
1. **Built-in database** — exact match for the 12 curated models
2. **Ollama API** — queries `/api/show` for parameter details (requires Ollama running)
3. **Name parsing** — extracts size from the tag (e.g., `model:14b` → 14B parameters)

## Missing Tools

On Linux, if optional system commands are missing, the tool will tell you what to install:

```
! Missing optional tools:

  * lspci - GPU detection
    Ubuntu/Debian/Mint: sudo apt install pciutils
    RHEL/Fedora/CentOS: sudo dnf install pciutils
    Arch: sudo pacman -S pciutils
```

## Supported Models

The built-in database includes popular Ollama models at Q4 quantization:

| Model | Params | VRAM Needed | Category |
|-------|--------|-------------|----------|
| tinyllama:1.1b | 1.1B | 0.8 GB | Small |
| gemma2:2b | 2.6B | 1.8 GB | Small |
| llama3.2:3b | 3.2B | 2.1 GB | General |
| phi3:mini | 3.8B | 2.5 GB | Small |
| codellama:7b | 6.7B | 4.2 GB | Code |
| mistral:7b | 7.2B | 4.5 GB | General |
| qwen2.5:7b | 7.6B | 4.8 GB | General |
| qwen2.5-coder:7b | 7.6B | 4.8 GB | Code |
| llama3:8b | 8.0B | 5.0 GB | General |
| gemma2:9b | 9.2B | 5.7 GB | General |
| deepseek-coder-v2:16b | 15.7B | 9.5 GB | Code |
| llama3:70b | 70.6B | 40.0 GB | General |

## Testing

```bash
cargo test
```

29 unit tests cover hardware output parsing, model database integrity, model lookup/estimation, analysis logic, and Apple Silicon scenarios.

## Cross-Platform Build

A build script is included that produces release binaries for macOS and Linux:

```bash
./build.sh
```

Requires `musl-cross` for Linux cross-compilation from macOS:

```bash
brew install filosottile/musl-cross/musl-cross
```

Outputs to `dist/`:
- `ai-hardware-eval-macos-arm64` — Apple Silicon
- `ai-hardware-eval-macos-x86_64` — Intel Mac
- `ai-hardware-eval-linux-x86_64` — Static musl binary (runs on any x86_64 Linux)
