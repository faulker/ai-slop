# Story 1.3: Main Window & App Lifecycle

Status: review

## Story

As a user,
I want to open the full Cirrus window and have the app persist in my menu bar,
so that closing the window doesn't stop my background operations.

## Acceptance Criteria

1. **Given** the tray popup is open
   **When** the user clicks "Open Cirrus"
   **Then** the main GUI window opens with a TabView containing Profiles, History, and Settings tabs (FR52)
   **And** the tray popup dismisses

2. **Given** the main GUI window is open
   **When** the user closes the window (Cmd+W or red close button)
   **Then** the window closes but the app continues running in the menu bar (FR60)
   **And** the menu bar icon remains visible and clickable

3. **Given** the user clicks "Quit" in the tray popup
   **When** the quit action is triggered
   **Then** a confirmation dialog appears (NFR20)
   **And** if confirmed, the app terminates completely (FR61)

4. **Given** the app is configured as a Login Item
   **When** the Mac starts up or the user logs in
   **Then** the app launches automatically and appears silently in the menu bar (FR59, FR62)

## Tasks / Subtasks

- [x] Task 1: Create MainWindowView with TabView (AC: #1)
  - [x] 1.1: Create `Views/MainWindow/MainWindowView.swift` with TabView
  - [x] 1.2: Add three tabs: Profiles, History, Settings (placeholder views for now)
  - [x] 1.3: Each tab uses `.tabItem { Label("name", systemImage: "sf.symbol") }`
- [x] Task 2: Wire "Open Cirrus" button (AC: #1)
  - [x] 2.1: In `AppDelegate`, add method to show main window
  - [x] 2.2: Use `NSWindow` or SwiftUI `Window` scene to present `MainWindowView`
  - [x] 2.3: Dismiss tray popup when main window opens
  - [x] 2.4: If window already open, bring to front with `makeKeyAndOrderFront`
- [x] Task 3: Implement close-keeps-running (AC: #2)
  - [x] 3.1: Override window close behavior so closing the window doesn't terminate the app
  - [x] 3.2: Use `NSWindow.delegate` `windowShouldClose` or SwiftUI `.defaultAppStorage` approach
  - [x] 3.3: Verify menu bar icon remains after window close
- [x] Task 4: Implement quit with confirmation (AC: #3)
  - [x] 4.1: Replace direct `terminate` call from Story 1.2 with confirmation dialog
  - [x] 4.2: Show `.alert` with "Are you sure you want to quit Cirrus?" (NFR20)
  - [x] 4.3: On confirm, call `NSApplication.shared.terminate(nil)`
  - [x] 4.4: On cancel, dismiss alert
- [x] Task 5: Implement Login Item (AC: #4)
  - [x] 5.1: Use `SMAppService.mainApp` to register/unregister as Login Item (macOS 13+ API)
  - [x] 5.2: Add toggle in Settings tab placeholder to enable/disable login item
  - [x] 5.3: App launches silently — `LSUIElement = true` already set in Story 1.1
- [x] Task 6: Write tests
  - [x] 6.1: Test quit confirmation dialog appears on quit action
  - [x] 6.2: Test Login Item registration/unregistration via `SMAppService`

## Dev Notes

### Architecture Compliance

**Layer:** Views (MainWindow) + App layer (AppDelegate modifications, CirrusApp scene).

**File locations:**
```
Cirrus/Cirrus/
├── CirrusApp.swift                          # MODIFY — add Window scene
├── AppDelegate.swift                        # MODIFY — add main window management, quit confirmation
└── Views/
    └── MainWindow/
        ├── MainWindowView.swift             # NEW — TabView root
        ├── Profiles/
        │   └── ProfileListView.swift        # NEW — placeholder "Profiles coming in Epic 2"
        ├── History/
        │   └── HistoryTabView.swift         # NEW — placeholder "History coming in Epic 4"
        └── Settings/
            └── SettingsTabView.swift        # NEW — placeholder with Login Item toggle
```

### Technical Requirements

**Window management approach:**
- Use SwiftUI `Window` scene in `CirrusApp.body` for the main window
- `AppDelegate` controls window visibility via `NSApp.activate(ignoringOtherApps:)` and window ordering
- Do NOT use `WindowGroup` — use `Window("Cirrus", id: "main")` for single-window behavior
- Set minimum window size: 700x500

**Close-keeps-running:**
- Since `LSUIElement = true` and the app is menu-bar-only, closing the window naturally keeps the app alive
- If using SwiftUI `Window` scene, closing just hides the window. The app continues because there's no `WindowGroup` managing termination.
- Verify with `applicationShouldTerminateAfterLastWindowClosed` returning `false` in AppDelegate

**Login Item (SMAppService):**
```swift
import ServiceManagement

// Register
try SMAppService.mainApp.register()

// Unregister
try SMAppService.mainApp.unregister()

// Check status
SMAppService.mainApp.status == .enabled
```
- Requires Login Items capability (added in Story 1.1)
- No user prompt — silently registers/unregisters
- Works with macOS 13+ (`ServiceManagement` framework)

**Quit confirmation dialog:**
- Triggered from tray popup "Quit" button
- In future (Epic 5), the dialog will also warn about active schedules (FR34)
- For now, simple "Are you sure?" confirmation is sufficient

### Enforcement Rules

- `applicationShouldTerminateAfterLastWindowClosed` MUST return `false`
- Use `SMAppService.mainApp` — NOT legacy `LSSharedFileList` APIs
- Main window is a single `Window` scene — NOT `WindowGroup` (prevents multiple instances)
- Destructive action (quit) requires confirmation dialog (NFR20)

### Dependencies

- **Depends on:** Story 1.1 (project, Login Items capability), Story 1.2 (AppDelegate, tray popup, "Open Cirrus" button)
- **Does NOT depend on:** Epics 2-5

### References

- [Source: architecture.md#Core Architectural Decisions] — State management, window lifecycle
- [Source: architecture.md#View Patterns] — SwiftUI view rules, sheets/alerts
- [Source: epics.md#Story 1.3] — Acceptance criteria

## Dev Agent Record

### Agent Model Used
Claude Opus 4.6

### Debug Log References
- Also addressed code review findings from Story 1.2: fixed monitor leak on escape/quit, screen bounds clamping, coordinate conversion bug, added @MainActor to TrayPopupPanel

### Completion Notes List
- MainWindowView with TabView (Profiles, History, Settings tabs with placeholder views)
- Window("Cirrus", id: "main") scene in CirrusApp for single-window behavior
- AppDelegate.openMainWindow() finds existing window or creates new, activates app
- applicationShouldTerminateAfterLastWindowClosed returns false (close-keeps-running)
- Quit confirmation via NSAlert from both tray popup and Cmd+Q
- TrayPopupView now uses onQuit callback instead of direct terminate
- Login Item toggle in SettingsTabView using SMAppService.mainApp
- Fixed Story 1.2 review items: onDismiss callback for escape dismiss, screen bounds clamping, button.convert(bounds, to: nil) for coordinate conversion
- 36 total tests passing

### File List
- `Cirrus/Cirrus/CirrusApp.swift` (MODIFIED — Window scene)
- `Cirrus/Cirrus/AppDelegate.swift` (MODIFIED — quit confirmation, main window management, onDismiss)
- `Cirrus/Cirrus/Views/TrayPopup/TrayPopupPanel.swift` (MODIFIED — @MainActor, onDismiss, screen clamping, coord fix)
- `Cirrus/Cirrus/Views/TrayPopup/TrayPopupView.swift` (MODIFIED — onQuit callback, shared dimensions)
- `Cirrus/Cirrus/Views/MainWindow/MainWindowView.swift` (NEW)
- `Cirrus/Cirrus/Views/MainWindow/Profiles/ProfileListView.swift` (NEW)
- `Cirrus/Cirrus/Views/MainWindow/History/HistoryTabView.swift` (NEW)
- `Cirrus/Cirrus/Views/MainWindow/Settings/SettingsTabView.swift` (NEW)
- `Cirrus/CirrusTests/Views/MainWindow/MainWindowViewTests.swift` (NEW)
