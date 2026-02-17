---
title: 'Fix Silent Pipeline Failures & Overlay Window Tracking'
slug: 'fix-pipeline-failures-overlay-tracking'
created: '2026-02-16'
status: 'completed'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [Swift, Rust, AppKit, Accessibility API, swift-bridge FFI, harper-core]
files_to_modify: ['spell-i-engine/src/lib.rs', 'Spell-i/TextMonitoring/TextMonitorCoordinator.swift', 'Spell-i/App/AppDelegate.swift', 'Spell-i/TextMonitoring/FocusTracker.swift', 'Spell-i/App/StatusBarController.swift', 'Spell-i/Utilities/Constants.swift', 'Spell-i/Overlay/OverlayWindowController.swift']
code_patterns: ['AXObserver for system notifications', 'engineQueue background dispatch with main-thread handoff', 'lintGeneration counter for stale result rejection', 'TypingDebouncer pattern for delayed actions', 'Unmanaged.passUnretained(self) for C callback refcon', 'catch_unwind at FFI boundary (used in lint_text, missing in new)']
test_patterns: ['Rust: #[cfg(test)] module in lib.rs (9 tests) and user_dict.rs (4 tests), run with cargo test', 'Swift: XCTest in Spell-iTests/ (1 test), run with xcodebuild test', 'Rust tests cover engine init, linting, user dictionary, edge cases']
---

# Tech-Spec: Fix Silent Pipeline Failures & Overlay Window Tracking

**Created:** 2026-02-16

## Overview

### Problem Statement

Two bugs in Spell-i's monitoring pipeline:

1. **Pipeline startup failure**: `coordinator.start()` is called inside the `initializeEngine` completion handler (AppDelegate.swift:111-115). If engine initialization fails for any reason — Rust panic crossing FFI (which is UB and typically causes SIGABRT), a hang in `LintGroup::new_curated()`, or any other failure in the `engineQueue.async` block — the completion handler never fires, so `start()` is never called. The event tap, focus tracker, and debouncer never start. The app appears running (menu bar icon visible) but does zero spell checking. There is no user-visible feedback for any failure. **The root cause of the specific failure the user is experiencing is uncertain** — it could be a panic, a hang, or a silent error. The fix must handle all cases by decoupling startup from engine init.

2. **Overlay drift on window move/scroll**: Squiggly underlines are positioned once per lint pass using AX bounds queries. When the user drags, resizes, or scrolls within the target application window, the overlay stays fixed at its original screen position. There is no mechanism to detect window movement or scroll and reposition or clear the overlay. Scrolling is arguably more common than window dragging.

### Solution

1. **Resilient pipeline startup**: Decouple engine initialization from monitoring startup. Call `start()` directly — it doesn't depend on the engine. Initialize the engine separately on `engineQueue` with `catch_unwind` wrapping. On failure, mark engine as failed and schedule a retry. Add user-visible status feedback via menu bar icon state. The `performLint()` method already checks for a nil engine (it's Optional) — when nil, it simply returns without crashing.

2. **Window/scroll movement tracking**: Add an AX observer for `kAXMovedNotification` and `kAXResizedNotification` on the focused app's frontmost window. Add a global scroll-wheel event monitor (same pattern as the existing global click monitor in OverlayWindowController). On any movement/scroll, clear the overlay immediately and trigger a re-lint after settling, using a proper `TypingDebouncer`-style debouncer (not `asyncAfter`).

### Scope

**In Scope:**
- Decouple `coordinator.start()` from engine init completion
- Add `catch_unwind` around `SpellEngine::new()` as defense-in-depth
- Add engine retry logic with exponential backoff (3 attempts)
- Add menu bar icon state feedback (normal / degraded / error)
- Add diagnostic logging at each pipeline stage
- Add AX observer for window moved/resized notifications in FocusTracker
- Add global scroll-wheel monitor for overlay clearing
- Clear overlay + re-lint on window movement/scroll with proper debouncing
- Tests for the new behavior

**Out of Scope:**
- Deep Electron/Chromium app compatibility improvements
- New UI windows for error reporting (beyond menu bar icon state)
- Multi-monitor overlay handling changes
- Changes to Harper itself

### Known Limitations

- **Electron/Chromium apps**: `kAXMovedNotification` and `kAXResizedNotification` may not fire for Electron/Chromium app windows. Window move tracking will not work for these apps. This is an AX API limitation.
- **Scroll in non-standard views**: The global scroll-wheel monitor clears the overlay on any scroll event system-wide. This is imprecise but safe — worst case is a brief flicker of underlines on scroll in unrelated windows.

## Context for Development

### Codebase Patterns

- Pipeline orchestration lives in `TextMonitorCoordinator` — it owns all sub-components
- AX calls must happen on main thread; Harper linting on background `engineQueue`
- `lintGeneration` counter prevents stale results from overwriting fresh ones
- FocusTracker already uses `AXObserver` for `kAXFocusedUIElementChangedNotification` via `AXObserverCreate` + `AXObserverAddNotification` + `CFRunLoopAddSource` — window observers follow the exact same pattern
- `Unmanaged.passUnretained(self).toOpaque()` used as `refcon` in AX observer callbacks
- `StatusBarController` manages the menu bar icon via `NSStatusItem` with `NSImage(systemSymbolName:)`
- All logging goes through the `Logger(category:)` utility class
- Rust FFI uses opaque pointers via `swift-bridge`; `SpellEngine` is constructed on Swift side as `SpellEngine()` which calls `__swift_bridge__$SpellEngine$new`
- `lint_text()` already wraps `Document::new()` + `linter.lint()` in `std::panic::catch_unwind(AssertUnwindSafe(...))` — the constructor needs the same treatment
- **Thread safety**: `self.engine` is written on `engineQueue` and read on `engineQueue`. The nil-check in `performLint()` is inside `engineQueue.async` (line 213), which is safe. Any new engine availability checks MUST also be inside `engineQueue.async`, never on the main thread.

### Files to Reference

| File | Purpose |
| ---- | ------- |
| `spell-i-engine/src/lib.rs` | Rust engine; `new()` lacks catch_unwind, `lint_text()` has it |
| `Spell-i/App/AppDelegate.swift` | Wires pipeline; chains `start()` inside `initializeEngine` completion |
| `Spell-i/TextMonitoring/TextMonitorCoordinator.swift` | Central orchestrator; `initializeEngine`, `start()`, `performLint()` |
| `Spell-i/TextMonitoring/FocusTracker.swift` | AX observer for focus; needs window move observer |
| `Spell-i/TextMonitoring/EventTapManager.swift` | CGEventTap; independent of engine |
| `Spell-i/TextMonitoring/TypingDebouncer.swift` | Debounce pattern to reuse for window move |
| `Spell-i/App/StatusBarController.swift` | Menu bar icon; needs state feedback |
| `Spell-i/Overlay/OverlayWindowController.swift` | Overlay; `updateUnderlines`, `clearUnderlines`, global click monitor pattern |
| `Spell-i/Utilities/Constants.swift` | Shared constants |

### Technical Decisions

- Engine init is decoupled from `start()` — monitoring begins immediately, `performLint()` safely no-ops when engine is nil (check stays inside `engineQueue.async`)
- On `catch_unwind` panic in Rust `new()`: return a "dead" `SpellEngine` with `degraded: true` flag. Its `lint_text()` checks `self.degraded` and returns empty results immediately. This avoids trying to reconstruct `LintGroup::new_curated()` which would panic again.
- Swift side checks `engine.is_degraded()` after init and schedules retry if true
- Retry logic: up to 3 attempts with 2s/5s/10s delays. After all retries fail, set state to `.failed`
- Window movement detection uses AX observer (consistent with existing FocusTracker pattern)
- Scroll detection uses `NSEvent.addGlobalMonitorForEvents(matching: .scrollWheel)` (consistent with existing click monitor pattern in OverlayWindowController)
- Window move/scroll handling uses a dedicated `TypingDebouncer` instance (NOT `asyncAfter`) for proper debouncing of rapid events
- Menu bar icon uses `textformat.abc` (active) and `exclamationmark.triangle` (degraded/error) — both available since macOS 11
- After engine init completes, must regenerate FFI bridge via `./build-rust.sh` (auto-runs as Xcode pre-build script)

## Implementation Plan

### Tasks

Tasks are ordered by dependency (lowest level first).

- [x] **Task 1: Add `catch_unwind` to Rust `SpellEngine::new()` and add `degraded` flag**
  - File: `spell-i-engine/src/lib.rs`
  - Action:
    1. Add `degraded: bool` field to the `SpellEngine` struct
    2. Wrap the body of `fn new() -> Self` in `std::panic::catch_unwind(AssertUnwindSafe(|| { ... }))`. On success, return the engine with `degraded: false`. On panic, log to stderr with `[spell-i-engine] SpellEngine::new() panicked: {:?}`, then return a "dead" `SpellEngine` with `degraded: true` and dummy/default fields (empty `Vec` for dictionary, default `PlainEnglish` parser — these don't need `LintGroup::new_curated()`)
    3. In `lint_text()`, add early return: `if self.degraded { return LintResults { items: Vec::new() }; }` before the existing lint logic
    4. Expose `fn is_degraded(&self) -> bool` in the `#[swift_bridge::bridge]` module. This is a `&self` method returning `bool` — `swift-bridge` handles this for opaque types.
  - Notes: The "dead" engine cannot lint but is a valid Rust object. It won't try to call `LintGroup::new_curated()` again. The retry mechanism on the Swift side creates a fresh `SpellEngine()` which will attempt full construction again. After changing the Rust code, `./build-rust.sh` (or Xcode pre-build) regenerates `Generated/` FFI files automatically.

- [x] **Task 2: Add Rust tests for panic-safe constructor**
  - File: `spell-i-engine/src/lib.rs`
  - Action: Add test `test_engine_new_does_not_panic` that verifies `SpellEngine::new()` returns a valid instance with `is_degraded() == false`. Add test `test_degraded_engine_returns_empty_results` that constructs a degraded engine directly (set `degraded: true` in test) and verifies `lint_text("teh")` returns count == 0.
  - Notes: Run with `cargo test`. We can't easily trigger a real panic in `new()` in tests, but we can verify the degraded path works.

- [x] **Task 3: Decouple `start()` from engine init in `TextMonitorCoordinator`**
  - File: `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`
  - Action:
    1. Add `EngineState` enum: `.initializing`, `.ready`, `.degraded(retryCount: Int)`, `.failed`
    2. Add `private(set) var engineState: EngineState = .initializing`
    3. Add `var onEngineStateChanged: ((EngineState) -> Void)?` callback
    4. Change `start()` to be callable independently — it already installs event tap, debouncer, focus tracker. Remove any dependency on engine being ready.
    5. Change `initializeEngine()` to drop the `completion:` parameter. New flow: runs on `engineQueue`, creates `SpellEngine()`, checks `is_degraded()`. If not degraded: sets `self.engine`, dispatches to main to set `engineState = .ready` and fire callback, then calls `performLint()`. If degraded: dispatches to main to set `engineState = .degraded(retryCount: 0)`, fire callback, and schedule retry.
    6. In `performLint()`: the existing `guard let engine = self.engine` check at the top of the `engineQueue.async` block is sufficient — when engine is nil, log "engine not yet available" and return. Do NOT add a check on the main thread (data race).
    7. Add `private func retryEngineInit(attempt: Int)` — schedules `initializeEngine()` again after delay (2s, 5s, 10s). Max 3 retries. On final failure, set `engineState = .failed`.
  - Notes: The `windowMoveDebouncer` (Task 7) is also added here as a `TypingDebouncer` instance.

- [x] **Task 4: Restructure `AppDelegate.setupApp()` initialization**
  - File: `Spell-i/App/AppDelegate.swift`
  - Action: Change `setupApp()` from:
    ```swift
    coordinator.initializeEngine { [weak self] in
        self?.coordinator?.start()
        self?.overlayController?.showOverlay()
    }
    ```
    To:
    ```swift
    coordinator.onEngineStateChanged = { [weak statusBar] state in
        statusBar?.updateState(state)
    }
    coordinator.start()
    overlayController.showOverlay()
    coordinator.initializeEngine()  // fire-and-forget
    ```
  - Notes: Monitoring starts immediately. Overlay shows (empty). Engine loads in background. State changes flow to status bar.

- [x] **Task 5: Add engine state feedback to `StatusBarController`**
  - File: `Spell-i/App/StatusBarController.swift`
  - Action:
    1. Add `func updateState(_ state: TextMonitorCoordinator.EngineState)` method
    2. Change icon based on state: `.ready` → `textformat.abc`, `.degraded` → `exclamationmark.triangle`, `.failed` → `xmark.circle`, `.initializing` → `textformat.abc` (same as ready, no flicker)
    3. Add a non-actionable `NSMenuItem` at top of menu showing status text: "Engine: Ready" / "Engine: Starting..." / "Engine: Degraded — retrying..." / "Engine: Failed — restart app"
    4. Use only SF Symbols available since macOS 11 (`textformat.abc`, `exclamationmark.triangle`, `xmark.circle`)
  - Notes: Status menu item should be grayed out (`.isEnabled = false`). Update it in `updateState()`.

- [x] **Task 6: Add window move/resize observer to `FocusTracker`**
  - File: `Spell-i/TextMonitoring/FocusTracker.swift`
  - Action:
    1. Add `func focusTrackerDidDetectWindowMove()` to `FocusTrackerDelegate` protocol
    2. In `setupAXObserver()`, after the existing focus-element observer on the app element: query the app for `kAXFocusedWindowAttribute` to get the frontmost window AXUIElement
    3. Add `kAXMovedNotification` and `kAXResizedNotification` observers on that window element using the same `AXObserver` instance
    4. In the observer callback: check the notification name — if it's `kAXMovedNotification` or `kAXResizedNotification`, call `delegate?.focusTrackerDidDetectWindowMove()`. Otherwise call `delegate?.focusTrackerDidChangeElement()` (existing behavior).
    5. Store the observed window element in a property so `teardownAXObserver()` can remove its notifications too
  - Notes: The AX observer callback runs on the main run loop (because `CFRunLoopAddSource` is called with `.commonModes` on the current/main run loop). This is the same threading guarantee as the existing focus observer. The teardown-on-app-switch pattern handles stale window elements.

- [x] **Task 7: Wire window move + scroll handling in `TextMonitorCoordinator`**
  - File: `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`
  - Action:
    1. Add `private let windowMoveDebouncer = TypingDebouncer(interval: Constants.windowMoveDebounceInterval)` — reuses the existing `TypingDebouncer` class for proper reset-on-each-event debouncing
    2. In `start()`: set `windowMoveDebouncer.onDebounced = { [weak self] in self?.performLint() }`
    3. Implement `focusTrackerDidDetectWindowMove()`: call `clearResults()` immediately, then `windowMoveDebouncer.keystroke()` to schedule re-lint after debounce
    4. In `stop()`: add `windowMoveDebouncer.cancel()`
  - Notes: This uses proper debouncing — each new move event resets the timer. During a drag, the overlay stays clear. Re-lint fires only after the drag settles. `TypingDebouncer.keystroke()` handles the reset-or-create-timer logic.

- [x] **Task 8: Add global scroll-wheel monitor**
  - File: `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`
  - Action:
    1. Add `private var scrollMonitor: Any?` property
    2. In `start()`: install `NSEvent.addGlobalMonitorForEvents(matching: .scrollWheel) { [weak self] _ in self?.handleScrollEvent() }`
    3. `handleScrollEvent()`: call `clearResults()` immediately, then `windowMoveDebouncer.keystroke()` (reuses same debouncer as window move — they share the same "clear + re-lint after settle" behavior)
    4. In `stop()`: `NSEvent.removeMonitor(scrollMonitor)` and nil it out
  - Notes: Same pattern as the global click monitor in `OverlayWindowController`. The scroll monitor fires for any scroll anywhere — this is imprecise but safe (worst case: brief underline flicker on unrelated scroll). The debouncer prevents excessive re-lints.

- [x] **Task 9: Add constants**
  - File: `Spell-i/Utilities/Constants.swift`
  - Action: Add to the Timing section:
    ```swift
    static let windowMoveDebounceInterval: TimeInterval = 0.3
    static let engineRetryDelays: [TimeInterval] = [2.0, 5.0, 10.0]
    ```
  - Notes: `windowMoveDebounceInterval` used by the window move/scroll debouncer. `engineRetryDelays` used by retry logic (index = attempt number).

- [x] **Task 10: Add tests**
  - Files: `Spell-iTests/App/StatusBarControllerTests.swift` (new), `Spell-iTests/TextMonitoring/FocusTrackerTests.swift` (new)
  - Action:
    1. **StatusBarControllerTests**: Test `updateState()` sets correct icon for each `EngineState` value. Test menu contains status text item. These are pure UI state tests — no AX needed.
    2. **FocusTrackerTests**: Test `handleAppActivation` updates `currentBundleID`. Test that creating/destroying a FocusTracker doesn't crash. Test delegate protocol conformance.
    3. **Do NOT test** `TextMonitorCoordinator` directly — it's a concrete class with hard-wired AX/CGEventTap dependencies. Would require protocol abstractions that are out of scope. Verify coordinator behavior through manual testing instead.
  - Notes: Keep tests realistic. Don't write tests that require AX permissions or running event loops. Focus on testable state logic.

### Acceptance Criteria

- [x] **AC 1**: Given the app is launched and SpellEngine initializes normally, when the user types misspelled words, then underlines appear (existing behavior preserved) and menu bar icon shows active state with "Engine: Ready" in menu.

- [x] **AC 2**: Given the app is launched and `start()` is called before engine init completes, when `performLint()` is triggered (via keystroke or focus change) while engine is nil, then no crash occurs, no underlines are shown, and a debug log message is emitted.

- [x] **AC 3**: Given the Rust `SpellEngine::new()` panics, when the Swift side receives the engine, then `is_degraded()` returns true, `lint_text()` returns empty results, the menu bar shows degraded state, and retry is scheduled. If retry succeeds (fresh `SpellEngine()` returns non-degraded), linting begins working.

- [x] **AC 4**: Given underlines are displayed over misspelled words in a window, when the user drags/moves that window, then the underlines are cleared immediately (not left floating at the old position).

- [x] **AC 5**: Given the user has finished moving a window that had spelling errors, when 300ms passes after the last move event, then a re-lint is triggered and underlines appear at the correct new positions.

- [x] **AC 6**: Given underlines are displayed and the user scrolls within the text view, when scroll events fire, then underlines are cleared immediately and re-lint is triggered after 300ms settle.

- [x] **AC 7**: Given the user switches to a different app, when the new app's window is later moved, then move/resize observers apply to the new app's window (old observers are torn down cleanly).

- [x] **AC 8**: Given the Rust engine, when `cargo test` is run, then all existing 13 tests pass plus new tests for panic-safe constructor and degraded engine behavior.

- [x] **AC 9**: Given the Swift test suite, when `xcodebuild test` is run, then new tests for status bar state transitions, focus tracker lifecycle, and related logic all pass.

## Additional Context

### Dependencies

- No new external dependencies required
- Existing: `harper-core` (via Rust FFI), `swift-bridge`, AppKit Accessibility APIs
- Task 3 depends on Task 1 (Rust changes must be compiled first via `./build-rust.sh`)
- Task 4 depends on Task 3 (AppDelegate uses new coordinator API)
- Task 5 depends on Task 3 (StatusBarController uses EngineState enum)
- Task 7 depends on Task 6 (delegate method must exist before implementation)
- Task 8 can be done in parallel with Task 6-7
- Task 10 depends on Tasks 5-6

### Testing Strategy

**Rust unit tests (`cargo test`):**
- `test_engine_new_does_not_panic` — verify normal construction returns non-degraded engine
- `test_degraded_engine_returns_empty_results` — verify degraded engine returns zero lints

**Swift unit tests (`xcodebuild test`):**
- `StatusBarControllerTests` — icon state changes for each EngineState, status menu item text
- `FocusTrackerTests` — app activation handling, bundleID tracking, setup/teardown lifecycle

**Manual testing (required — AX integration can't be unit tested):**
1. Build and run normally → verify spell checking works end-to-end
2. Open TextEdit, type misspelled word → verify underline appears
3. Drag TextEdit window → verify underline clears and reappears at new position
4. Resize TextEdit window → verify same clear + reposition behavior
5. Scroll within TextEdit → verify underline clears and reappears
6. Switch between apps → verify overlay clears and reappears correctly
7. Check menu bar icon → verify it shows active state
8. Check menu bar menu → verify status line shows "Engine: Ready"

### Notes

- **F1 addressed**: Root cause rewritten as uncertain. Solution handles all failure modes (panic, hang, error) by decoupling startup from engine init.
- **F2 addressed**: Retry logic added as explicit part of Task 3 with exponential backoff (2s/5s/10s, max 3 attempts).
- **F3 addressed**: Fallback engine is "dead" (degraded flag + early return in lint_text). Does NOT try to reconstruct LintGroup. Retry creates fresh SpellEngine() for full re-init.
- **F4 addressed**: Window move handling uses `TypingDebouncer` instance (proper reset-on-each-event debouncing), not `asyncAfter`.
- **F5 addressed**: Scroll events handled via global `NSEvent.addGlobalMonitorForEvents(matching: .scrollWheel)` — same pattern as existing click monitor.
- **F6 addressed**: Thread safety rule made explicit in Codebase Patterns section.
- **F7 addressed**: Coordinator tests dropped. Only StatusBarController and FocusTracker tested (actually testable without AX).
- **F8 addressed**: Build step note added — `./build-rust.sh` regenerates Generated/ automatically as Xcode pre-build.
- **F9 addressed**: Electron/Chromium limitation documented in Known Limitations section.
- **F10 addressed**: Degraded engine behavior defined — `lint_text()` returns empty, no underlines shown, clear state.
- **Risk: `swift-bridge` FFI** — `&self -> bool` is standard for opaque types. If it fails, fall back to `&self -> u8` with 0/1 encoding.
- **Risk: AX window element lifetime** — stale elements return error codes harmlessly. Teardown-on-app-switch handles cleanup.
- Accessibility permission resets on rebuild — separate known issue, not addressed here.

## Review Notes
- Adversarial review completed (14 findings)
- Findings: 14 total, 8 fixed (F1, F2, F3, F5, F7, F9, F12, F13), 1 reclassified as noise (F4), 5 skipped (F6 noise, F8 undecided, F10 undecided, F11 noise, F14 noise)
- Resolution approach: auto-fix real findings
