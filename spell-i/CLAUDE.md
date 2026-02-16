# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Generate Xcode project from spec (required after adding/removing files)
xcodegen generate

# Build (includes Rust pre-build step automatically)
xcodebuild -project Spell-i.xcodeproj -scheme Spell-i -configuration Debug build SYMROOT="$(pwd)/build"

# Build and run
open build/Debug/Spell-i.app

# Build Rust library manually (normally done by Xcode pre-build script)
./build-rust.sh

# Run Rust tests
cd spell-i-engine && cargo test

# Run Swift tests
xcodebuild -project Spell-i.xcodeproj -scheme Spell-i -configuration Debug test

# Validate FFI bridge
./build/Debug/Spell-i.app/Contents/MacOS/Spell-i --validate-ffi
```

## Architecture

Spell-i is a macOS menu bar app that provides real-time spell/grammar checking across all apps using the Accessibility API. It draws squiggly underlines over errors via a transparent overlay window.

### Pipeline

```
CGEventTap (keystroke detection, listen-only)
  → TypingDebouncer (400ms)
  → AccessibilityReader (AX API: read focused element text)
  → SpellEngine (Harper via Rust FFI, runs on background engineQueue)
  → OverlayWindowController (transparent full-screen window with squiggly underlines)
  → CorrectionPopupController (NSMenu with suggestions on underline click)
```

**TextMonitorCoordinator** orchestrates the entire pipeline. It owns all sub-components, dispatches lint to a background queue, and handles the main-thread ↔ background-thread handoff (AX calls must happen on main thread; Harper linting on background).

### Rust FFI Bridge

- **Engine**: `spell-i-engine/` crate wraps `harper-core` for spell/grammar checking
- **Bridge**: `swift-bridge` generates FFI code into `Generated/`
- **Bridging header**: `Spell-i/BridgingHeader.h` includes generated C headers
- **Static library**: `libspell_i_engine.a` linked at project root
- **Opaque types**: `SpellEngine` and `LintResults` cross FFI as opaque pointers with accessor methods (swift-bridge can't pass `Vec<Struct>`)
- **Offsets**: Harper returns Unicode scalar (character) offsets, NOT UTF-8 byte offsets; must navigate via `unicodeScalars` view then convert to UTF-16 for NSRange/AX APIs

### Coordinate Systems

This is the trickiest part of the codebase:
- **AX API** returns bounds in Quartz coordinates (top-left origin, Y increases downward)
- **NSScreen.frame** uses Cocoa coordinates (bottom-left origin, Y increases upward)
- **Overlay views** use `isFlipped = true` so they match Quartz/AX (top-left origin)
- **`OverlayPositionCalculator.viewRect`** converts AX rect → local view coords (just screen offset, no Y-flip needed since both are top-left)
- **NSWindow frame** positioning requires converting back to Cocoa bottom-left coords

### Click Detection

The overlay window has `ignoresMouseEvents = true` (passes all clicks through). A `NSEvent.addGlobalMonitorForEvents` detects clicks at underline positions and triggers the correction popup. The correction popup uses `NSMenu` (not a custom window) for reliable event delivery in an accessory app.

### Key Threading Rules

- AX reads/writes: **main thread only**
- Harper linting: **engineQueue** (background, `.userInitiated`)
- Generation counter (`lintGeneration`): prevents stale results from overwriting fresh ones
- `lastLintElement`: stored AXUIElement for corrections even after focus changes

## Important Gotchas

- **Accessibility permission resets on rebuild** — each new binary needs re-granting in System Settings
- **Electron/Chromium apps** (Slack, VS Code, Discord) require deep AX traversal (`deepFocusedElement`, `findTextElementInChildren`) — the focused element is often a web container, not the text input
- **Text editor role filter** — only `AXTextArea`, `AXTextField`, `AXComboBox`, `AXSearchField` support `kAXBoundsForRangeParameterizedAttribute`; other elements with text values will fail bounds queries
- **Focus change on popup click** clears `currentResults` — correction data (range, element) must be captured at popup-show time, not click time
- **`LSUIElement = YES`** — app is accessory-only (menu bar), no Dock icon, which affects window event delivery
- **Google Drive can evict source files** — files on this path may disappear from local filesystem; always commit important changes to git
- **Harper panics** on some malformed input — the Rust layer catches panics and returns empty results
- **User dictionary** at `~/Library/Application Support/Spell-i/dictionary.txt` — plain text, one word per line, case-insensitive
