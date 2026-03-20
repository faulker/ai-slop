# Story 4.2: Log Viewer & History Controls

Status: ready-for-dev

## Story

As a user,
I want to view syntax-highlighted logs and control jobs from the history tab,
so that I can diagnose failures and re-run jobs without switching screens.

## Acceptance Criteria

1. **Given** the user views a profile's run history
   **When** they click on a historical run entry
   **Then** a log viewer sheet opens displaying the raw rclone output for that run (FR40)

2. **Given** the log viewer is displaying output
   **When** the log contains error lines (e.g., "ERROR", "Failed to")
   **Then** those lines are highlighted with a red background (FR41)
   **And** warning lines are highlighted with a yellow background (FR41)

3. **Given** the user is on the history tab
   **When** a profile is selected
   **Then** Start and Cancel controls are available (FR57)
   **And** Start is shown for idle profiles, Cancel for running profiles

4. **Given** a job is currently running for the selected profile
   **When** the user views the history tab
   **Then** live log output streams at the top of the history view (FR58)
   **And** updates appear within 100ms of rclone output (NFR3)

5. **Given** the user clicks Start from the history tab
   **When** the job begins
   **Then** the live log view activates and streams output in real-time

## Tasks / Subtasks

- [ ] Task 1: Create LogViewerSheet (AC: #1, #2)
  - [ ] 1.1: Create `Views/MainWindow/History/LogViewerSheet.swift`
  - [ ] 1.2: Load raw log file content from disk via LogStore: `logStore.readLogFile(fileName:)`
  - [ ] 1.3: Display in scrollable monospace text view
  - [ ] 1.4: Apply syntax highlighting: scan each line for patterns
  - [ ] 1.5: Lines containing "ERROR" or "Failed" → red background (`.background(Color.red.opacity(0.2))`)
  - [ ] 1.6: Lines containing "NOTICE" or "WARNING" → yellow background
  - [ ] 1.7: Add close button and copy-to-clipboard button
- [ ] Task 2: Add log file reading to LogStore (AC: #1)
  - [ ] 2.1: Add `func readLogFile(fileName: String) -> String` to LogStore
  - [ ] 2.2: Read from `{configDir}/logs/runs/{fileName}`
  - [ ] 2.3: Return file contents as string, or error message if file not found
- [ ] Task 3: Wire history row to log viewer (AC: #1)
  - [ ] 3.1: Update HistoryRunRow to be tappable
  - [ ] 3.2: On tap, present LogViewerSheet as `.sheet(item:)`
  - [ ] 3.3: Pass the LogEntry to the sheet for file lookup
- [ ] Task 4: Add Start/Cancel to history tab (AC: #3, #5)
  - [ ] 4.1: Update HistoryTabView — add Start/Cancel button below profile dropdown
  - [ ] 4.2: Start: check network, start via JobManager
  - [ ] 4.3: Cancel: confirmation dialog, cancel via JobManager
- [ ] Task 5: Add live log to history tab (AC: #4)
  - [ ] 5.1: When selected profile has running job, show LiveLogView at top of history
  - [ ] 5.2: Reuse LiveLogView component from Story 3.3
  - [ ] 5.3: Live log + historical runs display in same view
- [ ] Task 6: Write tests
  - [ ] 6.1: Test LogStore.readLogFile returns content for existing file
  - [ ] 6.2: Test LogStore.readLogFile handles missing file gracefully
  - [ ] 6.3: Test syntax highlighting identifies ERROR and WARNING lines

## Dev Notes

### Architecture Compliance

**Layer:** Views (MainWindow/History).

**File locations:**
```
Cirrus/Cirrus/Views/MainWindow/History/
├── HistoryTabView.swift                 # MODIFY — add Start/Cancel, live log
├── HistoryRunRow.swift                  # MODIFY — add tap action
└── LogViewerSheet.swift                 # NEW
Cirrus/Cirrus/Stores/
└── LogStore.swift                       # MODIFY — add readLogFile
CirrusTests/Stores/
└── LogStoreTests.swift                  # MODIFY — add readLogFile tests
```

### Technical Requirements

**Syntax highlighting for log viewer (FR41):**
```swift
ForEach(lines.indices, id: \.self) { index in
    let line = lines[index]
    Text(line)
        .font(.system(.body, design: .monospaced))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 8)
        .padding(.vertical, 1)
        .background(backgroundColor(for: line))
}

func backgroundColor(for line: String) -> Color {
    let upper = line.uppercased()
    if upper.contains("ERROR") || upper.contains("FAILED") {
        return Color.red.opacity(0.15)
    }
    if upper.contains("WARNING") || upper.contains("NOTICE") {
        return Color.yellow.opacity(0.15)
    }
    return Color.clear
}
```

**Log file reading:**
```swift
func readLogFile(fileName: String) -> String {
    let url = configDirectoryURL()
        .appendingPathComponent("logs")
        .appendingPathComponent("runs")
        .appendingPathComponent(fileName)
    return (try? String(contentsOf: url, encoding: .utf8)) ?? "Log file not found."
}
```
- `try?` is acceptable here — this is a best-effort read for display purposes
- Missing files show a clear message, not a crash

**Reuse LiveLogView:** The same `LiveLogView` from Story 3.3 is reused here. It reads from `LogStore.liveBuffer[profileId]` and displays streaming output.

**Sheet pattern:**
```swift
.sheet(item: $selectedLogEntry) { entry in
    LogViewerSheet(entry: entry)
}
```

### Enforcement Rules

- Reuse `LiveLogView` from Story 3.3 — do NOT create a duplicate
- Reuse `StatusBadge` for all status indicators
- Log viewer is read-only — no editing of log files
- Log file reading uses `try?` (acceptable for display) — but log WRITING uses `try` (never silent)
- Cancel MUST show confirmation dialog (NFR20)

### Dependencies

- **Depends on:** Story 4.1 (HistoryTabView, HistoryRunRow), Story 3.1 (LogStore, JobManager), Story 3.3 (LiveLogView)
- **Does NOT depend on:** Epic 5

### References

- [Source: architecture.md#View Patterns] — sheets, @Environment
- [Source: architecture.md#Data Architecture] — log file storage
- [Source: ux-design-specification.md] — Log viewer highlighting, red/yellow backgrounds
- [Source: epics.md#Story 4.2] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
