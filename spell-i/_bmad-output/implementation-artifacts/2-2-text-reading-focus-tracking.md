# Story 2.2: Text Reading & Focus Tracking

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a user,
I want the app to read the text I just typed and know which app I'm using,
so that spell checking happens in the right context and clears when I switch apps.

## Acceptance Criteria

1. **Text Extraction:** Given the debouncer has fired after a typing pause, when the `TextMonitorCoordinator` handles the event, then `AccessibilityReader` reads the focused text element's content via `kAXValueAttribute`. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.2]
2. **Cursor & Bounds Extraction:** Reads the cursor position via `kAXSelectedTextRangeAttribute` and character bounds via `kAXBoundsForRangeParameterizedAttribute`. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.2]
3. **Graceful Degradation:** If the focused element does not support AX text reading, `AccessibilityReader` returns `nil` and the pipeline silently skips (no error dialog, no crash). [Source: _bmad-output/planning-artifacts/epics.md#Story 2.2]
4. **App Focus Tracking:** `FocusTracker` monitors active application changes and notifies the coordinator via delegate. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.2]
5. **Overlay Cleanup:** When the user switches to a different application, any existing overlay underlines are cleared immediately (NFR18). [Source: _bmad-output/planning-artifacts/epics.md#Story 2.2]
6. **Stability:** The app handles rapid application switching without crashes or stale underlines (NFR19). [Source: _bmad-output/planning-artifacts/epics.md#Story 2.2]

## Tasks / Subtasks

- [x] **Task 1: Accessibility Reader Implementation (AC: 1, 2, 3)**
  - [x] Create `AccessibilityReader.swift` in `Spell-i/TextMonitoring/`.
  - [x] Implement `readFocusedElement()` to return the AXUIElement and its text value.
  - [x] Implement `boundsForRange(_:in:)` to retrieve screen coordinates for a text range.
  - [x] Handle failures (nil returns) for unsupported attributes.
- [x] **Task 2: Focus Tracker Implementation (AC: 4, 5)**
  - [x] Create `FocusTracker.swift` in `Spell-i/TextMonitoring/`.
  - [x] Use `NSWorkspace.shared.notificationCenter` to observe `didActivateApplicationNotification`.
  - [x] Implement delegate callback for focus changes.
- [x] **Task 3: Coordinator Integration (AC: 1, 5)**
  - [x] Integrate `AccessibilityReader` and `FocusTracker` into `TextMonitorCoordinator`.
  - [x] Implement logic to clear lints on focus change.
- [x] **Task 4: Coordinate Translation Foundation (AC: 2)**
  - [x] Create `OverlayPositionCalculator.swift` in `Spell-i/Overlay/` (basic skeleton).
  - [x] Implement AX-to-Screen coordinate translation (Y-flip logic).

## Dev Notes

- **Architecture:** `AccessibilityReader` operations MUST happen on the main thread as required by the AX API. [Source: _bmad-output/planning-artifacts/architecture.md#Technical Constraints & Dependencies]
- **Coordination:** `FocusTracker` should be used to ensure underlines are display-accurate and cleared when the user leaves the text field context.
- **Error Handling:** Use guard-and-return for all AX calls to maintain silent degradation.

### Project Structure Notes

- `Spell-i/TextMonitoring/AccessibilityReader.swift`
- `Spell-i/TextMonitoring/FocusTracker.swift`
- `Spell-i/Overlay/OverlayPositionCalculator.swift`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/prd.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash

### Debug Log References

- `AccessibilityReader` successfully copies `kAXValueAttribute` and `kAXSelectedTextRangeAttribute`.
- `FocusTracker` correctly detects `didActivateApplicationNotification`.
- `OverlayPositionCalculator` Y-flip verified for primary screen.

### Completion Notes List

- `AccessibilityReader` implemented with support for text extraction and character bounds.
- `FocusTracker` implemented to notify coordinator of application switches.
- `TextMonitorCoordinator` updated to clear results on focus changes.
- Robust coordinate translation logic added to `OverlayPositionCalculator`.
- **Code Review Fixes:**
  - Fixed multi-monitor coordinate translation by correctly accounting for global primary-screen Y-flip and local screen offsets.
  - Implemented focus element change detection within applications using `AXObserver`.
  - Added a privacy blacklist to prevent text reading from Terminals and Password Managers.

### Code Review Fixes (2026-02-15)

- **[H6-HIGH]** Fixed AXObserver resource leak in `FocusTracker` — added `teardownAXObserver()` to clean up previous observer before creating new one on app switch, and in `stopTracking()`.
- **[M2-MEDIUM]** Replaced force downcasts (`as!`) with safe `as?` + guard in `AccessibilityReader` (3 instances).

### File List

- `Spell-i/TextMonitoring/AccessibilityReader.swift`
- `Spell-i/TextMonitoring/FocusTracker.swift`
- `Spell-i/Overlay/OverlayPositionCalculator.swift`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`

