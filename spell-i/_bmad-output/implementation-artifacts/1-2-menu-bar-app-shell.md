# Story 1.2: Menu Bar App Shell

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a user,
I want to see Spell-i in my menu bar and control it from there,
so that I can enable, disable, or quit spell checking without a Dock icon cluttering my workspace.

## Acceptance Criteria

1. **Menu Bar Presence:** Given the user launches Spell-i, when the app starts, then a menu bar icon appears in the system status bar. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.2]
2. **Dock Suppression:** When the app is running, no Dock icon is shown (`LSUIElement=YES`). [Source: _bmad-output/planning-artifacts/epics.md#Story 1.2]
3. **Menu Content:** Given the menu bar icon is visible, when the user clicks it, then a dropdown menu appears with "Enable Spell Checking" (or "Disable" if active), a separator, and "Quit". [Source: _bmad-output/planning-artifacts/epics.md#Story 1.2]
4. **State Toggle (Disable):** Given spell checking is enabled, when the user clicks "Disable", then the menu item updates to show "Enable", and the `TextMonitorCoordinator` is signaled to stop monitoring. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.2]
5. **State Toggle (Enable):** Given spell checking is disabled, when the user clicks "Enable", then the menu item updates to show "Disable", and the `TextMonitorCoordinator` is signaled to start monitoring. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.2]
6. **Clean Exit:** Given the app is running, when the user clicks "Quit", then the app terminates cleanly with no orphaned processes. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.2]
7. **Accessibility:** The menu bar button has an `accessibilityTitle` set for VoiceOver support. [Source: Google Research - macOS 14 Best Practices]

## Tasks / Subtasks

- [x] **Task 1: App Shell Configuration (AC: 2)**
  - [x] Verify `Info.plist` has `LSUIElement` set to `YES`.
  - [x] Set `NSApplication` activation policy to `.accessory` in `main.swift` or `AppDelegate`.
- [x] **Task 2: Status Bar Controller Implementation (AC: 1, 3, 7)**
  - [x] Create `StatusBarController.swift` in `Spell-i/App/`.
  - [x] Initialize `NSStatusItem` using `NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)`.
  - [x] Configure `statusItem.button` with a template image (placeholder for now) and `accessibilityTitle`.
- [x] **Task 3: Menu Construction & State Management (AC: 3, 4, 5)**
  - [x] Build `NSMenu` with items for Enable/Disable and Quit.
  - [x] Implement `toggleSpellChecking()` action in `StatusBarController`.
  - [x] Link toggle action to `TextMonitorCoordinator.start()` / `stop()` methods.
  - [x] Update menu item title/state dynamically based on `TextMonitorCoordinator` state.
- [x] **Task 4: App Delegate Integration (AC: 1, 6)**
  - [x] Initialize `StatusBarController` in `AppDelegate.applicationDidFinishLaunching`.
  - [x] Ensure `Quit` menu item calls `NSApp.terminate(_:)`.
- [x] **Task 5: Coordinator Skeleton (AC: 4, 5)**
  - [x] Create `TextMonitorCoordinator.swift` in `Spell-i/TextMonitoring/`.
  - [x] Implement `start()` and `stop()` methods (empty placeholders for now, with `os_log` statements).

## Dev Notes

- **Architecture Pattern:** Use the `polpiella.dev` AppKit menu bar pattern as identified in the architecture document. [Source: _bmad-output/planning-artifacts/architecture.md#Selected Starter]
- **Threading:** Ensure UI updates (menu item changes) occur on the main thread.
- **Logging:** Use `os_log(.info, log: .app, "...")` for lifecycle events.
- **Visuals:** Use a simple template image for the menu bar icon to ensure it adapts to light/dark modes.

### Project Structure Notes

- `Spell-i/App/AppDelegate.swift`
- `Spell-i/App/StatusBarController.swift`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`
- `Spell-i/Utilities/Constants.swift`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/ux-design-specification.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash

### Debug Log References

- Verified `LSUIElement=YES` in `Info.plist`.
- Confirmed `app.setActivationPolicy(.accessory)` in `main.swift`.
- `StatusBarController` updated with `NSStatusItem.variableLength` and `accessibilityTitle`.
- Menu toggle logic refined to change titles between "Enable" and "Disable".

### Completion Notes List

- Menu bar app shell fully implemented with `StatusBarController`.
- `AppDelegate` handles initialization and coordination between sub-systems.
- `TextMonitorCoordinator` skeleton with `start()`/`stop()` lifecycle management.
- Proper Dock icon suppression using `.accessory` activation policy.
- VoiceOver support added to the menu bar button.
- **Code Review Fixes:**
  - Prevented duplicate menu bar icon creation by making `setupApp()` idempotent.
  - Fixed accessibility of menu bar actions by adjusting visibility for the Obj-C runtime.
  - Removed dangerous global "q" key equivalent to avoid accidental application quits.
  - Unified logging subsystem across all components.

### File List

- `Spell-i/Info.plist`
- `Spell-i/App/main.swift`
- `Spell-i/App/AppDelegate.swift`
- `Spell-i/App/StatusBarController.swift`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`

