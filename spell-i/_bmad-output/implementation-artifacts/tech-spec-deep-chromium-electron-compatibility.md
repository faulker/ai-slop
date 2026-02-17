---
title: 'Deep Chromium/Electron App Compatibility'
slug: 'deep-chromium-electron-compatibility'
created: '2026-02-16'
status: 'ready-for-dev'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [Swift, Rust, macOS Accessibility API, harper-core, swift-bridge FFI]
files_to_modify:
  - 'Spell-i/Utilities/Constants.swift'
  - 'Spell-i/TextMonitoring/AccessibilityReader.swift'
  - 'Spell-i/TextMonitoring/TextMonitorCoordinator.swift'
  - 'Spell-iTests/TextMonitoring/AccessibilityReaderTests.swift (new)'
code_patterns:
  - 'Constants enum for all magic numbers and configurable values'
  - 'Logger per component (Logger(category:))'
  - 'Silent degradation on AX failures (log + skip, never crash)'
  - 'Two-pass search: prioritize known roles, then recurse into containers'
  - 'Main thread for AX calls, engineQueue for background lint'
  - 'Generation counter to prevent stale results overwriting fresh'
test_patterns:
  - 'XCTest framework with @testable import Spell_i'
  - 'Mock delegates for protocol testing'
  - '#if DEBUG test helpers for injecting state'
  - 'Spell-iTests/ mirrors source directory structure'
---

# Tech-Spec: Deep Chromium/Electron App Compatibility

**Created:** 2026-02-16

## Overview

### Problem Statement

Spell-i's AX text detection and underline rendering has significant gaps with Chromium-based browsers (Chrome), Electron apps (Slack), and non-standard editors (Zed IDE, GitTower). The current deep element traversal (`deepFocusedElement` / `findTextElementInChildren`) works for basic cases but fails when:
- Web-based text components don't implement `kAXBoundsForRangeParameterizedAttribute`
- AX hierarchies exceed the hardcoded depth limit of 8
- Non-standard AX roles are used by custom web frameworks or native editors
- Text is detected but underlines can't be positioned (silent failure)

### Solution

Improve deep element traversal strategies, add alternative bounds query fallbacks, make traversal depth configurable, and enhance role detection for web-based text areas. Best-effort approach — improve detection hit rate without guaranteeing 100% coverage across all web editors.

### Scope

**In Scope:**
- Improved deep traversal for Chrome, Slack, Zed IDE, GitTower
- Alternative bounds query strategies when `kAXBoundsForRangeParameterizedAttribute` fails
- Configurable max traversal depth (default bump + user setting)
- Enhanced AX role detection for web-based text components
- Testing/validation against target apps

**Out of Scope:**
- Window move/resize tracking fixes (scroll-to-clear is acceptable)
- Cross-origin iframe traversal
- Google Docs / complex collaborative editor support
- Polling-based window position fallback

## Context for Development

### Codebase Patterns

- **Architecture:** Clean separation — `AccessibilityReader` for AX queries, `TextMonitorCoordinator` for pipeline orchestration, `FocusTracker` for app/element change detection
- **Deep traversal:** Two strategies exist in `AccessibilityReader.swift`: (1) follow nested `kAXFocusedUIElementAttribute` down the tree, (2) breadth search children for text-bearing elements
- **Role filter:** Hardcoded `Set<String>` of 4 roles (`AXTextArea`, `AXTextField`, `AXComboBox`, `AXSearchField`) — only these are accepted as text elements
- **Container roles for recursion:** Only `AXWebArea`, `AXGroup`, `AXScrollArea` — missing types like `AXList`, `AXCell`, `AXSection`
- **Max depth:** Hardcoded `8` in `AccessibilityReader.maxTraversalDepth`
- **Bounds failures:** Silently skipped with logging in `TextMonitorCoordinator.performLint()` — no fallback attempted
- **Text replacement:** 3-strategy fallback in `TextReplacer` (AX range → full-text replace → clipboard paste) — already robust for web apps
- **Constants pattern:** `Constants` enum holds all magic numbers, timing intervals, and configurable values
- **Threading:** AX calls on main thread only; linting on `engineQueue` (background, `.userInitiated`)
- **No preferences system:** App has no user settings storage beyond the user dictionary file

### Files to Reference

| File | Purpose |
| ---- | ------- |
| `Spell-i/TextMonitoring/AccessibilityReader.swift` (225 lines) | Deep traversal, text extraction, role filtering, bounds queries |
| `Spell-i/TextMonitoring/TextMonitorCoordinator.swift` (488 lines) | Pipeline orchestration, bounds failure handling, lint dispatch |
| `Spell-i/TextMonitoring/FocusTracker.swift` (159 lines) | App/focus change detection via AX observers |
| `Spell-i/Utilities/Constants.swift` (62 lines) | App-wide constants and configurable values |
| `Spell-i/Overlay/OverlayPositionCalculator.swift` (54 lines) | AX rect → screen coordinate conversion |
| `Spell-i/Overlay/TextReplacer.swift` (190 lines) | 3-strategy text replacement (already web-app robust) |
| `Spell-iTests/TextMonitoring/FocusTrackerTests.swift` (84 lines) | Existing test pattern reference |

### Technical Decisions

- **Best-effort bounds:** Try alternative strategies (element-level bounds as coarse fallback) but accept silent degradation for unsupported apps
- **Configurable depth:** Expose via `Constants.swift` with a default bump to 12; no full preferences UI needed
- **Keep scroll-to-clear** as the window-move workaround for Electron apps
- **Expanded role set:** Add web-specific roles to `textEditRoles` and container recursion set based on investigation of target apps
- **No new dependencies:** All improvements within existing Swift AX layer

### Key Technical Anchors

- `AccessibilityReader.swift` line 27 — `maxTraversalDepth = 8` (change to configurable constant)
- `AccessibilityReader.swift` lines 144-146 — `textEditRoles` set (expand with web roles)
- `AccessibilityReader.swift` lines 129-135 — Container role filter in `findTextElementInChildren` (expand)
- `AccessibilityReader.swift` lines 84-105 — `deepFocusedElement` (enhance with subrole checking)
- `TextMonitorCoordinator.swift` lines 437-440 — Bounds failure path (add fallback strategy)

## Implementation Plan

### Tasks

- [ ] **Task 1: Add traversal and role constants to `Constants.swift`**
  - File: `Spell-i/Utilities/Constants.swift`
  - Action: Add to the `Constants` enum:
    - `static let maxTraversalDepth = 12` — bumped from 8, centralized here
    - `static let textEditRoles: Set<String>` — moved from `AccessibilityReader`, expanded with: `"AXTextArea"`, `"AXTextField"`, `"AXComboBox"`, `"AXSearchField"`, `"AXStaticText"` (Chrome read-only text that's sometimes editable via contenteditable)
    - `static let containerRolesForTraversal: Set<String>` — moved from inline check in `findTextElementInChildren`, expanded with: `"AXWebArea"`, `"AXGroup"`, `"AXScrollArea"`, `"AXList"`, `"AXCell"`, `"AXSection"`, `"AXLayoutArea"`, `"AXSplitGroup"`, `"AXTabGroup"`
  - Notes: Centralizing these makes them testable and configurable. `AXStaticText` requires an additional `isEditable` check in `textContext()` to avoid matching labels/headings — handled in Task 2.

- [ ] **Task 2: Enhance `AccessibilityReader` traversal and role detection**
  - File: `Spell-i/TextMonitoring/AccessibilityReader.swift`
  - Action — multiple changes:
    1. **Replace hardcoded depth/roles with Constants:**
       - Replace `private let maxTraversalDepth = 8` (line 27) with `Constants.maxTraversalDepth`
       - Replace `private static let textEditRoles: Set<String>` (lines 144-146) with `Constants.textEditRoles`
       - Replace inline role checks in `findTextElementInChildren` (line 131) with `Constants.containerRolesForTraversal.contains(role)`
    2. **Add `isEditable` guard for `AXStaticText`:**
       - In `textContext(from:)` method, after the role check passes, add a secondary check: if role is `AXStaticText`, verify the element has `kAXEditableAttribute` or a non-nil `kAXValueAttribute` that is writable. This prevents matching labels/headings. Check writability by attempting `AXUIElementIsAttributeSettable(element, kAXValueAttribute, &settable)`.
    3. **Add subrole awareness to `deepFocusedElement`:**
       - After checking `hasTextValue(childElement)` (line 99), also check if the child has an `AXSubrole` of `AXContentList` or `AXTextArea` — Chromium apps sometimes use subroles instead of standard roles.
       - Add a helper: `private func axSubrole(of element: AXUIElement) -> String?` following the same pattern as `axRole(of:)`.
    4. **Enhance `findTextElementInChildren` first pass:**
       - The current first pass only checks `AXTextArea` and `AXTextField` (lines 119-126). Expand to check all `Constants.textEditRoles` so that `AXComboBox`, `AXSearchField`, and editable `AXStaticText` are also found in the first pass.
    5. **Add `boundsForElement` public method:**
       - New method `func boundsForElement(_ element: AXUIElement) -> CGRect?` that reads `kAXPositionAttribute` and `kAXSizeAttribute` directly from the element. This returns the element's full bounding box (not per-character). Used as a coarse fallback when `boundsForRange` fails.
       - Implementation: Read `kAXPositionAttribute` → `CGPoint`, read `kAXSizeAttribute` → `CGSize`, combine into `CGRect`.
  - Notes: All changes preserve the existing silent-degradation pattern. The `isEditable` check uses `AXUIElementIsAttributeSettable` which is a standard AX call, safe on main thread.

- [ ] **Task 3: Add element-level bounds fallback in `TextMonitorCoordinator`**
  - File: `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`
  - Action: In the `performLint()` method, modify the bounds failure path (lines 437-440):
    - Current: When `boundsForRange` returns nil, increment `boundsFailures` and `continue` (skip the underline entirely).
    - New: When `boundsForRange` returns nil, try `accessibilityReader.boundsForElement(element)` as a fallback. If the element-level bounds succeed, use them but **only for the first bounds failure per lint pass** — drawing all underlines at the element's full bounds would overlap. For subsequent failures in the same pass, still skip.
    - Log when using the element-level fallback: `"performLint: using element-level bounds fallback for '\(raw.originalWord)'"`.
    - Add a `var usedElementFallback = false` flag before the loop, set to `true` after first use.
  - Notes: This gives at least one visible underline in apps where per-character bounds don't work, signaling to the user that Spell-i detected errors even if it can't precisely position every underline. The element-level rect will be imprecise (covers the whole text field) but is better than nothing.

- [ ] **Task 4: Add `AccessibilityReader` unit tests**
  - File: `Spell-iTests/TextMonitoring/AccessibilityReaderTests.swift` (new file)
  - Action: Create test file with:
    1. `testTextEditRolesContainsExpectedRoles()` — verify `Constants.textEditRoles` contains all 5 expected roles
    2. `testContainerRolesContainsExpectedRoles()` — verify `Constants.containerRolesForTraversal` contains all 9 expected container roles
    3. `testMaxTraversalDepthIsAtLeast12()` — verify `Constants.maxTraversalDepth >= 12`
    4. `testTextEditRolesDoesNotContainNonTextRoles()` — verify roles like `"AXButton"`, `"AXImage"`, `"AXToolbar"` are NOT in the set
    5. `testContainerRolesDoesNotContainLeafRoles()` — verify roles like `"AXTextField"`, `"AXButton"` are NOT in the container set
  - Notes: These tests validate configuration correctness without requiring live AX permissions. Follow existing pattern from `FocusTrackerTests.swift` — `@testable import Spell_i`, XCTest assertions.

### Acceptance Criteria

- [ ] **AC 1:** Given a Chromium-based app (Chrome, Slack) with a focused text input, when `readFocusedElement()` is called, then it successfully returns a `TextContext` with the text content from the nested AX element, traversing through `AXWebArea` → `AXGroup` → text element.

- [ ] **AC 2:** Given an AX hierarchy deeper than 8 levels (e.g., deeply nested Slack message compose), when `deepFocusedElement` or `findTextElementInChildren` traverses the tree, then it searches up to `Constants.maxTraversalDepth` (12) levels deep instead of stopping at 8.

- [ ] **AC 3:** Given a web-based text input with role `AXStaticText` and `contenteditable` (common in Chrome), when `textContext(from:)` evaluates the element, then it accepts the element only if `kAXValueAttribute` is settable, preventing false matches on non-editable labels.

- [ ] **AC 4:** Given a text element where `kAXBoundsForRangeParameterizedAttribute` fails (returns nil), when the coordinator builds underline display items, then it falls back to element-level bounds (`kAXPositionAttribute` + `kAXSizeAttribute`) for the first error in the lint pass, and logs the fallback usage.

- [ ] **AC 5:** Given the element-level bounds fallback is used for the first error, when subsequent errors in the same lint pass also fail `boundsForRange`, then they are skipped (not drawn with element-level bounds) to prevent overlapping underlines.

- [ ] **AC 6:** Given the expanded `containerRolesForTraversal` set, when `findTextElementInChildren` encounters an `AXList`, `AXCell`, `AXSection`, `AXLayoutArea`, `AXSplitGroup`, or `AXTabGroup` container, then it recurses into that container's children to search for text elements.

- [ ] **AC 7:** Given the `Constants.maxTraversalDepth` value, when unit tests run, then they verify the depth is at least 12 and that role sets contain all expected roles without non-text roles.

- [ ] **AC 8:** Given any of the above changes, when the app processes a standard native macOS text field (e.g., Spotlight, Notes, TextEdit), then existing functionality is not regressed — text detection, underlines, and corrections work identically to before.

## Additional Context

### Dependencies

- No new external dependencies required
- All improvements are within existing Swift AX layer
- Task 2 depends on Task 1 (Constants must exist first)
- Task 3 depends on Task 2 (needs `boundsForElement` method)
- Task 4 depends on Tasks 1-3 (tests validate all changes)

### Testing Strategy

**Unit Tests (automated, no AX permissions needed):**
- Role set membership tests (Task 4)
- Depth constant validation (Task 4)
- Negative tests ensuring non-text roles are excluded (Task 4)

**Manual Validation (requires running apps + AX permissions):**
- Open Chrome, navigate to a page with a text input (e.g., Google search bar), type misspelled text, verify underlines appear
- Open Slack, type in a message compose field, verify text is detected and underlines render
- Open Zed IDE, type in a source file, verify text detection works
- Open GitTower, type in a commit message or search field, verify text detection works
- Verify existing native apps (TextEdit, Notes, Spotlight) still work correctly (regression check)
- Test the element-level bounds fallback by observing the debug log for "using element-level bounds fallback" messages in Chrome/Slack

### Notes

**Risk: `AXStaticText` false positives**
Adding `AXStaticText` to `textEditRoles` risks matching non-editable text (labels, headings). The `isEditable` guard (Task 2, item 2) mitigates this, but some apps may report `AXStaticText` as settable even when it's not truly a user input. If this causes noise, the fallback is to remove `AXStaticText` from the role set and rely on the existing `AXTextField`/`AXTextArea` matching.

**Risk: Element-level bounds imprecision**
The element-level bounds fallback (Task 3) draws a single underline covering the entire text field, not under a specific word. This is deliberately coarse — it signals "errors detected" rather than "error here." If users find this confusing, it can be gated behind a debug flag.

**Future considerations (out of scope):**
- Per-app configuration (custom depth, role overrides per bundle ID)
- AX tree caching to avoid repeated deep traversals on each lint pass
- `AXRangeForPositionParameterizedAttribute` as an alternative bounds strategy
- Integration with Accessibility Inspector for debugging AX hierarchies
