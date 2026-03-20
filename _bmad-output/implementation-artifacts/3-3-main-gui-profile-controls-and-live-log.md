# Story 3.3: Main GUI Profile Controls & Live Log

Status: ready-for-dev

## Story

As a user,
I want to start and cancel jobs from the main GUI and see live output,
so that I have full control and visibility from the management interface.

## Acceptance Criteria

1. **Given** the user navigates to the Profiles tab
   **When** profiles exist
   **Then** the profile list displays each profile with: name, source, destination, last run status/time, next scheduled run, and a Start/Cancel button (FR53)
   **And** the list renders within 300ms with 50+ profiles (NFR2)

2. **Given** a profile is idle in the main GUI
   **When** the user clicks Start
   **Then** the job begins executing via JobManager (FR21)
   **And** the status badge updates to running

3. **Given** a profile has a running job in the main GUI
   **When** the user clicks Cancel
   **Then** a confirmation dialog appears (NFR20)
   **And** upon confirmation, the job is cancelled (FR23)

4. **Given** a job is currently running
   **When** the user views the running job
   **Then** live log output streams in real-time via LiveLogView (FR42)
   **And** log updates appear within 100ms of rclone output (NFR3)

## Tasks / Subtasks

- [ ] Task 1: Create GUIProfileRow (AC: #1)
  - [ ] 1.1: Create `Views/Components/GUIProfileRow.swift`
  - [ ] 1.2: Display: StatusBadge + name + source → remote:path + last run status/time
  - [ ] 1.3: Show next scheduled run if schedule exists (display "No schedule" otherwise)
  - [ ] 1.4: Start button for idle profiles, Cancel button for running profiles
- [ ] Task 2: Update ProfileListView (AC: #1, #2, #3)
  - [ ] 2.1: Update `Views/MainWindow/Profiles/ProfileListView.swift`
  - [ ] 2.2: Use SwiftUI `List` with `GUIProfileRow` for each profile
  - [ ] 2.3: Wire Start action → JobManager.startJob
  - [ ] 2.4: Wire Cancel action → confirmation alert → JobManager.cancelJob
  - [ ] 2.5: Keep "New Profile" and edit/delete actions from previous stories
- [ ] Task 3: Create LiveLogView (AC: #4)
  - [ ] 3.1: Create `Views/MainWindow/History/LiveLogView.swift`
  - [ ] 3.2: Observe `LogStore.liveBuffer[profileId]` for streaming content
  - [ ] 3.3: Display in scrollable monospace text view
  - [ ] 3.4: Auto-scroll to bottom as new content arrives
  - [ ] 3.5: Respect `accessibilityReduceMotion` — disable auto-scroll animation if set
- [ ] Task 4: Wire live log to running jobs (AC: #4)
  - [ ] 4.1: When a job starts from the GUI, show LiveLogView below the profile list or as expandable section
  - [ ] 4.2: LiveLogView reads from LogStore.liveBuffer which is updated by JobManager's pipe handlers
  - [ ] 4.3: Log updates display within 100ms (NFR3) — SwiftUI observes @Observable LogStore
- [ ] Task 5: Write tests
  - [ ] 5.1: Test GUIProfileRow displays correct information for idle and running states
  - [ ] 5.2: Test cancel confirmation dialog appears

## Dev Notes

### Architecture Compliance

**Layer:** Views (Components, MainWindow/Profiles, MainWindow/History).

**File locations:**
```
Cirrus/Cirrus/Views/
├── Components/
│   └── GUIProfileRow.swift              # NEW
├── MainWindow/
│   ├── Profiles/
│   │   └── ProfileListView.swift        # MODIFY — full implementation
│   └── History/
│       └── LiveLogView.swift            # NEW
```

### Technical Requirements

**GUIProfileRow — full-detail profile row:**
- Displays more info than PopupProfileRow (source, destination, schedule)
- Same StatusBadge component reused
- Action closures passed in — row doesn't access managers directly

**LiveLogView — streaming log display:**
```swift
struct LiveLogView: View {
    let profileId: UUID
    @Environment(LogStore.self) var logStore

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                Text(logStore.liveBuffer[profileId] ?? "")
                    .font(.system(.body, design: .monospaced))
                    .id("bottom")
            }
            .onChange(of: logStore.liveBuffer[profileId]) {
                if !reduceMotion {
                    proxy.scrollTo("bottom", anchor: .bottom)
                }
            }
        }
    }
}
```
- Monospace font for log readability
- Auto-scroll to bottom on new content
- Check `accessibilityReduceMotion` for animation

**Performance (NFR2):** SwiftUI `List` is lazy by default — renders only visible rows. With `@Observable` ProfileStore, the list efficiently updates only changed rows. 50+ profiles should render well within 300ms.

**Performance (NFR3):** `LogStore.liveBuffer` is `@Observable`. When `appendChunk` updates the buffer on main actor, SwiftUI immediately re-renders LiveLogView. The pipe → main actor path is < 100ms.

### Enforcement Rules

- `GUIProfileRow` is a pure view — takes Profile + status + action closures
- Cancel MUST show confirmation dialog (NFR20)
- Live log reads from `LogStore.liveBuffer` — not directly from pipe handles
- Auto-scroll respects `accessibilityReduceMotion`

### Dependencies

- **Depends on:** Story 2.2 (ProfileStore), Story 2.3 (ProfileListView), Story 3.1 (JobManager, LogStore)
- **Does NOT depend on:** Epics 4-5

### References

- [Source: architecture.md#View Patterns] — pure views, @Environment
- [Source: architecture.md#Concurrency & Threading Patterns] — pipe → @MainActor → UI
- [Source: ux-design-specification.md] — Live log streaming, reduced motion
- [Source: epics.md#Story 3.3] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
