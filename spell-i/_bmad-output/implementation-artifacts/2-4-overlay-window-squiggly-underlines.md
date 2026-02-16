# Story 2.4: Overlay Window & Squiggly Underlines

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a user,
I want to see red and blue squiggly underlines beneath my errors right where they appear in any app,
so that I can spot mistakes without leaving my current workflow.

## Acceptance Criteria

1. **Visual Feedback:** Red squiggly underlines (`NSColor.systemRed`) appear beneath spelling errors; blue (`NSColor.systemBlue`) appear beneath grammar issues. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.4]
2. **Underline Styling:** Underlines use 3px wave amplitude, 6px period, 1.5px stroke width. [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Squiggly underline rendering]
3. **Accurate Positioning:** Underlines are positioned accurately at the screen coordinates of the erroneous text using `OverlayPositionCalculator` (Y-flip logic). [Source: _bmad-output/planning-artifacts/epics.md#Story 2.4]
4. **Interaction Transparency:** All clicks pass through the overlay to the host app by default (`ignoresMouseEvents = true`), except on underline hit-test regions. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.4]
5. **Dynamic Updates:** Underlines clear instantly on app switch and update after typing pauses to reflect new/removed errors. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.4]
6. **Persistence:** The full-screen transparent overlay persists for the app's lifetime. [Source: _bmad-output/planning-artifacts/epics.md#Story 2.4]

## Tasks / Subtasks

- [x] **Task 1: Overlay Window Implementation (AC: 4, 6)**
  - [x] Create `OverlayWindowController.swift` in `Spell-i/Overlay/`.
  - [x] Initialize `NSWindow` with `.borderless` style, `.floating` level, and `.clear` background.
  - [x] Configure `ignoresMouseEvents = true` and `collectionBehavior` for full-screen compatibility.
- [x] **Task 2: Squiggly Underline View (AC: 1, 2)**
  - [x] Create `SquigglyUnderlineView.swift` in `Spell-i/Overlay/` (or `SquigglyRenderer.swift`).
  - [x] Implement Core Graphics drawing logic for sine-wave squiggles.
  - [x] Implement `updateUnderlines(_:)` to redraw when results change.
- [x] **Task 3: Coordinate Translation (AC: 3)**
  - [x] Complete `OverlayPositionCalculator.swift`.
  - [x] Implement robust screen-frame relative coordinate translation.
- [x] **Task 4: Click Interception Foundation (AC: 4)**
  - [x] Override `hitTest(_:)` in the overlay content view.
  - [x] Return `self` only if the point is within a padding region of an underline.

## Dev Notes

- **UX:** Precision is everything. If underlines are even 2px off, the experience feels "broken". [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Key Design Challenges]
- **Performance:** `NSView.draw(_:)` should be optimized. Redraw only the necessary rects if possible, or clear and redraw all if simple enough.
- **Coordination:** The `TextMonitorCoordinator` should trigger updates on the main thread.

### Project Structure Notes

- `Spell-i/Overlay/OverlayWindowController.swift`
- `Spell-i/Overlay/SquigglyUnderlineView.swift`
- `Spell-i/Overlay/OverlayPositionCalculator.swift`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/prd.md]
- [Source: _bmad-output/planning-artifacts/ux-design-specification.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash

### Debug Log References

- `OverlayWindowController` correctly initializes with `.floating` and `.clear` background.
- `SquigglyUnderlineView` sine-wave drawing verified via manual inspection.
- `hitTest` override in `OverlayContentView` successfully intercepts clicks on underlines.

### Completion Notes List

- Full-screen transparent overlay window implemented.
- Custom Core Graphics drawing for red/blue squiggly underlines.
- Pixel-precise hit-testing allows clicking underlines while passing other events to host apps.
- Dynamic underline updates wired to lint results.
- **Code Review Fixes:**
  - Implemented dynamic multi-monitor support by having the overlay window follow the active screen containing the focused text element.
  - Refined sine-wave rendering logic to produce smooth, non-jagged curves.
  - Added a 2px vertical offset to underlines to prevent visual collision with character descenders.
  - Corrected hit-test logic to use central `Constants` for interaction padding.

### Code Review Fixes (2026-02-15)

- **[H2-HIGH]** Fixed `AppDelegate.swift` color logic — `errorType == "spelling"` never matched Harper's `"Spelling"` output. Changed to case-insensitive `lowercased().contains("spell")` check so spelling errors correctly render as red.

### File List

- `Spell-i/Overlay/OverlayWindowController.swift`
- `Spell-i/Overlay/SquigglyUnderlineView.swift`
- `Spell-i/Overlay/OverlayContentView.swift`
- `Spell-i/Overlay/OverlayPositionCalculator.swift`

