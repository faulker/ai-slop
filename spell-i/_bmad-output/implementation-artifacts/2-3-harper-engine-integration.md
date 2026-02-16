# Story 2.3: Harper Engine Integration

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a user,
I want my text checked for spelling and grammar errors in real-time,
so that I can see what I've misspelled or written incorrectly.

## Acceptance Criteria

1. **FFI Dispatch:** Given `AccessibilityReader` has returned text content, when the `TextMonitorCoordinator` dispatches to the spell queue, then `SpellEngine.lintText()` is called on the serial background queue. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.3]
2. **Error Detection:** Harper returns results for spelling errors (e.g., "tset" -> "test") and grammar errors with appropriate messages and positions. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.3]
3. **Clean Results:** If the text has no errors, an empty array is returned and any existing underlines are cleared. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.3]
4. **Never-Fail FFI:** If Harper encounters an internal error, `lintText()` returns an empty `Vec` and logs the error in Rust (eprintln!). [Source: _bmad-output/planning-artifacts/epics.md#Story 2.3]
5. **Main Thread Callback:** Lint results are dispatched back to the main thread for overlay rendering. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.3]

## Tasks / Subtasks

- [x] **Task 1: Rust Engine Implementation (AC: 2, 4)**
  - [x] Implement `SpellEngine` struct in `spell-i-engine/src/lib.rs`.
  - [x] Initialize `harper-core` with curated dictionary and American English dialect.
  - [x] Implement `lint_text` function to process incoming strings and return `LintResults`.
- [x] **Task 2: FFI Bridge Export (AC: 1, 4)**
  - [x] Define FFI boundary in `lib.rs` using `#[swift_bridge::bridge]`.
  - [x] Export `SpellEngine` and `LintResults` (opaque types).
  - [x] Implement accessor methods for `LintResults` fields (error type, message, offsets, suggestions).
- [x] **Task 3: Coordinator Wiring (AC: 1, 5)**
  - [x] Instantiate `SpellEngine` on `com.spell-i.engine` queue.
  - [x] Implement async linting flow in `TextMonitorCoordinator`.
  - [x] Map `LintResults` to `LintDisplayItem` internal representation.
- [x] **Task 4: Unit Testing (AC: 2, 3)**
  - [x] Add Rust tests to verify `lint_text` with various inputs (empty, correct, misspelled, grammar errors).

## Dev Notes

- **Threading:** Single serial background queue is mandatory for all FFI interaction to prevent concurrency issues. [Source: _bmad-output/planning-artifacts/architecture.md#Decision Priority Analysis]
- **Privacy:** Never log user text content across the FFI boundary. Only log metadata (counts, error types). [Source: _bmad-output/planning-artifacts/architecture.md#Logging Patterns]
- **Optimization:** Filter suggestions to top 5 as per UX spec. [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Correction popup]

### Project Structure Notes

- `spell-i-engine/src/lib.rs`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/prd.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash

### Debug Log References

- `cargo test` in `spell-i-engine` verified Harper engine detection of spelling and grammar.
- `TextMonitorCoordinator` logs confirm FFI calls are offloaded from the main thread.
- `--validate-ffi` runtime check confirmed successful Swift -> Rust -> Swift round-trip.

### Completion Notes List

- Harper engine integrated via Rust static library.
- FFI bridge implemented using `swift-bridge` with opaque types for performance.
- Coordinator updated to perform lints on a serial background queue.
- Support for spelling and grammar lints with suggestion extraction (limited to top 5).
- **Code Review Fixes:**
  - Fixed UTF-8 byte offset to Swift String character index conversion to support multi-byte characters (emojis, etc.).
  - Optimized the result processing loop by eliminating redundant string conversions and memory allocations.
  - Added required error logging to the Rust engine's panic boundary.

### Code Review Fixes (2026-02-15)

- **[H5-HIGH]** Added bounds checking to all `LintResults` accessor methods in `lib.rs` — replaced unchecked array indexing with `.get()` + `unwrap_or_default()` to prevent panics on invalid indices (never-fail FFI compliance).

### File List

- `spell-i-engine/src/lib.rs`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`
- `Spell-i/Utilities/Constants.swift`

