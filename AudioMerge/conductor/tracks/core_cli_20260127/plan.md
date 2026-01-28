# Implementation Plan: Core CLI & Scanning

## Phase 1: Project Scaffolding [checkpoint: c001fac]
- [x] Task: Initialize Rust project and dependencies 51cdb43
    - [x] Run `cargo init`
    - [x] Add `clap`, `walkdir`, `natord` (for sorting) to `Cargo.toml`
- [x] Task: Conductor - User Manual Verification 'Phase 1: Project Scaffolding' (Protocol in workflow.md)

## Phase 2: CLI & Traversal
- [x] Task: Implement CLI Argument Parsing 892439c
    - [x] Define `Args` struct with `clap`
    - [x] Implement `on-error` enum (halt, prompt, skip)
- [ ] Task: Implement Recursive Scanning
    - [ ] Use `walkdir` to find audio files
    - [ ] Group files by their parent directory
- [ ] Task: Implement Natural Sorting
    - [ ] Sort files within each group using natural ordering
- [ ] Task: Implement Dry-Run Logic
    - [ ] Print table showing Folder -> [File List] -> Output Name
- [ ] Task: Conductor - User Manual Verification 'Phase 2: CLI & Traversal' (Protocol in workflow.md)
