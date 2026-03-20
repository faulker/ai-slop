# Story 1.2: Menu Bar Icon & Tray Popup Shell

Status: review

## Story

As a user,
I want to see a Cirrus icon in my menu bar that opens a popup when clicked,
so that I can access the app quickly from anywhere on my Mac.

## Acceptance Criteria

1. **Given** the app is launched
   **When** the app finishes starting
   **Then** a grayscale template image appears in the macOS menu bar (FR43)
   **And** no Dock icon is visible (FR62)

2. **Given** the menu bar icon is visible
   **When** the user clicks the menu bar icon
   **Then** a borderless floating popup panel appears anchored below the icon
   **And** the popup uses `NSVisualEffectView` for vibrancy/translucency
   **And** the popup has `.accessibilityRole(.popover)` for VoiceOver

3. **Given** the popup is open
   **When** the user clicks outside the popup, presses Escape, or re-clicks the menu bar icon
   **Then** the popup dismisses

4. **Given** the popup is open
   **When** the user views the popup content
   **Then** an "Open Cirrus" button is visible (FR52)
   **And** a "Quit" button is visible (FR61)

## Tasks / Subtasks

- [x] Task 1: Create AppDelegate with NSStatusItem (AC: #1)
  - [x] 1.1: Create `AppDelegate.swift` with `NSApplicationDelegate` conformance
  - [x] 1.2: Set up `NSStatusItem` with `NSStatusBar.system.statusItem(withLength: .squareLength)`
  - [x] 1.3: Configure status item button with template image from `MenuBarIcon` asset
  - [x] 1.4: Wire button action to toggle popup visibility
  - [x] 1.5: Connect `AppDelegate` to `CirrusApp` via `@NSApplicationDelegateAdaptor`
- [x] Task 2: Create menu bar icon asset (AC: #1)
  - [x] 2.1: Add `MenuBarIcon.imageset` to `Assets.xcassets` with grayscale template image
  - [x] 2.2: Set "Render As" to "Template Image" in asset catalog
  - [x] 2.3: Provide @1x (18x18) and @2x (36x36) variants
- [x] Task 3: Create TrayPopupPanel (AC: #2, #3)
  - [x] 3.1: Create `Views/TrayPopup/TrayPopupPanel.swift` — `NSPanel` subclass
  - [x] 3.2: Configure as borderless, non-activating floating panel (`.nonactivatingPanel`, `.fullSizeContentView`)
  - [x] 3.3: Add `NSVisualEffectView` as background with `.popover` material
  - [x] 3.4: Embed `TrayPopupView` via `NSHostingView`
  - [x] 3.5: Position panel anchored below the status item
  - [x] 3.6: Implement dismiss on click-outside via `NSEvent.addGlobalMonitorForEvents`
  - [x] 3.7: Implement dismiss on Escape via key event handler
  - [x] 3.8: Set `.accessibilityRole(.popover)` on the hosting view
- [x] Task 4: Create TrayPopupView shell (AC: #4)
  - [x] 4.1: Create `Views/TrayPopup/TrayPopupView.swift` — root SwiftUI view
  - [x] 4.2: Add header with app name "Cirrus"
  - [x] 4.3: Add placeholder content area (will be populated in Story 3.2)
  - [x] 4.4: Add footer with "Open Cirrus" button and "Quit" button
  - [x] 4.5: Wire "Open Cirrus" to open main window (placeholder action for now)
  - [x] 4.6: Wire "Quit" to `NSApplication.shared.terminate(nil)` (no confirmation yet — Story 1.3 adds confirmation)
- [x] Task 5: Write tests
  - [x] 5.1: Test `TrayPopupPanel` initialization and configuration properties

## Dev Notes

### Architecture Compliance

**Layer:** This story creates Views (TrayPopup) and modifies the App layer (AppDelegate). Uses AppKit (`NSStatusItem`, `NSPanel`, `NSHostingView`) bridging to SwiftUI.

**Custom NSStatusItem + NSWindow approach (NOT MenuBarExtra):** Architecture explicitly chose this for full control over borderless window, vibrancy, positioning, and dismiss behavior. Do NOT use `MenuBarExtra`.

**File locations:**
```
Cirrus/Cirrus/
├── AppDelegate.swift                    # NEW — NSStatusItem + popup management
├── Assets.xcassets/
│   └── MenuBarIcon.imageset/           # NEW — grayscale template image
└── Views/TrayPopup/
    ├── TrayPopupPanel.swift            # NEW — NSPanel subclass
    └── TrayPopupView.swift             # NEW — root SwiftUI popup view
```

### Technical Requirements

**TrayPopupPanel (NSPanel subclass):**
- Style mask: `.borderless`, `.nonactivatingPanel`, `.fullSizeContentView`
- Level: `.popUpMenu` (floats above other windows)
- `isOpaque = false`, `backgroundColor = .clear`
- Background: `NSVisualEffectView` with `.popover` material and `.behindWindow` blending mode
- Corner radius: 12pt (standard macOS popover)
- Positioning: Calculate from `statusItem.button?.window?.frame` — center horizontally below icon, offset 4pt below menu bar

**Dismiss behavior:**
- Click outside: `NSEvent.addGlobalMonitorForEvents(matching: .leftMouseDown)` — check if click is outside panel frame
- Escape key: Override `cancelOperation(_:)` or use `keyDown(with:)` in the panel
- Re-click menu bar icon: Toggle in the button action handler
- Remove global monitor when panel closes to avoid leaks

**AppDelegate integration with SwiftUI:**
```swift
@main
struct CirrusApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate

    var body: some Scene {
        // Window scene added in Story 1.3
    }
}
```

**Popup open performance:** Must open within 200ms (NFR1). The panel is pre-created at app launch and shown/hidden — never recreated on each click.

### Enforcement Rules

- The `NSPanel` and `NSHostingView` are AppKit — do not try to do this in pure SwiftUI
- Pre-create the panel at app launch, toggle visibility on click — never recreate
- Use `.popover` material for `NSVisualEffectView` — not `.sidebar` or `.menu`
- Remove global event monitors when panel dismisses to prevent memory leaks
- Accessibility: Set `.accessibilityRole(.popover)` so VoiceOver announces "popover" when opened

### Dependencies

- **Depends on:** Story 1.1 (Xcode project, directory structure)
- **Does NOT depend on:** Any other story

### References

- [Source: architecture.md#Starter Template Evaluation] — Custom NSStatusItem + NSWindow decision
- [Source: architecture.md#Project Structure & Boundaries] — TrayPopup directory
- [Source: architecture.md#View Patterns] — SwiftUI view rules
- [Source: ux-design-specification.md] — Popup vibrancy, dismiss behavior, accessibility
- [Source: epics.md#Story 1.2] — Acceptance criteria

## Dev Agent Record

### Agent Model Used
Claude Opus 4.6

### Debug Log References
- Generated template icon PNGs using Swift script (cloud shape, 18x18 @1x and 36x36 @2x)
- MenuBarIcon asset loads from catalog with SF Symbol fallback

### Completion Notes List
- AppDelegate with NSStatusItem, toggle popup, global click monitor for dismiss
- TrayPopupPanel: NSPanel subclass — borderless, non-activating, popUpMenu level, NSVisualEffectView with .popover material, 12pt corner radius
- TrayPopupView: header (Cirrus), placeholder content, footer (Open Cirrus + Quit buttons)
- Dismiss: click-outside via global monitor, Escape via cancelOperation, re-click toggle
- Accessibility: .popover role on hosting view
- CirrusApp updated with @NSApplicationDelegateAdaptor
- 8 panel configuration tests, 34 total tests passing

### File List
- `Cirrus/Cirrus/AppDelegate.swift` (NEW)
- `Cirrus/Cirrus/CirrusApp.swift` (MODIFIED)
- `Cirrus/Cirrus/Views/TrayPopup/TrayPopupPanel.swift` (NEW)
- `Cirrus/Cirrus/Views/TrayPopup/TrayPopupView.swift` (NEW)
- `Cirrus/Cirrus/Assets.xcassets/MenuBarIcon.imageset/Contents.json` (MODIFIED)
- `Cirrus/Cirrus/Assets.xcassets/MenuBarIcon.imageset/MenuBarIcon_1x.png` (NEW)
- `Cirrus/Cirrus/Assets.xcassets/MenuBarIcon.imageset/MenuBarIcon_2x.png` (NEW)
- `Cirrus/CirrusTests/Views/TrayPopup/TrayPopupPanelTests.swift` (NEW)
