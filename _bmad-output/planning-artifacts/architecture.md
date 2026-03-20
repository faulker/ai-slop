---
stepsCompleted: [1, 2, 3, 4, 5, 6, 7, 8]
lastStep: 8
status: 'complete'
completedAt: '2026-02-27'
inputDocuments: ['_bmad-output/planning-artifacts/prd.md', '_bmad-output/planning-artifacts/ux-design-specification.md', '_bmad-output/brainstorming/brainstorming-session-2026-02-27.md']
workflowType: 'architecture'
project_name: 'Cirrus'
user_name: 'Sane'
date: '2026-02-27'
---

# Architecture Decision Document

_This document builds collaboratively through step-by-step discovery. Sections are appended as we work through each architectural decision together._

## Project Context Analysis

### Requirements Overview

**Functional Requirements:**
62 FRs across 8 capability areas. The app is a thin orchestration layer — it doesn't process files, it assembles rclone commands from profile configurations and manages the execution lifecycle. The architectural weight is in process management, state synchronization, and real-time I/O streaming, not in domain logic.

| Category | FR Count | Architectural Weight |
|----------|----------|---------------------|
| rclone Setup & Configuration | 7 | Low — one-time setup flows, PATH detection, version check |
| Profile Management | 12 | Medium — CRUD with JSON persistence, command parsing, form validation |
| Job Execution | 9 | **High** — process spawning, tracking, cancellation, config snapshots, concurrency |
| Scheduling | 6 | Medium — cron expression evaluation, timer management, missed job detection |
| Logging & History | 8 | **High** — real-time stdout/stderr capture, file storage, streaming to UI |
| Tray Popup Dashboard | 10 | Medium — custom NSWindow, state display, action dispatch |
| Main GUI | 6 | Low — standard SwiftUI views consuming shared state |
| App Lifecycle | 4 | Medium — Login Item registration, quit handling, process cleanup |

**Non-Functional Requirements:**
21 NFRs. The performance NFRs (popup < 200ms, log streaming < 100ms, UI responsiveness during concurrent jobs) require careful async architecture. The reliability NFRs (atomic writes, no orphaned processes, complete log capture) require robust process and file management.

**Scale & Complexity:**
- Primary domain: Native macOS desktop application
- Complexity level: Low — no backend, no database, no network protocol implementation
- Estimated architectural components: ~8-10 modules
- Key technical challenge: Process lifecycle management with real-time I/O

### Technical Constraints & Dependencies

- **Swift + SwiftUI** — Language and framework locked in from brainstorming/PRD
- **macOS Ventura+** — Minimum OS version, determines available APIs
- **rclone CLI** — External dependency, not bundled. App shells out via `Process` API. Must support rclone 1.60+
- **No App Sandbox** — Required for arbitrary filesystem access and child process spawning. Distribution outside Mac App Store (GitHub releases)
- **No backend** — All data local. No network calls except rclone's own operations
- **Solo developer** — Architecture must be simple enough for one person to build and maintain

### Cross-Cutting Concerns Identified

1. **State synchronization** — Profile state, job status, and history must be consistent across tray popup and main GUI. Both surfaces observe the same data model. Changes (job starts, completes, fails) propagate to all visible UI instantly.

2. **Process lifecycle management** — Every rclone child process must be tracked from spawn to termination. Cleanup on app quit (SIGTERM children). Handle edge cases: force quit, system sleep, crash recovery. No orphaned processes (NFR10).

3. **Async I/O and threading** — rclone stdout/stderr read on background threads. UI updates dispatched to main thread. File writes on background thread with atomic rename. Timer callbacks for cron scheduling. All must be thread-safe.

4. **File atomicity** — Profile JSON and log index writes must be atomic (write to temp file, rename). Prevents corruption from crashes or power loss (NFR11). Log index must stay consistent with raw log files (NFR12).

5. **Real-time UI updates** — Status badges, elapsed timers, and live log streaming require sub-second UI refresh from background process events. SwiftUI's `@Observable` / `@Published` pattern is the natural fit.

## Starter Template Evaluation

### Primary Technology Domain

Native macOS desktop application — Swift + SwiftUI. No web, no cross-platform. The "starter" is Xcode's macOS App template with specific configuration choices.

### Key Scaffolding Decisions

#### Menu Bar Implementation Approach

Two options evaluated:

| Approach | Pros | Cons |
|----------|------|------|
| `MenuBarExtra` (.window style) | Apple's native API, simple setup, built into SwiftUI scene lifecycle | Limited control over window chrome, translucency, positioning, and dismiss behavior |
| Custom `NSStatusItem` + `NSWindow` + `NSHostingView` | Full control over borderless window, `NSVisualEffectView` vibrancy, precise positioning, custom dismiss logic | More AppKit boilerplate, manual window lifecycle management |

**Decision: Custom NSStatusItem + NSWindow approach.** The UX spec requires a borderless floating panel with vibrancy/translucency, precise anchor positioning below the menu bar icon, and custom dismiss behavior (click-outside, Escape, icon re-click). `MenuBarExtra` with `.window` style doesn't provide sufficient control over these behaviors.

#### Testing Framework

| Framework | Best For | Limitation |
|-----------|----------|------------|
| Swift Testing (`@Test`, `#expect()`) | Unit tests — modern syntax, parallel by default, parameterized tests via traits | No UI testing, no performance testing, Swift 6+ / Xcode 16+ only |
| XCTest | UI tests, performance tests, established ecosystem | Verbose, class-based, Obj-C heritage |

**Decision: Swift Testing for unit tests, XCTest reserved for UI tests if needed.** Swift Testing is Apple's recommended path for new projects. Both frameworks can coexist in the same test target.

#### Package Manager

**Swift Package Manager (SPM)** — built into Xcode, standard for Swift projects. No CocoaPods or Carthage.

#### Third-Party Dependencies (Minimal)

| Category | Decision |
|----------|----------|
| UI Components | None — SwiftUI native + custom components per UX spec |
| Cron Parsing | Evaluate lightweight Swift cron libraries (or write ~100 lines of cron evaluation) |
| JSON Coding | Foundation's `Codable` — no external JSON libraries |
| Networking | None — rclone handles all network operations |
| Logging (internal) | `os.Logger` (Apple's unified logging) for app diagnostics |
| Updates | None for MVP. Sparkle framework for Growth phase |

### Selected Starter: Xcode macOS App Template

**Initialization:**

```
Xcode → New Project → macOS → App
- Interface: SwiftUI
- Language: Swift
- Testing System: Swift Testing
- App Lifecycle: SwiftUI App
- Bundle Identifier: com.sane.cirrus
```

**Post-initialization configuration:**
- Disable App Sandbox entitlement (required for arbitrary filesystem access and child process spawning)
- Add Login Items capability for launch-at-startup
- Set deployment target to macOS 14.0 (Sonoma)
- Configure signing for Developer ID (outside App Store distribution)

**Architectural Decisions Provided by Starter:**

- **Language & Runtime:** Swift 6, SwiftUI App lifecycle (`@main` struct)
- **Build Tooling:** Xcode build system, Swift compiler
- **Testing:** Swift Testing (unit), XCTest (UI if needed)
- **Code Organization:** Single-target macOS app with standard Xcode project structure
- **Development Experience:** Xcode previews, SwiftUI hot reload, LLDB debugger

**Note:** Project initialization using this Xcode setup should be the first implementation story.

## Core Architectural Decisions

### Decision Priority Analysis

**Critical Decisions (Block Implementation):**
- Deployment target, observable pattern, state management, project organization, process management, data models

**Not Applicable to Cirrus:**
- Authentication & Security (no user accounts, no backend)
- API & Communication (no API — rclone is invoked as a subprocess)
- Infrastructure & Deployment (local desktop app, GitHub releases for distribution)

**Deferred Decisions (Post-MVP):**
- Update framework (Sparkle — Growth phase)
- Profile import/export format (Vision phase)

### Deployment Target

- **Decision:** macOS 14+ (Sonoma)
- **Rationale:** Unlocks `@Observable` macro, modern SwiftUI APIs. Covers last ~3 major releases. Reasonable coverage for a developer/power-user audience.
- **Affects:** All SwiftUI views, state management pattern, available system APIs

### State Management Architecture

**Decision: `@Observable` macro with dedicated manager classes.**

The app uses 5 observable manager objects, injected into the SwiftUI environment at app startup:

| Manager | Responsibility | Observes/Owns |
|---------|---------------|---------------|
| `ProfileStore` | Profile CRUD, JSON persistence, atomic writes | Array of `Profile` structs |
| `JobManager` | Process spawning, tracking, cancellation, config snapshots | Dictionary of `JobRun` keyed by profile ID |
| `ScheduleManager` | Cron evaluation, timer firing, missed job detection | Timer references, schedule state per profile |
| `LogStore` | Log index management, raw log file access, streaming | Log index entries, file handles for active streams |
| `AppSettings` | rclone path, config location, theme, general preferences | Settings struct with JSON persistence |

**Rationale:** Each manager has a single responsibility. All are `@Observable`, so SwiftUI views automatically react to state changes. No single "god object." Managers communicate through direct method calls (e.g., `ScheduleManager` calls `JobManager.startJob()`).

**Dependency flow:** `ScheduleManager` → `JobManager` → `LogStore`. `ProfileStore` is independent. `AppSettings` is read by all.

### Project File Organization

```
Cirrus/
├── CirrusApp.swift              # @main entry, environment injection
├── AppDelegate.swift            # NSStatusItem setup, NSWindow management
├── Models/
│   ├── Profile.swift            # Profile Codable struct
│   ├── JobRun.swift             # Job execution record struct
│   ├── LogEntry.swift           # Log index entry struct
│   └── AppSettingsModel.swift   # Settings Codable struct
├── Stores/
│   ├── ProfileStore.swift       # Profile CRUD + persistence
│   ├── JobManager.swift         # Process lifecycle management
│   ├── ScheduleManager.swift    # Cron timer management
│   ├── LogStore.swift           # Log index + file management
│   └── AppSettings.swift        # Settings persistence
├── Services/
│   ├── RcloneService.swift      # Command assembly, PATH detection, version check
│   ├── RcloneCommandParser.swift # Paste-to-create parsing
│   └── FilterFileWriter.swift   # Temporary filter file generation
├── Views/
│   ├── TrayPopup/
│   │   ├── TrayPopupPanel.swift     # NSWindow + NSHostingView setup
│   │   ├── TrayPopupView.swift      # Root SwiftUI view for popup
│   │   └── PopupProfileRow.swift    # Profile row — popup variant
│   ├── MainWindow/
│   │   ├── MainWindowView.swift     # TabView root
│   │   ├── Profiles/
│   │   │   ├── ProfileListView.swift
│   │   │   └── ProfileFormView.swift
│   │   ├── History/
│   │   │   ├── HistoryTabView.swift
│   │   │   ├── HistoryRunRow.swift
│   │   │   └── LogViewerSheet.swift
│   │   └── Settings/
│   │       └── SettingsTabView.swift
│   └── Components/
│       ├── StatusBadge.swift
│       ├── GUIProfileRow.swift
│       └── CronBuilderView.swift
└── Utilities/
    ├── CronParser.swift         # Cron expression evaluation
    ├── AtomicFileWriter.swift   # Write-to-temp + rename
    └── NetworkMonitor.swift     # NWPathMonitor wrapper
```

**Rationale:** Flat enough for a solo developer to navigate quickly. Grouped by architectural role (Models, Stores, Services, Views, Utilities). Views mirror the two UI surfaces (TrayPopup, MainWindow) plus shared Components.

### Process Management Strategy

**Decision: `JobManager` owns all `Process` instances in a dictionary keyed by profile ID.**

**Lifecycle:**
1. **Start:** Snapshot profile config → write temp filter file if needed → assemble `Process` with args → set up stdout/stderr `Pipe` → launch process → store in active jobs dictionary → notify `LogStore` to begin streaming
2. **Stream:** Background `DispatchQueue` reads pipe `fileHandleForReading` with `readabilityHandler`. Each chunk forwarded to `LogStore` which appends to raw log file and publishes to UI via `@Observable`
3. **Complete:** `terminationHandler` fires → read remaining pipe data → finalize log file → update `LogStore` index → update job status → remove from active dictionary → clean up temp filter file
4. **Cancel:** Send `SIGTERM` to process → wait briefly → `SIGKILL` if still running → same completion flow
5. **App Quit:** Iterate active jobs dictionary → `SIGTERM` all → wait up to 2 seconds → `SIGKILL` remaining → exit

**Config snapshot:** Profile struct is value-type (`struct`). Copying it at job start is the snapshot — no deep copy needed.

**Concurrency:** No enforced limit. Each job gets its own `Process` + `Pipe` + background read queue. Dictionary access protected by `@MainActor` since `JobManager` is `@Observable`.

### Data Architecture

**Profile Model (`Profile.swift`):**
```swift
struct Profile: Codable, Identifiable {
    let id: UUID
    var name: String
    var sourcePath: String
    var remoteName: String
    var remotePath: String
    var action: RcloneAction      // enum: sync, copy, move, delete
    var ignorePatterns: [String]
    var extraFlags: String
    var schedule: CronSchedule?   // optional cron expression + enabled flag
    var groupName: String?        // for future profile grouping
    var sortOrder: Int
    var createdAt: Date
    var updatedAt: Date
}
```

**File Storage Layout:**
```
~/.config/cirrus/
├── settings.json                # AppSettings
├── profiles/
│   ├── {uuid}.json              # One file per profile
│   └── ...
└── logs/
    ├── index.json               # Array of LogEntry (manifest)
    └── runs/
        ├── {uuid}_{timestamp}.log   # Raw rclone output per run
        └── ...
```

**Atomic Write Strategy:** `AtomicFileWriter` writes to `{filename}.tmp` in the same directory, then calls `FileManager.replaceItem(at:withItemAt:)` for atomic rename. This is crash-safe — either the old file or the new file exists, never a partial write.

### Error Handling Strategy

**Error Classification:**

| Category | Examples | User-Facing Treatment |
|----------|----------|----------------------|
| rclone errors | Non-zero exit code, connection refused, permission denied | Red status badge + error preserved in log. User sees badge, clicks through to log viewer with highlighted error lines |
| File I/O errors | Can't write profile JSON, log directory missing | Inline error message in the relevant view. Retry action where applicable |
| Configuration errors | rclone not found, invalid profile fields | Guided resolution flow (locate rclone, highlight invalid fields) |
| System errors | Network unavailable, process spawn failed | Inline message with explanation. Block action until resolved |

**Error Propagation:** Managers throw typed errors (`CirrusError` enum with cases like `.rcloneNotFound`, `.profileSaveFailure`, `.processSpawnFailed`). Views catch and display inline. No silent failures — every error surfaces in the UI per NFR8 and NFR21.

**No crash-on-error:** All errors are recoverable. The app never force-unwraps external data or crashes on unexpected rclone output.

### Decision Impact Analysis

**Implementation Sequence:**
1. Project setup (Xcode template + configuration)
2. Data models (`Profile`, `LogEntry`, `AppSettings`)
3. `AtomicFileWriter` + `ProfileStore` (persistence layer)
4. `RcloneService` (command assembly, PATH detection)
5. `JobManager` + `LogStore` (process lifecycle + log capture)
6. Tray popup UI (NSStatusItem + NSWindow + popup views)
7. Main GUI (TabView + profile list + profile form)
8. History tab + log viewer
9. `ScheduleManager` + `CronParser`
10. Polish (empty states, error flows, accessibility)

**Cross-Component Dependencies:**
- `JobManager` depends on `ProfileStore` (reads profile), `LogStore` (writes logs), `RcloneService` (builds commands)
- `ScheduleManager` depends on `JobManager` (triggers jobs), `ProfileStore` (reads schedules)
- All Views depend on their respective Stores via SwiftUI environment
- `AppSettings` is a dependency of `RcloneService` (rclone path) and all views (theme)

## Implementation Patterns & Consistency Rules

### Pattern Categories Defined

**12 critical conflict areas** where AI agents could make different choices when implementing Cirrus.

### Naming Patterns

**Swift Naming Conventions (follow Apple's API Design Guidelines):**

| Element | Convention | Example |
|---------|-----------|---------|
| Types (struct, class, enum, protocol) | PascalCase | `ProfileStore`, `RcloneAction`, `JobStatus` |
| Functions, methods, properties | camelCase | `startJob()`, `profileName`, `isRunning` |
| Enum cases | camelCase | `.sync`, `.copy`, `.failed`, `.running` |
| Boolean properties | Reads as assertion | `isRunning`, `hasSchedule`, `canStart` |
| File names | Match primary type | `ProfileStore.swift`, `StatusBadge.swift` |
| Test files | Suffix with `Tests` | `ProfileStoreTests.swift`, `CronParserTests.swift` |

**Anti-patterns:**
- `ProfileMgr` → use `ProfileStore` (no abbreviations except widely understood ones like `URL`, `ID`)
- `getProfile()` → use `profile(for:)` (no `get` prefix — Swift convention)
- `ProfileData` → use `Profile` (no `Data`/`Info` suffixes on models)

### JSON & Data Format Patterns

**JSON Serialization (all `Codable` structs):**

| Field | Format | Example |
|-------|--------|---------|
| Property names | camelCase (Swift default) | `"sourcePath"`, `"remoteName"` |
| Dates | ISO 8601 string | `"2026-02-27T14:30:00Z"` |
| UUIDs | Lowercase string | `"a1b2c3d4-e5f6-..."` |
| Enums | Raw string value | `"sync"`, `"failed"`, `"running"` |
| Optional fields | Omitted when nil | Field absent from JSON, not `null` |

**Encoder/Decoder Configuration (set once, used everywhere):**
```swift
extension JSONEncoder {
    static let cirrus: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }()
}
extension JSONDecoder {
    static let cirrus: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()
}
```

**Rule:** All JSON encoding/decoding uses `JSONEncoder.cirrus` and `JSONDecoder.cirrus`. Never create one-off encoders.

### File & Path Patterns

**Log file naming:** `{profileId}_{ISO8601timestamp}.log`
- Example: `a1b2c3d4_2026-02-27T14-30-00Z.log`
- Colons replaced with hyphens in timestamp for filesystem compatibility

**Profile file naming:** `{uuid}.json`
- Example: `a1b2c3d4-e5f6-7890-abcd-ef1234567890.json`

**Config directory access:** Always go through `AppSettings.configDirectoryURL`. Never hardcode `~/.config/cirrus/`. The user can change the config location.

**File path construction:** Always use `URL` and `appendingPathComponent()`. Never string-concatenate paths.

### Concurrency & Threading Patterns

**Decision: Swift Structured Concurrency (`async/await`) as the primary pattern. GCD only for `Process` pipe reading.**

| Context | Pattern | Rationale |
|---------|---------|-----------|
| Manager classes | `@MainActor @Observable` | All state mutations on main thread, automatic SwiftUI observation |
| File I/O | `async` methods, off main actor | `await profileStore.save(profile)` — non-blocking |
| Process pipe reading | `readabilityHandler` (GCD callback) | `Process`/`Pipe` API is GCD-based, not async/await |
| Pipe → main thread | `@MainActor` dispatch from handler | `Task { @MainActor in logStore.append(chunk) }` |
| Timer/scheduling | `Task.sleep` or `DispatchSourceTimer` | For cron evaluation loops |

**Anti-patterns:**
- `DispatchQueue.main.async { }` for UI updates → use `@MainActor` instead
- Bare `Thread` or `pthread` → never needed
- `DispatchSemaphore` for synchronization → use Swift actors or async/await
- Force-unwrapping (`!`) on anything from external sources (rclone output, JSON, file reads)

### State Management Patterns

**Observable Pattern:**
```swift
@MainActor @Observable
final class ProfileStore {
    private(set) var profiles: [Profile] = []
    // Views observe `profiles` automatically via @Observable
}
```

**Rules:**
- All `@Observable` managers are `@MainActor` — state mutations always on main thread
- Properties that views observe are `private(set)` — external mutation only through methods
- Managers are injected via SwiftUI `.environment()` at the app root
- Views access managers via `@Environment(ProfileStore.self) var profileStore`
- Managers never import SwiftUI — they are view-agnostic

**Inter-Manager Communication:**
- Direct method calls, not notifications or Combine publishers
- Example: `scheduleManager.onScheduleFired { profileId in jobManager.startJob(for: profileId) }`
- Closures or delegate patterns for callbacks between managers
- No `NotificationCenter` for internal app communication (hard to trace, no type safety)

### Error Handling Patterns

**Error Type:**
```swift
enum CirrusError: LocalizedError {
    case rcloneNotFound
    case rcloneExecutionFailed(exitCode: Int32, stderr: String)
    case profileSaveFailed(underlying: Error)
    case profileNotFound(id: UUID)
    case processSpawnFailed(underlying: Error)
    case networkUnavailable
    case invalidCronExpression(String)
    case configDirectoryInaccessible(path: String)

    var errorDescription: String? { /* user-facing message */ }
}
```

**Rules:**
- All errors are typed `CirrusError` — no raw `Error` throws
- Every case has a user-facing `errorDescription` (per NFR21)
- Managers throw errors — views catch and display inline
- rclone stderr is captured and preserved in `CirrusError.rcloneExecutionFailed`, never discarded
- `try?` is forbidden on operations that must not silently fail (file writes, process spawning)
- `try?` is acceptable for best-effort operations (loading cached state on startup)

### View Patterns

**SwiftUI View Rules:**
- Views are structs — no classes
- Views contain no business logic — all logic lives in managers/services
- Views access state via `@Environment` managers, not via init parameters (except leaf components like `StatusBadge` which take simple value parameters)
- Sheets/alerts use the `.sheet(item:)` / `.alert(isPresented:)` pattern
- No `AnyView` — use `@ViewBuilder` or concrete types
- Accessibility: Every custom interactive element gets `.accessibilityLabel()` and `.accessibilityHint()`

**Component Reuse:**
- `StatusBadge` is a pure view — takes `JobStatus` enum, returns colored badge. No state access.
- Profile rows (popup variant, GUI variant) are pure views — take `Profile` + `JobStatus` + action closures
- Log viewer takes `[String]` lines + highlighting rules. No direct `LogStore` access.

### Testing Patterns

**Test Organization:**
```
CirrusTests/
├── Stores/
│   ├── ProfileStoreTests.swift
│   ├── JobManagerTests.swift
│   └── LogStoreTests.swift
├── Services/
│   ├── RcloneServiceTests.swift
│   ├── RcloneCommandParserTests.swift
│   └── CronParserTests.swift
└── Models/
    └── ProfileTests.swift
```

**Rules:**
- Test files mirror the source directory structure
- Test names describe behavior: `func testStartJobUpdatesStatusToRunning()`
- Use Swift Testing `@Test` macro and `#expect()` — not XCTest assertions
- Test managers with mock dependencies (protocol-based where needed)
- `RcloneCommandParser` and `CronParser` are pure functions — heavily unit tested with parameterized tests
- No network calls in tests — mock `Process` execution for `JobManager` tests

### Enforcement Guidelines

**All AI Agents MUST:**
1. Follow Apple's Swift API Design Guidelines for all naming
2. Use `JSONEncoder.cirrus` / `JSONDecoder.cirrus` for all serialization
3. Mark all `@Observable` classes as `@MainActor`
4. Use `CirrusError` enum for all thrown errors — no raw `Error`
5. Never hardcode file paths — always derive from `AppSettings.configDirectoryURL`
6. Never use `DispatchQueue.main.async` — use `@MainActor` annotation
7. Never use `NotificationCenter` for internal communication — use direct calls
8. Never force-unwrap (`!`) external data
9. Place tests in `CirrusTests/` mirroring source structure
10. Use Swift Testing `@Test` / `#expect()` for all new tests

## Project Structure & Boundaries

### Complete Project Directory Structure

```
Cirrus/
├── Cirrus.xcodeproj/                    # Xcode project file
├── Cirrus/
│   ├── CirrusApp.swift                  # @main entry, environment injection, scene definition
│   ├── AppDelegate.swift                # NSStatusItem setup, NSWindow lifecycle, global hotkey
│   ├── Info.plist                        # App metadata, LSUIElement=true (hide dock icon)
│   ├── Cirrus.entitlements              # Sandbox disabled, network access
│   ├── Assets.xcassets/                 # App icon, menu bar icon (template image)
│   │   ├── AppIcon.appiconset/
│   │   └── MenuBarIcon.imageset/        # Grayscale template image
│   │
│   ├── Models/
│   │   ├── Profile.swift                # Profile Codable struct + RcloneAction enum
│   │   ├── JobRun.swift                 # Active job state (process ref, start time, status)
│   │   ├── LogEntry.swift               # Log index entry (profile ID, timestamp, status, duration, filename)
│   │   ├── JobStatus.swift              # Enum: idle, running, success, failed, canceled
│   │   ├── CronSchedule.swift           # Cron expression + enabled flag
│   │   └── AppSettingsModel.swift       # Settings Codable struct (rclone path, config dir, theme)
│   │
│   ├── Stores/
│   │   ├── ProfileStore.swift           # Profile CRUD, JSON persistence, load/save all
│   │   ├── JobManager.swift             # Process spawning, tracking, cancellation, cleanup
│   │   ├── ScheduleManager.swift        # Cron evaluation loop, timer management
│   │   ├── LogStore.swift               # Log index CRUD, raw file access, live streaming buffer
│   │   └── AppSettings.swift            # Settings load/save, config directory URL provider
│   │
│   ├── Services/
│   │   ├── RcloneService.swift          # Command assembly, PATH detection, version check, listremotes
│   │   ├── RcloneCommandParser.swift    # Paste-to-create: parse rclone command → Profile fields
│   │   └── FilterFileWriter.swift       # Write temp filter file from ignore patterns, cleanup
│   │
│   ├── Views/
│   │   ├── TrayPopup/
│   │   │   ├── TrayPopupPanel.swift     # NSWindow subclass: borderless, vibrancy, positioning
│   │   │   ├── TrayPopupView.swift      # Root SwiftUI view: header, profile list, footer
│   │   │   ├── PopupProfileRow.swift    # Profile row: badge + name + metadata + action button
│   │   │   └── PopupEmptyState.swift    # "Create your first profile" CTA
│   │   │
│   │   ├── MainWindow/
│   │   │   ├── MainWindowView.swift     # TabView: Profiles | History | Settings
│   │   │   ├── Profiles/
│   │   │   │   ├── ProfileListView.swift    # Profile list with action buttons
│   │   │   │   ├── ProfileFormView.swift    # Create/edit form with all fields
│   │   │   │   ├── ActionSelectorView.swift # Segmented picker with action descriptions
│   │   │   │   └── PasteCommandView.swift   # Paste rclone command input + parse trigger
│   │   │   ├── History/
│   │   │   │   ├── HistoryTabView.swift      # Profile dropdown + run list + start/cancel
│   │   │   │   ├── HistoryRunRow.swift       # Single run entry: badge + date + duration + status
│   │   │   │   ├── LogViewerSheet.swift      # Modal sheet: syntax-highlighted log output
│   │   │   │   └── LiveLogView.swift         # Streaming log output for running jobs
│   │   │   └── Settings/
│   │   │       └── SettingsTabView.swift     # rclone path, version, config dir, theme picker
│   │   │
│   │   └── Components/
│   │       ├── StatusBadge.swift         # Color + SF Symbol for JobStatus
│   │       ├── GUIProfileRow.swift       # Full-detail profile row for main GUI
│   │       └── CronBuilderView.swift    # Visual cron builder + raw input + summary
│   │
│   └── Utilities/
│       ├── CronParser.swift             # Cron expression → next fire date evaluation
│       ├── AtomicFileWriter.swift       # Write-to-temp + atomic rename
│       ├── NetworkMonitor.swift         # NWPathMonitor wrapper for connectivity check
│       ├── CirrusError.swift            # Typed error enum with LocalizedError
│       └── JSONCoders.swift             # JSONEncoder.cirrus / JSONDecoder.cirrus extensions
│
├── CirrusTests/
│   ├── Stores/
│   │   ├── ProfileStoreTests.swift      # CRUD, persistence, atomic writes
│   │   ├── JobManagerTests.swift        # Process lifecycle, cancellation, cleanup
│   │   └── LogStoreTests.swift          # Index management, file consistency
│   ├── Services/
│   │   ├── RcloneServiceTests.swift     # Command assembly correctness
│   │   ├── RcloneCommandParserTests.swift # Parse accuracy for various command formats
│   │   └── FilterFileWriterTests.swift  # Filter file generation
│   ├── Utilities/
│   │   ├── CronParserTests.swift        # Cron evaluation edge cases, parameterized
│   │   └── AtomicFileWriterTests.swift  # Atomic write correctness
│   └── Models/
│       └── ProfileTests.swift           # Codable round-trip, validation
│
└── README.md                            # Setup, build, test, debug instructions
```

### Architectural Boundaries

**Layer Boundaries (strict dependency direction: Views → Stores → Services → Utilities):**

```
┌─────────────────────────────────────────────┐
│  Views (SwiftUI)                            │
│  TrayPopup / MainWindow / Components        │
│  ↓ reads state via @Environment             │
│  ↓ calls methods on stores                  │
├─────────────────────────────────────────────┤
│  Stores (@Observable managers)              │
│  ProfileStore / JobManager / LogStore /      │
│  ScheduleManager / AppSettings              │
│  ↓ uses services for rclone interaction     │
│  ↓ uses utilities for file I/O, parsing     │
├─────────────────────────────────────────────┤
│  Services (stateless logic)                 │
│  RcloneService / RcloneCommandParser /       │
│  FilterFileWriter                           │
│  ↓ uses utilities                           │
├─────────────────────────────────────────────┤
│  Utilities (pure functions, no state)       │
│  CronParser / AtomicFileWriter /            │
│  NetworkMonitor / CirrusError / JSONCoders  │
├─────────────────────────────────────────────┤
│  Models (data structs, no behavior)         │
│  Profile / JobRun / LogEntry / JobStatus /   │
│  CronSchedule / AppSettingsModel            │
└─────────────────────────────────────────────┘
```

**Rules:**
- Views never import Services or Utilities directly — they go through Stores
- Stores never import Views — they are view-agnostic
- Services are stateless — they receive inputs and return outputs
- Models are shared across all layers — they are pure data
- No circular dependencies between layers

**External Integration Boundary:**

```
┌──────────────┐     ┌───────────────────┐     ┌────────────┐
│  JobManager  │ ──→ │  RcloneService    │ ──→ │  rclone    │
│  (store)     │     │  (command builder) │     │  (CLI)     │
│              │ ←── │                   │ ←── │            │
│  stdout/err  │     │  Process + Pipe   │     │  output    │
└──────────────┘     └───────────────────┘     └────────────┘
```

The `RcloneService` is the sole boundary with the external `rclone` CLI. No other component spawns processes or interacts with rclone directly.

### Requirements to Structure Mapping

| FR Category | Primary Files | Test Files |
|-------------|--------------|------------|
| rclone Setup & Config (FR1-7) | `RcloneService.swift`, `AppSettings.swift`, `SettingsTabView.swift` | `RcloneServiceTests.swift` |
| Profile Management (FR8-19) | `Profile.swift`, `ProfileStore.swift`, `ProfileFormView.swift`, `PasteCommandView.swift`, `RcloneCommandParser.swift` | `ProfileStoreTests.swift`, `RcloneCommandParserTests.swift`, `ProfileTests.swift` |
| Job Execution (FR20-28) | `JobManager.swift`, `JobRun.swift`, `FilterFileWriter.swift`, `NetworkMonitor.swift` | `JobManagerTests.swift`, `FilterFileWriterTests.swift` |
| Scheduling (FR29-34) | `ScheduleManager.swift`, `CronSchedule.swift`, `CronParser.swift`, `CronBuilderView.swift` | `CronParserTests.swift` |
| Logging & History (FR35-42) | `LogStore.swift`, `LogEntry.swift`, `HistoryTabView.swift`, `LogViewerSheet.swift`, `LiveLogView.swift` | `LogStoreTests.swift` |
| Tray Popup (FR43-52) | `TrayPopupPanel.swift`, `TrayPopupView.swift`, `PopupProfileRow.swift`, `PopupEmptyState.swift` | — (UI, tested manually) |
| Main GUI (FR53-58) | `MainWindowView.swift`, `ProfileListView.swift`, `HistoryTabView.swift`, `GUIProfileRow.swift` | — (UI, tested manually) |
| App Lifecycle (FR59-62) | `CirrusApp.swift`, `AppDelegate.swift` | — |

### Data Flow

```
User clicks Start
        │
        ▼
PopupProfileRow → JobManager.startJob(profileId)
        │
        ├─→ ProfileStore.profile(for: id)     → Profile struct (snapshot)
        ├─→ RcloneService.buildCommand(profile) → [String] args
        ├─→ FilterFileWriter.write(patterns)    → temp file URL
        ├─→ Process.launch()                    → pid tracked
        │
        ├─→ Pipe.readabilityHandler             → chunks on background queue
        │       │
        │       ▼
        │   LogStore.appendChunk(chunk)          → writes to .log file
        │       │                                → updates @Observable buffer
        │       ▼
        │   SwiftUI observes LogStore            → LiveLogView updates
        │
        ▼
Process.terminationHandler
        │
        ├─→ LogStore.finalizeRun()              → writes LogEntry to index.json
        ├─→ JobManager removes from active dict → JobStatus updates
        ├─→ FilterFileWriter.cleanup()          → deletes temp file
        │
        ▼
SwiftUI observes JobManager                    → StatusBadge updates across all views
```

### Development Workflow

**Build:** `⌘B` in Xcode or `xcodebuild -scheme Cirrus -configuration Debug build`

**Test:** `⌘U` in Xcode or `xcodebuild test -scheme Cirrus`

**Run:** `⌘R` in Xcode. App appears as menu bar icon. No Dock icon (`LSUIElement = true` in Info.plist).

**Distribution:** Archive → Export with Developer ID signing → distribute `.dmg` or `.zip` via GitHub Releases.

## Architecture Validation Results

### Coherence Validation ✅

**Decision Compatibility:**
All technology choices are internally consistent. macOS 14+ (Sonoma) deployment target enables `@Observable` macro, which is the foundation for the state management pattern. Swift Testing requires Swift 6 / Xcode 16, which ships with macOS 14+ SDKs. Custom `NSStatusItem + NSWindow` approach works cleanly with `NSHostingView` to embed SwiftUI. No App Sandbox removal is consistent with arbitrary filesystem access and `Process` spawning. No version conflicts detected.

**Pattern Consistency:**
Naming conventions follow Apple's Swift API Design Guidelines throughout — types PascalCase, properties/methods camelCase, enums camelCase. The `@MainActor @Observable` pattern is applied uniformly across all 5 manager classes. Error handling flows through a single `CirrusError` enum everywhere. JSON serialization uses shared `JSONEncoder.cirrus` / `JSONDecoder.cirrus` extensions — no one-off coders. The 10 enforcement guidelines are internally consistent with all pattern definitions.

**Structure Alignment:**
The 5-layer architecture (Views → Stores → Services → Utilities → Models) cleanly supports all decisions. The directory tree matches the architectural layer diagram. `RcloneService` is the sole external integration boundary, and only `JobManager` calls it — no leaky abstractions. File paths derive from `AppSettings.configDirectoryURL`, not hardcoded strings. The test structure mirrors the source structure as specified.

### Requirements Coverage Validation ✅

**Functional Requirements Coverage (62 FRs):**

| FR Category | FRs | Architectural Support |
|---|---|---|
| rclone Setup & Config | FR1-7 | `RcloneService` (PATH detection, version check, `listremotes`), `AppSettings` (rclone path, config dir), `SettingsTabView` |
| Profile Management | FR8-19 | `Profile` model, `ProfileStore` (CRUD, persistence), `ProfileFormView`, `PasteCommandView`, `RcloneCommandParser`, `ActionSelectorView` |
| Job Execution | FR20-28 | `JobManager` (process lifecycle, config snapshot, concurrent execution, cleanup), `FilterFileWriter`, `NetworkMonitor` |
| Scheduling | FR29-34 | `ScheduleManager` (cron evaluation, timer management), `CronParser`, `CronSchedule` model, `CronBuilderView` |
| Logging & History | FR35-42 | `LogStore` (index CRUD, streaming buffer), `LogEntry` model, `HistoryTabView`, `LogViewerSheet`, `LiveLogView` |
| Tray Popup | FR43-52 | `TrayPopupPanel` (NSWindow), `TrayPopupView`, `PopupProfileRow`, `PopupEmptyState`, `StatusBadge` |
| Main GUI | FR53-58 | `MainWindowView` (TabView), `ProfileListView`, `GUIProfileRow`, `HistoryTabView` (profile dropdown) |
| App Lifecycle | FR59-62 | `CirrusApp` (@main, Login Item), `AppDelegate` (NSStatusItem, window management) |

All 62 FRs have architectural support. No gaps in functional coverage.

**Non-Functional Requirements Coverage (21 NFRs):**

| NFR | Requirement | Architectural Support |
|---|---|---|
| NFR1 | Popup < 200ms | Custom NSWindow pre-created, SwiftUI observes cached state |
| NFR2 | Profile list < 300ms with 50+ profiles | SwiftUI `List` with `@Observable` — lazy rendering |
| NFR3 | Log streaming < 100ms latency | `readabilityHandler` (GCD) → `@MainActor` dispatch → `@Observable` buffer |
| NFR4 | Status badge updates < 1s | `terminationHandler` → `@Observable` `JobManager` |
| NFR5 | Paste parse < 1s | `RcloneCommandParser` — pure function, no I/O |
| NFR6 | Memory < 100MB idle | Native app, no web runtime, file-backed persistence |
| NFR7 | UI responsive during concurrent jobs | `@MainActor` stores, background pipe reading |
| NFR8 | No silent failures | `CirrusError` enum, every execution creates `LogEntry` |
| NFR9 | Schedule accuracy < 5s | In-app timer with cron evaluation loop |
| NFR10 | No orphaned processes | `JobManager` SIGTERM/SIGKILL on quit, active jobs dictionary |
| NFR11 | Atomic profile writes | `AtomicFileWriter` (write-to-temp + rename) |
| NFR12 | Log index consistency | `LogStore` manages index + raw files together |
| NFR13 | rclone 1.60+ support | `RcloneService` version check |
| NFR14 | UTF-8 output handling | Foundation `String` — native UTF-8 |
| NFR15 | Exit code handling | `CirrusError.rcloneExecutionFailed(exitCode:stderr:)` |
| NFR16 | Valid filter syntax | `FilterFileWriter` generates rclone filter files |
| NFR17 | VoiceOver accessible | SwiftUI built-in + `.accessibilityLabel()` enforcement guideline |
| NFR18 | Color independence | `StatusBadge` uses color + SF Symbol shape |
| NFR19 | macOS HIG compliance | Native SwiftUI, no third-party UI frameworks |
| NFR20 | Destructive action confirmation | View patterns: `.alert(isPresented:)` / `.sheet(item:)` |
| NFR21 | User-facing error messages | `CirrusError: LocalizedError` with `errorDescription` |

All 21 NFRs have architectural support. No gaps.

### Implementation Readiness Validation ✅

**Decision Completeness:**
All critical technology decisions documented with specific versions (macOS 14+, Swift 6, rclone 1.60+). Implementation patterns cover 12 conflict areas with concrete code examples. 10 enforcement guidelines are specific and enforceable. Examples provided for `@Observable` pattern, JSON coders, error enum, and data model.

**Structure Completeness:**
Complete directory tree lists ~40 source files and ~10 test files. Every file has a 1-line purpose description. Layer boundaries are explicit with dependency rules. Integration boundary diagram shows the exact RcloneService → rclone flow.

**Pattern Completeness:**
Naming conventions cover types, functions, properties, enums, files, tests, and booleans. Anti-patterns listed for each category. Concurrency pattern table covers every threading scenario. Inter-manager communication pattern defined (direct calls, no NotificationCenter).

### Gap Analysis Results

**Critical Gaps:** None found. All implementation-blocking decisions are documented.

**Important Gaps (addressable, not blocking):**

1. **FR3 — rclone auto-install mechanism:** The architecture references rclone discovery and PATH detection in `RcloneService`, but doesn't specify the download/install flow details (URL, verification, platform binary selection). Resolvable during story development.

2. **PRD deployment target discrepancy:** The PRD still says "macOS minimum version: Ventura+" while the architecture specifies macOS 14+ (Sonoma). Not an architecture gap — PRD should be updated for consistency.

**Nice-to-Have Gaps:**

1. **No explicit app diagnostics logging:** `os.Logger` is mentioned in the starter template for app diagnostics but isn't reflected in the project structure or patterns. A logging utility for the app's own debug output (distinct from rclone log capture) would help with debugging.

2. **No explicit test fixture strategy:** Test patterns define structure and assertions but don't specify how test fixtures (sample profiles, mock rclone output) are organized.

### Architecture Completeness Checklist

**✅ Requirements Analysis**

- [x] Project context thoroughly analyzed (62 FRs, 21 NFRs, 8 categories)
- [x] Scale and complexity assessed (low — thin orchestration layer)
- [x] Technical constraints identified (no sandbox, no backend, rclone external)
- [x] Cross-cutting concerns mapped (5 concerns)

**✅ Architectural Decisions**

- [x] Critical decisions documented with versions (macOS 14+, Swift 6, rclone 1.60+)
- [x] Technology stack fully specified (Swift + SwiftUI, Xcode, SPM, Swift Testing)
- [x] Integration patterns defined (RcloneService as sole external boundary)
- [x] Performance considerations addressed (async patterns, @MainActor, background I/O)

**✅ Implementation Patterns**

- [x] Naming conventions established (12 categories, anti-patterns listed)
- [x] Structure patterns defined (5-layer architecture, dependency rules)
- [x] Communication patterns specified (direct method calls, no NotificationCenter)
- [x] Process patterns documented (error handling, concurrency, JSON encoding)

**✅ Project Structure**

- [x] Complete directory structure defined (~40 source files, ~10 test files)
- [x] Component boundaries established (Views → Stores → Services → Utilities → Models)
- [x] Integration points mapped (JobManager → RcloneService → rclone CLI)
- [x] Requirements to structure mapping complete (8 FR categories → files → tests)

### Architecture Readiness Assessment

**Overall Status:** READY FOR IMPLEMENTATION

**Confidence Level:** High — all 83 requirements (62 FR + 21 NFR) have explicit architectural support, all decisions are coherent, and patterns are comprehensive enough for AI agents to implement consistently.

**Key Strengths:**
- Clean 5-layer architecture with strict dependency direction
- Single `@Observable` pattern across all state management — no mixed paradigms
- Sole external integration boundary (`RcloneService`) isolates all rclone interaction
- Comprehensive enforcement guidelines prevent common AI agent inconsistencies
- Value-type `Profile` struct provides config snapshot for free (no deep copy needed)

**Areas for Future Enhancement:**
- App diagnostics logging (`os.Logger` integration)
- Test fixture organization
- rclone auto-install implementation details

### Implementation Handoff

**AI Agent Guidelines:**

- Follow all architectural decisions exactly as documented
- Use implementation patterns consistently across all components
- Respect project structure and layer boundaries
- Refer to this document for all architectural questions
- Follow all 10 enforcement guidelines without exception

**First Implementation Priority:**
Create Xcode project with the specified template configuration, then implement data models (`Profile`, `JobRun`, `LogEntry`, `JobStatus`, `CronSchedule`, `AppSettingsModel`) followed by `AtomicFileWriter` and `ProfileStore`.
