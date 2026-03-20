# Story 4.1: History Tab & Per-Profile Run History

Status: ready-for-dev

## Story

As a user,
I want to view the run history for each profile,
so that I can audit past sync operations and quickly identify failures.

## Acceptance Criteria

1. **Given** the user navigates to the History tab
   **When** the tab loads
   **Then** a profile dropdown is displayed at the top with all profiles listed (FR54)
   **And** each profile name in the dropdown has a status indicator (green/red/yellow) next to it (FR55)

2. **Given** the user selects a profile from the dropdown
   **When** the profile's history loads
   **Then** all historical runs are displayed sorted by most recent first (FR38)
   **And** each run shows: date/time, status (successful/failed/canceled/interrupted), and duration (FR39)

3. **Given** the user switches to a different profile via the dropdown
   **When** the selection changes
   **Then** the run history updates to show the newly selected profile's runs (FR56)

4. **Given** a profile has no run history
   **When** it is selected in the dropdown
   **Then** an empty state message is displayed explaining no runs have been executed yet

## Tasks / Subtasks

- [ ] Task 1: Implement HistoryTabView (AC: #1, #2, #3, #4)
  - [ ] 1.1: Update `Views/MainWindow/History/HistoryTabView.swift` — replace placeholder
  - [ ] 1.2: Profile dropdown (Picker) at top with all profiles from ProfileStore
  - [ ] 1.3: Each profile in dropdown shows StatusBadge + name (FR55)
  - [ ] 1.4: State: `@State private var selectedProfileId: UUID?`
  - [ ] 1.5: Filter LogStore entries by selected profile ID, sort by most recent first
  - [ ] 1.6: Empty state for no-history profiles
- [ ] Task 2: Create HistoryRunRow (AC: #2)
  - [ ] 2.1: Create `Views/MainWindow/History/HistoryRunRow.swift`
  - [ ] 2.2: Display: StatusBadge + formatted date/time + duration + status text
  - [ ] 2.3: Date format: "Feb 27, 2026 at 2:30 PM" using `DateFormatter` or `.formatted()`
  - [ ] 2.4: Duration format: "2m 34s" or "1h 5m 12s"
  - [ ] 2.5: Make tappable (selection for log viewer in Story 4.2)
- [ ] Task 3: Add helper methods to LogStore (AC: #2)
  - [ ] 3.1: `func entries(for profileId: UUID) -> [LogEntry]` — filtered and sorted
  - [ ] 3.2: `func lastStatus(for profileId: UUID) -> JobStatus?` — most recent entry status
- [ ] Task 4: Wire dropdown status indicators (AC: #1)
  - [ ] 4.1: For each profile in dropdown, determine current status:
    - Running job → `.running`
    - Last log entry success → `.success`
    - Last log entry failed → `.failed`
    - No history → `.idle`
  - [ ] 4.2: Use StatusBadge in dropdown labels
- [ ] Task 5: Write tests
  - [ ] 5.1: Test LogStore.entries(for:) filters correctly
  - [ ] 5.2: Test LogStore.lastStatus returns most recent entry's status
  - [ ] 5.3: Test empty state displays when no entries exist

## Dev Notes

### Architecture Compliance

**Layer:** Views (MainWindow/History) + Stores (LogStore enhancement).

**File locations:**
```
Cirrus/Cirrus/Views/MainWindow/History/
├── HistoryTabView.swift                 # MODIFY — replace placeholder
└── HistoryRunRow.swift                  # NEW
Cirrus/Cirrus/Stores/
└── LogStore.swift                       # MODIFY — add helper methods
CirrusTests/Stores/
└── LogStoreTests.swift                  # MODIFY — add filter/sort tests
```

### Technical Requirements

**Profile dropdown with status badges:**
```swift
Picker("Profile", selection: $selectedProfileId) {
    ForEach(profileStore.profiles) { profile in
        HStack {
            StatusBadge(status: currentStatus(for: profile))
            Text(profile.name)
        }
        .tag(profile.id as UUID?)
    }
}
```

**Status determination for dropdown:**
1. Check `jobManager.isRunning(for: profileId)` → `.running`
2. Check `logStore.lastStatus(for: profileId)` → use that status
3. No entries → `.idle`

**Date formatting:**
```swift
logEntry.startedAt.formatted(date: .abbreviated, time: .shortened)
// → "Feb 27, 2026, 2:30 PM"
```

**Duration formatting:**
```swift
func formatDuration(_ seconds: Double) -> String {
    let hours = Int(seconds) / 3600
    let minutes = (Int(seconds) % 3600) / 60
    let secs = Int(seconds) % 60
    if hours > 0 { return "\(hours)h \(minutes)m \(secs)s" }
    if minutes > 0 { return "\(minutes)m \(secs)s" }
    return "\(secs)s"
}
```

### Enforcement Rules

- Reuse `StatusBadge` component — do NOT create separate status indicators
- Filter/sort logic lives in LogStore — NOT in the view
- Views access stores via `@Environment`

### Dependencies

- **Depends on:** Story 3.1 (LogStore, JobManager), Story 2.2 (ProfileStore), Story 1.3 (History tab placeholder)
- **Does NOT depend on:** Story 4.2 or Epic 5

### References

- [Source: architecture.md#View Patterns] — @Environment, pure views
- [Source: architecture.md#Data Architecture] — LogEntry structure
- [Source: epics.md#Story 4.1] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
