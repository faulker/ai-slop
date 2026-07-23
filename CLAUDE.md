# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Repo Is

ai-slop is a catch-all monorepo of small, independent, AI-generated tools and experiments. It is not one application. Each top-level folder is its own self-contained subproject with its own language, toolchain, and build. When a tool graduates to being genuinely used, it tends to move out into its own repo.

## Working In This Repo

Treat each subproject folder as a separate project:
- `cd` into the specific subproject before building, running, or testing. There is no root-level build.
- Read that subproject's own `README.md` first, and its `CLAUDE.md` if present, for accurate commands and conventions. The per-subproject file is authoritative over anything here.
- Do not assume shared dependencies or conventions across subprojects; they differ (Rust vs TypeScript vs Swift, different build scripts).

## Subprojects

- **aetr/** (Rust core + Swift macOS app + Kotlin Android app): encrypted text/voice sent as COFDM audio bursts through analog FM radios; shared `aetr-core` does all crypto/FEC/modem work via UniFFI. Has its own `docs/`; `cargo test -p aetr-core --release`, `macos/build.sh`, `android/gradlew assembleDebug`.
- **AudioMerge/** (Rust CLI): recursively merges MP3 files from subdirectories into consolidated files (e.g. audiobooks). `cargo build`, `cargo run`.
- **BookmarkCleaner/** (Rust TUI): scans exported browser bookmarks for dead links, lets you review/remove them, upgrades HTTP to HTTPS. `cargo run`.
- **claude-usage/** (Rust CLI): pulls Claude Code usage from the Anthropic API using the OAuth token in the macOS Keychain. `cargo run`.
- **obd2-writer/** (Rust CLI + TUI): reads/writes a 2023 Toyota Tacoma's ECUs over Bluetooth via an OBDLink MX+ scanner, with live dashboards, DID scanning, and backup/restore. Has its own `CLAUDE.md`.
- **orcha-ai/** (TypeScript/Node CLI): orchestrates multiple Claude Code agents from a markdown spec, building a task DAG and running tasks in parallel. Uses `vitest`; `package.json` scripts.
- **spell-i/** (Swift macOS menu bar app + Rust engine): system-wide spell/grammar checking across apps via the Accessibility API, backed by the Harper engine over Rust FFI. Has its own `CLAUDE.md`; build via `build.sh` / `build-rust.sh`.
- **stash-mgr/** (Rust TUI): manages git stashes with live diff previews, selective file stashing, and vim keybindings. `cargo run`.
- **ThoughtQueue/** (Swift macOS menu bar app, code in `txtmem/`): captures text from any app and sends it to Claude Desktop as a new conversation. Has its own `CLAUDE.md`; build via `build.sh`.
- **ai-hardware-eval/** (Rust): hardware evaluation experiment. Has its own `CLAUDE.md`.

Language conventions follow the global rules: Rust subprojects use `cargo test`, TypeScript subprojects use ES modules and `vitest`, Swift menu bar apps use XcodeGen (`project.yml`) plus a `build.sh`.

## Model Selection

Pick per the subproject's task, not the repo:
- **Claude Fable 5 (`claude-fable-5`):** hardest work in any subproject: FFI boundaries (spell-i), Bluetooth/ECU protocol and safety (obd2-writer), agent-orchestration DAG logic (orcha-ai), and Keychain/credential handling (claude-usage).
- **Claude Opus 4.8 (`claude-opus-4-8`):** default for building out a subproject feature or multi-file change.
- **Claude Sonnet 5 (`claude-sonnet-5`):** routine coding, small CLI/TUI tweaks, tests within a subproject.
- **Claude Haiku 4.5 (`claude-haiku-4-5`):** quick lookups, README/doc edits, boilerplate, cheap subagent work.
