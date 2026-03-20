# Story 3.2: Tray Popup Dashboard

Status: ready-for-dev

## Story

As a user,
I want a tray popup that shows all my profiles with status and lets me start/cancel jobs,
so that I can manage syncs with two clicks without opening the full app.

## Acceptance Criteria

1. **Given** profiles exist and the user clicks the menu bar icon
   **When** the tray popup opens
   **Then** all profiles are displayed with status badges (green checkmark/red xmark/yellow clock) (FR45)
   **And** idle profiles show the last successful run date/time (FR46)
   **And** running profiles show elapsed run time (FR47)
   **And** the popup opens within 200ms (NFR1)

2. **Given** a profile is idle
   **When** the user views its row in the popup
   **Then** a Start button is available (FR48)
   **And** a History link is available that opens the history tab in the main GUI (FR50)

3. **Given** a profile has a running job
   **When** the user views its row in the popup
   **Then** a Cancel button replaces the Start button (FR49)
   **And** the elapsed time updates in real-time

4. **Given** the user clicks Start on a profile
   **When** the network is unavailable
   **Then** the job is prevented from starting with a clear message (FR27)

5. **Given** the user clicks Start on a profile
   **When** the network is available
   **Then** the job starts via JobManager (FR20)
   **And** the status badge transitions to running state

6. **Given** the user clicks Cancel on a running profile
   **When** the cancel is confirmed
   **Then** the job is cancelled via JobManager (FR22)

7. **Given** no profiles exist
   **When** the user opens the tray popup
   **Then** a "Create your first profile" button is displayed (FR51)
   **And** clicking it opens the main GUI to the profile creation form

8. **Given** status badges are displayed
   **When** a user with color vision deficiency views them
   **Then** each badge uses both color AND an SF Symbol shape (checkmark/xmark/clock/circle) (NFR18)

## Tasks / Subtasks

- [ ] Task 1: Create StatusBadge component (AC: #1, #8)
  - [ ] 1.1: Create `Views/Components/StatusBadge.swift`
  - [ ] 1.2: Takes `JobStatus` enum → returns color + SF Symbol:
    - `.idle` → gray + circle
    - `.running` → yellow + clock
    - `.success` → green + checkmark.circle.fill
    - `.failed` → red + xmark.circle.fill
    - `.canceled` → orange + minus.circle
  - [ ] 1.3: Add `.accessibilityLabel` with status description
- [ ] Task 2: Create PopupProfileRow (AC: #1, #2, #3)
  - [ ] 2.1: Create `Views/TrayPopup/PopupProfileRow.swift`
  - [ ] 2.2: Display: StatusBadge + profile name + metadata (last run or elapsed time)
  - [ ] 2.3: Idle state: show last successful run date (from LogStore), Start button, History link
  - [ ] 2.4: Running state: show elapsed time (computed from job start), Cancel button
  - [ ] 2.5: Elapsed time updates via SwiftUI timer: `.onReceive(Timer.publish(every: 1, ...))`
- [ ] Task 3: Create PopupEmptyState (AC: #7)
  - [ ] 3.1: Create `Views/TrayPopup/PopupEmptyState.swift`
  - [ ] 3.2: Display "Create your first profile" CTA button
  - [ ] 3.3: Clicking opens main GUI to profile creation form
- [ ] Task 4: Update TrayPopupView (AC: #1-8)
  - [ ] 4.1: Update `Views/TrayPopup/TrayPopupView.swift` — replace placeholder content
  - [ ] 4.2: If profiles exist: show ScrollView of PopupProfileRow items
  - [ ] 4.3: If no profiles: show PopupEmptyState
  - [ ] 4.4: Access ProfileStore, JobManager, LogStore via @Environment
- [ ] Task 5: Wire Start/Cancel actions (AC: #4, #5, #6)
  - [ ] 5.1: Start: check NetworkMonitor.isConnected → if false, show error message
  - [ ] 5.2: Start: if connected, call `jobManager.startJob(for: profile)`
  - [ ] 5.3: Cancel: show brief confirmation, then call `jobManager.cancelJob(for: profileId)`
- [ ] Task 6: Write tests
  - [ ] 6.1: Test StatusBadge renders correct symbol for each JobStatus case
  - [ ] 6.2: Test StatusBadge has accessibility labels

## Dev Notes

### Architecture Compliance

**Layer:** Views (TrayPopup, Components).

**File locations:**
```
Cirrus/Cirrus/Views/
├── TrayPopup/
│   ├── TrayPopupView.swift              # MODIFY — replace placeholder
│   ├── PopupProfileRow.swift            # NEW
│   └── PopupEmptyState.swift            # NEW
└── Components/
    └── StatusBadge.swift                # NEW
```

### Technical Requirements

**StatusBadge — reusable across popup, GUI profile list, and history dropdown:**
```swift
struct StatusBadge: View {
    let status: JobStatus

    var body: some View {
        Image(systemName: symbolName)
            .foregroundStyle(color)
            .accessibilityLabel(accessibilityText)
    }

    private var symbolName: String {
        switch status {
        case .idle: "circle"
        case .running: "clock.fill"
        case .success: "checkmark.circle.fill"
        case .failed: "xmark.circle.fill"
        case .canceled: "minus.circle"
        }
    }
}
```
- This is a pure view — takes value, returns visual. No state access.
- Uses BOTH color AND shape for color independence (NFR18)

**Elapsed time display:**
```swift
TimelineView(.periodic(schedule: .init(minimumInterval: 1))) { _ in
    Text(elapsedString(from: job.startedAt))
}
```
Or use `.onReceive(Timer.publish(every: 1, on: .main, in: .common).autoconnect())`

**Last successful run date (FR46):**
- Query LogStore for latest entry where `profileId == profile.id && status == .success`
- Display formatted date: "Today, 2:30 PM" or "Feb 27, 2026"
- Display "Never run" if no successful entries exist

**Network check (FR27):**
- Check `networkMonitor.isConnected` before starting job
- If not connected, show inline error: "No network connection. Cannot start sync."
- Do NOT disable the button — show the error on tap

**Popup performance (NFR1):** Panel is pre-created (Story 1.2). Views observe `@Observable` stores with cached state. No loading spinners needed — data is already in memory.

### Enforcement Rules

- `StatusBadge` is a pure view — no @Environment, no state access
- Profile rows are pure views — take Profile + JobStatus + action closures
- Views contain no business logic — Start/Cancel go through JobManager
- Network check goes through NetworkMonitor — don't check connectivity directly

### Dependencies

- **Depends on:** Story 1.2 (TrayPopupView shell), Story 2.2 (ProfileStore), Story 3.1 (JobManager, LogStore, NetworkMonitor)
- **Does NOT depend on:** Stories 3.3 or Epics 4-5

### References

- [Source: architecture.md#View Patterns] — pure views, @Environment, no AnyView
- [Source: architecture.md#Project Structure & Boundaries] — Components directory
- [Source: ux-design-specification.md] — Status badges, popup layout, empty state
- [Source: epics.md#Story 3.2] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
