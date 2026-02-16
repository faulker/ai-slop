# Story 1.1: Project Scaffolding & FFI Bridge Validation

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a developer,
I want a working Xcode + Rust project with a validated FFI bridge,
so that all subsequent features can be built on the hybrid Swift/Rust architecture.

## Acceptance Criteria

1. **Build Automation:** Given a fresh clone of the repository, when the Xcode project is built, then the `build-rust.sh` script compiles `spell-i-engine` via Cargo. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.1]
2. **FFI Code Generation:** `swift-bridge` generates Swift/C bridging code in `Generated/` during the build process. [Source: _bmad-output/planning-artifacts/architecture.md#Build Process Structure]
3. **Static Linking:** Xcode links `libspell_i_engine.a` successfully and the app binary launches without crash. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.1]
4. **Functional FFI Bridge:** When a test harness calls `SpellEngine.new()` and `lintText("This is a tset")`, then the engine returns at least one `LintResult` with a suggestion for "tset". [Source: _bmad-output/planning-artifacts/epics.md#Story 1.1]
5. **Stability:** The FFI round-trip (Swift -> Rust -> Swift) completes without crash or memory error. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.1]
6. **Engine Validation:** When `cargo test` is run in `spell-i-engine/`, then all unit tests pass verifying `lint_text` returns correct results for known misspellings. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.1]

## Tasks / Subtasks

- [x] **Task 1: Rust Crate Initialization (AC: 1, 6)**
  - [x] Initialize `spell-i-engine` crate: `cargo init --lib spell-i-engine`
  - [x] Configure `Cargo.toml` with `crate-type = ["staticlib"]` and dependencies: `harper-core`, `swift-bridge`.
  - [x] Create `build.rs` to invoke `swift-bridge-build` for FFI generation.
- [x] **Task 2: Xcode Project Scaffolding (AC: 1, 3)**
  - [x] Create macOS App project "Spell-i" (AppKit, no storyboard).
  - [x] Configure `Info.plist`: `LSUIElement = YES` (Menu bar only).
  - [x] Disable App Sandbox in Entitlements (Required for `CGEventTap` and `AX` API).
  - [x] Create `BridgingHeader.h` and configure `SWIFT_OBJC_BRIDGING_HEADER` in build settings.
  - [x] Set `LIBRARY_SEARCH_PATHS` to include the Cargo output directory.
- [x] **Task 3: Build System Integration (AC: 1, 2)**
  - [x] Create `build-rust.sh` to handle `cargo build` and library moving/renaming if needed.
  - [x] Add "Run Script" build phase in Xcode to invoke `build-rust.sh` before "Compile Sources".
- [x] **Task 4: FFI Bridge Implementation (AC: 4, 5)**
  - [x] Implement `SpellEngine` struct in `lib.rs` with `new()` and `lint_text(&str)` functions.
  - [x] Define `LintResult` struct in Rust and export via `swift-bridge`.
  - [x] Implement `lint_text` to wrap `harper-core` linting logic.
- [x] **Task 5: Swift Validation Harness (AC: 4, 5)**
  - [x] Create `SpellEngine` wrapper in Swift (if not auto-generated sufficiently).
  - [x] In `AppDelegate` or `main.swift`, call the bridge on launch and log the results for "tset".
  - [x] Verify `LintResult` data is correctly received in Swift.
- [x] **Task 6: Rust Unit Testing (AC: 6)**
  - [x] Add tests in `src/lib.rs` to verify `harper-core` integration independently of Swift.

## Dev Notes

- **Architecture:** Coordinator pattern, Never-fail FFI (always return empty Vec on error), Single serial background queue for FFI. [Source: _bmad-output/planning-artifacts/architecture.md#Decision Priority Analysis]
- **FFI Boundary:** Only 4 functions planned (`new`, `lint_text`, `add_user_word`, `remove_user_word`). Focus on the first two for this story.
- **Privacy:** 100% offline. Do not add any networking libraries. [Source: _bmad-output/planning-artifacts/prd.md#Privacy]
- **Threading:** Ensure FFI calls are dispatched to a serial background queue (`com.spell-i.engine`).

### Project Structure Notes

- **Swift root:** `Spell-i/`
- **Rust root:** `spell-i-engine/`
- **Generated headers:** `Generated/` (should be added to Search Paths)
- **Static Library:** `libspell_i_engine.a`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/prd.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash (interactive CLI agent)

### Debug Log References

- `cargo test` output showing 11 tests passed (after fixing `test_add_user_word_suppresses_lint`).
- `build-rust.sh` output showing successful static library build.
- `xcodebuild` output showing successful compilation and linking.
- `--validate-ffi` runtime output showing successful FFI round-trip for "This is a tset".

### Completion Notes List

- Rust `spell-i-engine` crate initialized and configured as `staticlib`.
- `harper-core` integrated for spelling and grammar linting.
- `swift-bridge` FFI bridge implemented with `SpellEngine` and `LintResults` opaque types.
- Xcode project configured as a menu bar app (`LSUIElement=YES`) with App Sandbox disabled.
- `build-rust.sh` integrated as a "Run Script" build phase to automate Rust compilation.
- `FFIValidator` implemented in Swift to verify the bridge at runtime.
- Verified that "tset" is correctly flagged with "test" as a suggestion.
- **Code Review Fixes:**
  - Fixed UTF-8/UTF-16 encoding mismatch in Swift-Rust string indexing.
  - Hardened FFI boundary with `catch_unwind` to prevent panics from crashing the app.
  - Improved `build-rust.sh` to support cross-architecture (Intel/Apple Silicon) builds and respect Xcode configurations.
  - Strengthened unit test assertions for Unicode and correct text.

### File List

- `spell-i-engine/Cargo.toml`
- `spell-i-engine/src/lib.rs`
- `spell-i-engine/build.rs`
- `build-rust.sh`
- `Spell-i.xcodeproj/project.pbxproj`
- `Spell-i/App/main.swift`
- `Spell-i/App/FFIValidator.swift`
- `Spell-i/BridgingHeader.h`

