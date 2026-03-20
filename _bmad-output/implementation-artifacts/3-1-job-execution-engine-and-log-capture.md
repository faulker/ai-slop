# Story 3.1: Job Execution Engine & Log Capture

Status: ready-for-dev

## Story

As a user,
I want my rclone jobs to execute reliably with complete log capture,
so that every sync operation is tracked and I never lose visibility into what happened.

## Acceptance Criteria

1. **Given** a job is started for a profile
   **When** JobManager receives the start request
   **Then** the profile configuration is snapshotted as a value-type copy (FR25)
   **And** RcloneService assembles the command with direct args for flags and a temp filter file for ignore patterns (FR26)
   **And** a `Process` is spawned with stdout and stderr `Pipe` attached

2. **Given** a job is running
   **When** rclone produces output
   **Then** stdout and stderr are captured completely via `readabilityHandler` on a background queue (FR35)
   **And** output chunks are appended to a raw log file at `{configDir}/logs/runs/{profileId}_{timestamp}.log` (FR37)
   **And** a `LogEntry` is created in the JSON log index with profile ID, timestamp, status, and duration (FR36)

3. **Given** multiple jobs are started concurrently
   **When** they execute simultaneously
   **Then** each job runs independently with its own Process, Pipe, and log file (FR24)
   **And** the UI remains responsive (NFR7)

4. **Given** the app is quit while jobs are running
   **When** the quit sequence begins
   **Then** all running processes receive SIGTERM (FR28)
   **And** after 2 seconds, any remaining processes receive SIGKILL (NFR10)
   **And** log files for interrupted jobs are finalized with "interrupted" status

5. **Given** a job completes (success or failure)
   **When** the `terminationHandler` fires
   **Then** remaining pipe data is read and flushed to the log file
   **And** the LogEntry is updated with final status and duration
   **And** the temp filter file is cleaned up
   **And** job status updates within 1 second (NFR4)

## Tasks / Subtasks

- [ ] Task 1: Create LogEntry model (AC: #2)
  - [ ] 1.1: Create `Models/LogEntry.swift`
  - [ ] 1.2: Fields: id (UUID), profileId (UUID), startedAt (Date), completedAt (Date?), status (JobStatus), durationSeconds (Double?), logFileName (String)
  - [ ] 1.3: `Codable` conformance
- [ ] Task 2: Create JobRun model (AC: #1)
  - [ ] 2.1: Create `Models/JobRun.swift`
  - [ ] 2.2: Fields: profileId (UUID), profileSnapshot (Profile), process (Process), startedAt (Date), status (JobStatus), logFileURL (URL)
  - [ ] 2.3: This is an in-memory runtime object — NOT Codable (contains Process reference)
- [ ] Task 3: Implement FilterFileWriter (AC: #1)
  - [ ] 3.1: Create `Services/FilterFileWriter.swift`
  - [ ] 3.2: `static func write(patterns: [String]) throws -> URL` — writes temp filter file
  - [ ] 3.3: Each pattern as `- pattern` line (rclone filter-from format)
  - [ ] 3.4: `static func cleanup(at url: URL)` — deletes temp file
  - [ ] 3.5: Temp files in `FileManager.default.temporaryDirectory`
- [ ] Task 4: Enhance RcloneService for command assembly (AC: #1)
  - [ ] 4.1: Add `buildCommand(profile: Profile, filterFileURL: URL?) -> [String]` to RcloneService
  - [ ] 4.2: Assemble: `[action, sourcePath, remote:path, --filter-from filterFile, extraFlags...]`
  - [ ] 4.3: Return array of arguments (not including rclone binary path)
- [ ] Task 5: Implement LogStore (AC: #2)
  - [ ] 5.1: Create `Stores/LogStore.swift` — `@MainActor @Observable`
  - [ ] 5.2: `private(set) var entries: [LogEntry]` — loaded from `{configDir}/logs/index.json`
  - [ ] 5.3: `func loadIndex()` — read and decode index.json
  - [ ] 5.4: `func createEntry(profileId:logFileName:) -> LogEntry` — add to index
  - [ ] 5.5: `func finalizeEntry(id:status:duration:)` — update entry, save index
  - [ ] 5.6: `func saveIndex()` — write via AtomicFileWriter (NFR12)
  - [ ] 5.7: `var liveBuffer: [UUID: String]` — per-job live output for streaming (FR42)
  - [ ] 5.8: `func appendChunk(jobId:chunk:)` — append to live buffer and file
- [ ] Task 6: Implement JobManager (AC: #1-5)
  - [ ] 6.1: Create `Stores/JobManager.swift` — `@MainActor @Observable`
  - [ ] 6.2: `private(set) var activeJobs: [UUID: JobRun]` — keyed by profile ID
  - [ ] 6.3: `func startJob(for profile: Profile)` — snapshot, build command, spawn Process, track
  - [ ] 6.4: Set up stdout/stderr `Pipe` with `readabilityHandler` on background queue
  - [ ] 6.5: Forward chunks to `LogStore.appendChunk()` via `Task { @MainActor in ... }`
  - [ ] 6.6: Set `terminationHandler` — flush remaining data, finalize LogEntry, clean up filter file, remove from activeJobs
  - [ ] 6.7: `func cancelJob(for profileId: UUID)` — send SIGTERM, wait, SIGKILL
  - [ ] 6.8: `func cancelAllJobs()` — SIGTERM all, wait 2s, SIGKILL remaining
  - [ ] 6.9: `func isRunning(for profileId: UUID) -> Bool`
  - [ ] 6.10: `var runningCount: Int` — computed from activeJobs
- [ ] Task 7: Implement NetworkMonitor (AC: implicit from FR27)
  - [ ] 7.1: Create `Utilities/NetworkMonitor.swift`
  - [ ] 7.2: Wrap `NWPathMonitor` for connectivity checking
  - [ ] 7.3: `@MainActor @Observable` with `isConnected: Bool` property
  - [ ] 7.4: Start monitoring on init, stop on deinit
- [ ] Task 8: Wire into app lifecycle
  - [ ] 8.1: Create JobManager and LogStore instances in CirrusApp/AppDelegate
  - [ ] 8.2: Inject via `.environment()` on root views
  - [ ] 8.3: Call `jobManager.cancelAllJobs()` in app termination handler
  - [ ] 8.4: Update quit confirmation to show count of running jobs
- [ ] Task 9: Write tests
  - [ ] 9.1: `CirrusTests/Services/FilterFileWriterTests.swift` — write/read/cleanup
  - [ ] 9.2: `CirrusTests/Services/RcloneServiceTests.swift` — command assembly (add to existing)
  - [ ] 9.3: `CirrusTests/Stores/LogStoreTests.swift` — index CRUD, entry finalization
  - [ ] 9.4: `CirrusTests/Stores/JobManagerTests.swift` — mock Process execution, cancellation
  - [ ] 9.5: `CirrusTests/Models/LogEntryTests.swift` — Codable round-trip

## Dev Notes

### Architecture Compliance

**Layer:** Models (LogEntry, JobRun) + Services (FilterFileWriter, RcloneService enhancement) + Stores (JobManager, LogStore) + Utilities (NetworkMonitor).

**This is the largest story — core execution engine.** No direct user-facing views, but provides the engine for Stories 3.2 and 3.3.

**File locations:**
```
Cirrus/Cirrus/
├── Models/
│   ├── LogEntry.swift                    # NEW
│   └── JobRun.swift                      # NEW
├── Services/
│   ├── RcloneService.swift               # MODIFY — add buildCommand
│   └── FilterFileWriter.swift            # NEW
├── Stores/
│   ├── JobManager.swift                  # NEW
│   └── LogStore.swift                    # NEW
└── Utilities/
    └── NetworkMonitor.swift              # NEW
CirrusTests/
├── Models/
│   └── LogEntryTests.swift              # NEW
├── Services/
│   ├── FilterFileWriterTests.swift      # NEW
│   └── RcloneServiceTests.swift         # MODIFY
└── Stores/
    ├── JobManagerTests.swift            # NEW
    └── LogStoreTests.swift              # NEW
```

### Technical Requirements

**Process lifecycle (exact sequence):**
1. Snapshot profile: `let snapshot = profile` (value-type copy)
2. Write filter file: `let filterURL = try FilterFileWriter.write(patterns: snapshot.ignorePatterns)`
3. Build command: `let args = RcloneService.buildCommand(profile: snapshot, filterFileURL: filterURL)`
4. Create Process: `let process = Process(); process.executableURL = URL(fileURLWithPath: rclonePath); process.arguments = args`
5. Set up pipes: `let stdoutPipe = Pipe(); let stderrPipe = Pipe(); process.standardOutput = stdoutPipe; process.standardError = stderrPipe`
6. Set `readabilityHandler` on both pipes — forward to LogStore on background queue
7. Set `terminationHandler` — finalize on main actor
8. Launch: `try process.run()`
9. Store in activeJobs dictionary

**Pipe reading pattern:**
```swift
stdoutPipe.fileHandleForReading.readabilityHandler = { handle in
    let data = handle.availableData
    guard !data.isEmpty else { return }
    let chunk = String(data: data, encoding: .utf8) ?? ""
    Task { @MainActor in
        logStore.appendChunk(jobId: jobRun.id, chunk: chunk)
    }
}
```

**Log file naming:** `{profileId}_{ISO8601timestamp}.log`
- Replace colons with hyphens in timestamp for filesystem compatibility
- Example: `a1b2c3d4_2026-02-27T14-30-00Z.log`

**Log index:** `{configDir}/logs/index.json` — array of `LogEntry` objects
- Written via AtomicFileWriter on every finalization
- Consistency with raw files (NFR12): entry points to filename, file must exist

**SIGTERM → SIGKILL sequence:**
```swift
func cancelAllJobs() {
    for (_, job) in activeJobs {
        job.process.terminate()  // SIGTERM
    }
    Task {
        try await Task.sleep(for: .seconds(2))
        for (_, job) in activeJobs where job.process.isRunning {
            kill(job.process.processIdentifier, SIGKILL)
        }
    }
}
```

### Enforcement Rules

- `JobManager` and `LogStore` are `@MainActor @Observable`
- Config snapshot is automatic — Profile is a value type (struct)
- `RcloneService` is the SOLE component that builds rclone commands
- Pipe → main thread via `Task { @MainActor in ... }` — NOT `DispatchQueue.main.async`
- Never use `try?` on process spawning or log writes — always propagate errors
- Log index writes use `AtomicFileWriter` (NFR12)
- No orphaned processes: `cancelAllJobs()` called on app quit (NFR10)

### Dependencies

- **Depends on:** Story 1.1 (CirrusError, AtomicFileWriter, JSONCoders), Story 2.1 (RcloneService), Story 2.2 (Profile model, ProfileStore)
- **Does NOT depend on:** Stories 3.2-3.3 or Epics 4-5

### References

- [Source: architecture.md#Process Management Strategy] — Complete lifecycle
- [Source: architecture.md#Concurrency & Threading Patterns] — readabilityHandler, @MainActor
- [Source: architecture.md#Data Architecture] — File storage layout, log naming
- [Source: architecture.md#State Management Patterns] — @Observable, private(set)
- [Source: epics.md#Story 3.1] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
