# Story 1.3: Accessibility Permission & Onboarding

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a user,
I want to understand why Spell-i needs Accessibility permission and grant it easily,
so that I can start using the app quickly and trust that my data stays private.

## Acceptance Criteria

1. **Onboarding Trigger:** Given the user launches Spell-i for the first time without Accessibility permission, when the app starts, then an onboarding window appears (~400x250px, non-resizable, centered). [Source: _bmad-output/planning-artifacts/epics.md#Story 1.3]
2. **Onboarding Content:** The window explains that Accessibility permission is needed and states "No data leaves your Mac — everything is checked offline". [Source: _bmad-output/planning-artifacts/epics.md#Story 1.3]
3. **CTA Button:** A single "Open System Settings" button is displayed with a dark green accent color. [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Onboarding window]
4. **System Integration:** When the user clicks "Open System Settings", then System Settings opens to the Accessibility pane. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.3]
5. **Auto-Dismissal:** When the user toggles Spell-i on in System Settings, the onboarding window detects the grant and dismisses automatically. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.3]
6. **Persistence:** If permission is not granted, the onboarding window reappears on next launch. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.3]
7. **Direct Active State:** Given Accessibility permission has been previously granted, when the user launches Spell-i, then no onboarding window appears and the app proceeds directly to active state. [Source: _bmad-output/planning-artifacts/epics.md#Story 1.3]

## Tasks / Subtasks

- [x] **Task 1: Accessibility Permission Checker (AC: 1, 5, 7)**
  - [x] Create `AccessibilityPermissionChecker.swift` in `Spell-i/Permissions/`.
  - [x] Implement `isAccessibilityEnabled()` using `AXIsProcessTrusted()`.
  - [x] Implement a polling mechanism or use a timer to detect permission changes while the app is running.
- [x] **Task 2: Onboarding Window Controller (AC: 1, 2, 3)**
  - [x] Create `OnboardingWindowController.swift` in `Spell-i/Permissions/`.
  - [x] Design the window in code: ~400x250px, `.titled`, `.closable`, `.fullSizeContentView`.
  - [x] Add labels for explanation and privacy promise.
  - [x] Add "Open System Settings" button with dark green background/accent.
- [x] **Task 3: System Settings Integration (AC: 4)**
  - [x] Implement action to open `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` using `NSWorkspace.shared.open()`.
- [x] **Task 4: App Delegate Integration (AC: 1, 5, 6, 7)**
  - [x] Update `AppDelegate.applicationDidFinishLaunching` to check permission.
  - [x] If not granted, instantiate and show `OnboardingWindowController`.
  - [x] Implement callback from onboarding to re-check permission and transition to `setupApp()`.
- [x] **Task 5: Visual Polish (AC: 1, 3)**
  - [x] Ensure the onboarding window is centered on the screen.
  - [x] Implement non-resizable constraint for the onboarding window.

## Dev Notes

- **Architecture:** Use a clean delegate or completion block pattern for `OnboardingWindowController` to notify `AppDelegate` of completion.
- **Privacy:** Ensure the "checked offline" message is prominent to build user trust. [Source: _bmad-output/planning-artifacts/prd.md#Privacy]
- **UX:** Zero animations for state transitions as per UX spec. [Source: _bmad-output/planning-artifacts/ux-design-specification.md#Key Design Decisions]

### Project Structure Notes

- `Spell-i/Permissions/AccessibilityPermissionChecker.swift`
- `Spell-i/Permissions/OnboardingWindowController.swift`
- `Spell-i/App/AppDelegate.swift`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/prd.md]
- [Source: _bmad-output/planning-artifacts/ux-design-specification.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash

### Debug Log References

- Verified `AccessibilityPermissionChecker` logic with `AXIsProcessTrusted`.
- `OnboardingWindowController` polling timer correctly detects permission grant.
- AppDelegate integration confirmed to show onboarding only when necessary.

### Completion Notes List

- Accessibility permission flow fully implemented.
- Onboarding window created with compliant styling (400x250, dark green button).
- Auto-dismissal and polling logic implemented for seamless user experience.
- AppDelegate correctly wires permission detection to app setup.
- **Code Review Fixes:**
  - Added missing privacy promise text: "No data leaves your Mac — everything is checked offline."
  - Improved permission detection efficiency by adding `didBecomeActive` notification monitoring and reducing polling frequency.
  - Set onboarding window to `.floating` level to ensure it stays visible during the permission granting process.
  - Fixed potential UI clipping in the title label by using a wrapping label.

### File List

- `Spell-i/Permissions/AccessibilityPermissionChecker.swift`
- `Spell-i/Permissions/OnboardingWindowController.swift`
- `Spell-i/App/AppDelegate.swift`

