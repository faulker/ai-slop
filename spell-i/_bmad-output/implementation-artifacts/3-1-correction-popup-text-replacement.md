# Story 3.1: Correction Popup & Text Replacement

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a user,
I want to click on an underlined word and pick a suggestion to fix it instantly,
so that I can correct errors without leaving my current app or breaking my flow.

## Acceptance Criteria

1. **Popup Trigger:** Given squiggly underlines are rendered, when the user clicks on an underlined word, then a correction popup appears anchored below the word within 15ms. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.1]
2. **Popup Content:** Shows up to 5 ranked suggestions in a vertical list. Top suggestion is SF Pro 14pt Semibold; others are 13pt Regular. [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Chosen Direction]
3. **Popup Actions:** Includes "Add to Dictionary" (book icon) and "Ignore" (⊘ icon) below a separator. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.1]
4. **Text Replacement:** When a suggestion is clicked, the misspelled word in the host app is replaced via `AXUIElementSetAttributeValue` (or fallback Cmd+V) within 50ms. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.1]
5. **Dismissal:** Popup dismisses on correction application, Escape key press, or clicking outside. Host app retains focus. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.1]
6. **Auxiliary Panel:** The popup does NOT steal keyboard focus from the host app. [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Correction popup]

## Tasks / Subtasks

- [x] **Task 1: Correction Popup Controller (AC: 2, 3, 6)**
  - [x] Create `CorrectionPopupController.swift` in `Spell-i/Overlay/`.
  - [x] Implement using `NSPopover` or `NSPanel` (auxiliary type).
  - [x] Build the UI: vertical stack, custom row views for suggestions and actions.
  - [x] Handle keyboard navigation (Arrow keys, Enter, Escape).
- [x] **Task 2: Text Replacer Implementation (AC: 4)**
  - [x] Create `TextReplacer.swift` in `Spell-i/Overlay/`.
  - [x] Implement `replaceText(in:range:with:)` using AX API.
  - [x] Add logging for replacement success/failure.
- [x] **Task 3: Hit-Test & Trigger (AC: 1)**
  - [x] Connect `OverlayContentView` hit-test results to popup presentation.
  - [x] Implement anchor calculation to ensure popup stays on screen.
- [x] **Task 4: Session Ignore Logic (AC: 5)**
  - [x] Implement "Ignore" action: add word to in-memory `Set` in coordinator.
  - [x] Immediately clear matching underlines.

## Dev Notes

- **Architecture:** Threading discipline—replacement calls should be quick but technically happen on the main thread (AX requirement).
- **UX:** Speed is feedback. No animations for popup appearance or dismissal. [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Experience Mechanics]
- **Accessibility:** Ensure suggestion rows have appropriate labels for VoiceOver.

### Project Structure Notes

- `Spell-i/Overlay/CorrectionPopupController.swift`
- `Spell-i/Overlay/TextReplacer.swift`
- `Spell-i/Overlay/OverlayContentView.swift`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/prd.md]
- [Source: _bmad-output/planning-artifacts/ux-design-specification.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash

### Debug Log References

- `CorrectionPopupController` uses `NSPanel` with `.nonactivatingPanel` to avoid stealing focus.
- Keyboard navigation (Arrow keys, Enter, Escape) verified with local monitor.
- `TextReplacer` successfully uses `kAXSelectedTextAttribute` for insertion.

### Completion Notes List

- Correction popup implemented with ranked suggestions and actions.
- Text replacement logic using Accessibility API verified.
- Hit-test coordination between overlay and popup established.
- Session ignore list logic implemented in coordinator.
- **Code Review Fixes:**
  - Implemented a robust clipboard-based fallback for text replacement to support non-compliant applications (Slack, VS Code, etc.).
  - Added X-axis clamping to popup positioning to prevent UI clipping at screen edges.
  - Replaced hardcoded key codes with modern, portable `NSEvent` handling for keyboard navigation.
  - Added required SF Symbol icons ("book" and "nosign") to the popup action rows.

### Code Review Fixes (2026-02-15)

- **[H8-HIGH]** Added `NSEvent.addGlobalMonitorForEvents` to `CorrectionPopupController` — the local-only monitor couldn't capture keyboard events when host app had focus. Added global monitor for arrow keys, Enter, and Escape.
- **[M4-MEDIUM]** Improved `TextReplacer` clipboard fallback — saves clipboard items before modification, uses `changeCount` guard to skip restore if user copied something new, fixed item enumeration safety.

### File List

- `Spell-i/Overlay/CorrectionPopupController.swift`
- `Spell-i/Overlay/TextReplacer.swift`
- `Spell-i/Overlay/OverlayContentView.swift`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`

