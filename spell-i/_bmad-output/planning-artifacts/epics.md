---
stepsCompleted: [1]
inputDocuments:
  - '_bmad-output/planning-artifacts/prd.md'
  - '_bmad-output/planning-artifacts/architecture.md'
  - '_bmad-output/planning-artifacts/ux-design-specification.md'
---

# spell-i - Epic Breakdown

## Overview

This document provides the complete epic and story breakdown for spell-i, decomposing the requirements from the PRD, UX Design if it exists, and Architecture requirements into implementable stories.

## Requirements Inventory

### Functional Requirements

FR1: User can launch Spell-i and see a menu bar icon indicating the app is running
FR2: User can enable or disable spell checking globally via the menu bar menu
FR3: User can quit the application from the menu bar menu
FR4: App runs as a menu bar-only application with no Dock icon
FR5: App detects whether Accessibility permission has been granted
FR6: App presents an onboarding window explaining why Accessibility permission is needed and that all processing is offline
FR7: User can navigate to System Settings to grant Accessibility permission from the onboarding window
FR8: App detects when Accessibility permission is granted and dismisses the onboarding window automatically
FR9: App displays a clear message when Accessibility permission is not granted and spell checking cannot function
FR10: App detects keystrokes system-wide across all applications
FR11: App waits for a typing pause before initiating a spell check
FR12: App reads the text content of the currently focused text element in any application
FR13: App reads the cursor position within the focused text element
FR14: App tracks which application is currently focused
FR15: App re-enables keystroke monitoring if macOS disables it
FR16: App checks text for spelling errors using the Harper engine
FR17: App checks text for grammar errors using the Harper engine
FR18: App returns error locations, categories, messages, and suggested corrections for each detected issue
FR19: App supports English (American dialect) spell and grammar checking
FR20: App draws squiggly underlines beneath misspelled or grammatically incorrect words in the host application
FR21: App positions underlines accurately at the screen coordinates of the erroneous text
FR22: App clears underlines when the user switches to a different application
FR23: App updates underlines when text changes (new errors appear, fixed errors disappear)
FR24: Underlines do not interfere with the user's ability to interact with the host application
FR25: User can click on an underlined word to see a popup with suggested corrections
FR26: User can select a suggestion to replace the misspelled word in the host application
FR27: The host application's text is updated with the selected correction
FR28: The correction popup dismisses after a correction is applied
FR29: User can dismiss the correction popup without applying a correction
FR30: User can add a word to their personal dictionary from the correction popup
FR31: Words added to the dictionary are no longer flagged as errors in any application
FR32: The user dictionary persists across application restarts
FR33: The user dictionary is stored as a human-readable file

### NonFunctional Requirements

NFR1: CGEventTap callback adds < 1ms latency per keystroke
NFR2: Spell check (debounce fire to overlay update) completes within 15ms
NFR3: Correction popup appears within 15ms of clicking an underlined word
NFR4: Text replacement completes within 50ms of selecting a suggestion
NFR5: App reaches lint-ready state within 3 seconds of launch
NFR6: Memory footprint remains below 30MB resident during normal use
NFR7: CPU usage stays below 0.1% when idle
NFR8: CPU usage stays below 2% during continuous typing
NFR9: FFI boundary crossing overhead remains below 0.1ms per spell check cycle
NFR10: App makes zero network calls — all processing is local
NFR11: No telemetry, analytics, or usage data is collected or transmitted
NFR12: Text read from host applications is processed in-memory and never persisted (except user dictionary words)
NFR13: No user account, registration, or authentication required
NFR14: User dictionary stored only on the local filesystem under the user's control
NFR15: App runs continuously for 8+ hour sessions without crashes or memory leaks
NFR16: App automatically re-enables CGEventTap if macOS disables it via `tapDisabledByTimeout`
NFR17: App continues to function (menu bar presence, no crash) if the Harper engine fails to initialize
NFR18: Overlay clears cleanly when switching applications with no visual artifacts
NFR19: App handles rapid application switching without crashes or stale underlines
NFR20: User dictionary writes are atomic — no data loss on unexpected quit

### Additional Requirements

From Architecture:
- Starter Template: Assemble from swift-bridge codegen-visualizer and polpiella.dev AppKit menu bar pattern.
- FFI Bridge: Minimal 4-function bridge (new, lint_text, add_user_word, remove_user_word).
- Threading: Single serial background queue (com.spell-i.engine) for all FFI operations.
- Error Handling: Never-fail FFI (always return empty Vec on error).
- Coordinate System: Full-screen NSWindow overlay with direct AX→view Y-flip translation.
- Build Config: arm64 primary for MVP.

From UX Design:
- Zero animations: State transitions must be instantaneous.
- Silent degradation: No error UI for unsupported apps or AX failures.
- Interaction: Plain click on underlined word triggers popup; two-click correction flow.
- Visual Language: Red squiggly for spelling, blue for grammar.
- Accessibility: VoiceOver support for correction popup and onboarding.

### FR Coverage Map

FR1: Epic 1 - App launch and menu bar icon visibility
FR2: Epic 3 - Enable/disable toggle in menu bar
FR3: Epic 3 - Quit application from menu bar
FR4: Epic 1 - Dock icon suppression (LSUIElement)
FR5: Epic 1 - Accessibility permission detection
FR6: Epic 1 - Onboarding window explanation
FR7: Epic 1 - Navigation to System Settings
FR8: Epic 1 - Auto-dismiss onboarding on grant
FR9: Epic 1 - Permission failure messaging
FR10: Epic 2 - System-wide keystroke detection
FR11: Epic 2 - Typing pause debounce mechanism
FR12: Epic 2 - Focused text element reading
FR13: Epic 2 - Cursor position tracking
FR14: Epic 2 - Active application tracking
FR15: Epic 2 - Event tap re-enable recovery
FR16: Epic 1 - Engine initialization (moved to foundation)
FR17: Epic 2 - Grammar checking integration
FR18: Epic 2 - Error result processing (offsets, messages)
FR19: Epic 2 - American English dialect support
FR20: Epic 2 - Squiggly underline rendering
FR21: Epic 2 - Precise overlay positioning
FR22: Epic 2 - Overlay clearing on app switch
FR23: Epic 2 - Real-time underline updates
FR24: Epic 2 - Non-interfering overlay transparency
FR25: Epic 3 - Correction popup on click
FR26: Epic 3 - Suggestion selection interaction
FR27: Epic 3 - Text replacement via AX API
FR28: Epic 3 - Popup dismissal after correction
FR29: Epic 3 - Popup dismissal on click-away
FR30: Epic 3 - Add to Dictionary interaction
FR31: Epic 3 - Dictionary-based error suppression
FR32: Epic 3 - User dictionary persistence
FR33: Epic 3 - Plain text dictionary storage

## Epic 1: App Foundation & Onboarding

Users can launch the app, see it in the menu bar, understand the privacy-first model, and easily grant the necessary permissions to start spell-checking.

### Story 1.1: Project Scaffolding & FFI Bridge Validation

As a developer,
I want a working Xcode + Rust project with a validated FFI bridge,
so that all subsequent features can be built on the hybrid Swift/Rust architecture.

**Acceptance Criteria:**

**Given** a fresh clone of the repository
**When** the Xcode project is built
**Then** the `build-rust.sh` script compiles `spell-i-engine` via Cargo
**And** `swift-bridge` generates Swift/C bridging code in `Generated/` during the build process
**And** Xcode links `libspell_i_engine.a` successfully and the app binary launches without crash
**And** when a test harness calls `SpellEngine.new()` and `lintText("This is a tset")`, then the engine returns at least one `LintResult` with a suggestion for "test"
**And** the FFI round-trip (Swift -> Rust -> Swift) completes without crash or memory error
**And** when `cargo test` is run in `spell-i-engine/`, then all unit tests pass verifying `lint_text` returns correct results for known misspellings.

### Story 1.2: Menu Bar App Shell

As a user,
I want to see Spell-i in my menu bar and control it from there,
so that I can enable, disable, or quit spell checking without a Dock icon cluttering my workspace.

**Acceptance Criteria:**

**Given** the user launches Spell-i
**When** the app starts
**Then** a menu bar icon appears in the system status bar
**And** no Dock icon is shown (`LSUIElement=YES`)
**And** when the user clicks the icon, a menu appears with "Enable Spell Checking" (or "Disable" if active), a separator, and "Quit"
**And** when the user clicks "Quit", then the app terminates cleanly
**And** the menu bar button has an `accessibilityTitle` set for VoiceOver support.

### Story 1.3: Accessibility Permission & Onboarding

As a user,
I want to understand why Spell-i needs Accessibility permission and grant it easily,
so that I can start using the app quickly and trust that my data stays private.

**Acceptance Criteria:**

**Given** the user launches Spell-i for the first time without Accessibility permission
**When** the app starts
**Then** an onboarding window appears (~400x250px, centered) explaining the need for permission and the "checked offline" privacy promise
**And** a "Open System Settings" button is displayed with a dark green accent color
**When** the user clicks the button
**Then** System Settings opens to the Accessibility pane
**And** when the user grants permission, the onboarding window detects the grant and dismisses automatically
**And** the app then proceeds directly to the active state or `setupApp()` logic.

Users get immediate visual feedback on their writing across any application. Typographical and grammatical errors are highlighted automatically without interrupting their typing flow.

### Story 2.1: Keystroke Detection & Typing Debounce

As a developer,
I want to detect when the user is typing and when they pause,
so that I can trigger spell checking without interfering with their active writing flow.

**Acceptance Criteria:**

**Given** the app has Accessibility permission and is enabled
**When** the user types in any application
**Then** the `EventTapManager` detects keystrokes system-wide
**And** the `TypingDebouncer` resets its timer (default 400ms) on every keystroke
**When** the user stops typing for 400ms
**Then** the debouncer triggers a callback to signal a spell check is needed
**And** if macOS disables the event tap due to timeout, the manager automatically re-installs it.

### Story 2.2: Text Reading & Focus Tracking

As a developer,
I want to read the text and cursor position from the focused application,
so that I know exactly what to check and where the user is currently working.

**Acceptance Criteria:**

**Given** a typing pause is detected
**When** the `AccessibilityReader` is invoked
**Then** it identifies the currently focused UI element and its parent application
**And** it extracts the full text content and current cursor range
**And** it identifies if the application is on a "blacklist" (like a password field or terminal if configured) to skip reading
**And** the `FocusTracker` triggers a "clear overlay" event when the user switches between different applications.

### Story 2.3: Harper Engine Integration

As a developer,
I want to process the extracted text through the Harper engine,
so that I can identify spelling and grammar errors with high performance.

**Acceptance Criteria:**

**Given** a text string and cursor context
**When** the `TextMonitorCoordinator` dispatches to the background engine queue
**Then** the Rust `SpellEngine` lints the text and returns a list of `LintResults`
**And** the results include character offsets, error categories (spelling/grammar), and clear messages
**And** the processing completes in under 15ms for typical paragraph lengths
**And** the coordinator converts the Rust byte-offsets into Swift-compatible character indices correctly.

### Story 2.4: Overlay Window & Squiggly Underlines

As a user,
I want to see visual indicators directly under my typos,
so that I can identify errors without leaving my current application.

**Acceptance Criteria:**

**Given** the engine has returned lint results with character ranges
**When** the `OverlayPositionCalculator` maps these ranges to screen coordinates using `AXBoundsForRange`
**Then** the `OverlayWindow` renders squiggly underlines at those exact positions
**And** spelling errors are red and grammar errors are blue
**And** the overlay is transparent and does not intercept mouse clicks by default (allowing normal interaction with the host app)
**And** the underlines disappear instantly when the user switches apps or corrects the text.

Users can fix their mistakes with a couple of clicks and "teach" the app their personal vocabulary, ensuring the tool becomes more accurate and personalized over time.

### Story 3.1: Correction Popup & Text Replacement

As a user,
I want to click an underlined word and choose a correction,
so that I can fix my typos instantly without re-typing the whole word.

**Acceptance Criteria:**

**Given** a squiggly underline is visible
**When** the user clicks directly on the underlined region
**Then** a `CorrectionPopup` appears anchored below the word
**And** the popup displays a list of suggestions from the Harper engine
**When** the user clicks a suggestion
**Then** the `TextReplacer` uses the Accessibility API to replace the original word in the host app
**And** the popup dismisses and the underline is removed
**And** if the user clicks away from the popup, it dismisses without making any changes.

### Story 3.2: User Dictionary Management

As a user,
I want to add specialized terms to my personal dictionary,
so that the app stops flagging them as errors across all my applications.

**Acceptance Criteria:**

**Given** the correction popup is open for a word
**When** the user clicks "Add to Dictionary"
**Then** the word is passed to the Rust engine and appended to `~/Library/Application Support/Spell-i/dictionary.txt`
**And** the engine immediately re-lints the current text, causing the underline to disappear
**And** the word remains ignored even after the app or system is restarted
**And** the dictionary file remains human-readable as a plain text file.

<!-- Repeat for each epic in epics_list (N = 1, 2, 3...) -->
