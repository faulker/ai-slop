# Story 2.1: rclone Discovery & App Settings

Status: ready-for-dev

## Story

As a user,
I want Cirrus to find my rclone installation automatically and let me configure app settings,
so that I can get started quickly without manual path configuration.

## Acceptance Criteria

1. **Given** the app launches for the first time
   **When** rclone is installed and available in the system PATH
   **Then** the app auto-detects the rclone binary path (FR2)
   **And** stores the resolved path in AppSettings

2. **Given** rclone is not found in PATH
   **When** the user opens the Settings tab
   **Then** the user can manually locate the rclone binary via a file picker (FR1)
   **And** the user is offered the option to download and install rclone to `~/.local/bin` (FR3)

3. **Given** rclone is configured
   **When** the user views the Settings tab
   **Then** the installed rclone version is displayed (FR4)
   **And** the current rclone binary path is shown and editable (FR1)
   **And** the config storage location is shown with an option to change it (FR5)

4. **Given** the user changes the config storage location
   **When** the new path is confirmed
   **Then** AppSettings persists the new location
   **And** all subsequent profile and log operations use the new directory

## Tasks / Subtasks

- [ ] Task 1: Implement RcloneService (AC: #1, #3)
  - [ ] 1.1: Create `Services/RcloneService.swift`
  - [ ] 1.2: Implement `detectRclone()` — search PATH using `Process("/usr/bin/which", ["rclone"])` or iterate PATH entries
  - [ ] 1.3: Implement `version(at path:)` — run `rclone version` and parse first line
  - [ ] 1.4: Implement `downloadAndInstall()` — download rclone binary to `~/.local/bin/rclone` (FR3)
  - [ ] 1.5: All methods throw `CirrusError` on failure (`.rcloneNotFound`, `.rcloneExecutionFailed`)
- [ ] Task 2: Enhance AppSettings for rclone (AC: #1, #4)
  - [ ] 2.1: Add rclone path auto-detection on first launch via `RcloneService.detectRclone()`
  - [ ] 2.2: Store detected path in `AppSettingsModel.rclonePath`
  - [ ] 2.3: Expose computed `rcloneVersion` property that caches version string
- [ ] Task 3: Build SettingsTabView (AC: #2, #3, #4)
  - [ ] 3.1: Implement `Views/MainWindow/Settings/SettingsTabView.swift` (replace placeholder from Story 1.3)
  - [ ] 3.2: rclone section: path display, "Browse" file picker, "Download rclone" button, version display
  - [ ] 3.3: Storage section: config directory path, "Change" folder picker
  - [ ] 3.4: Login Item toggle (already scaffolded in Story 1.3)
  - [ ] 3.5: File pickers use `NSOpenPanel` wrapped for SwiftUI
- [ ] Task 4: Config directory management (AC: #4)
  - [ ] 4.1: When config directory changes, create new directory structure (profiles/, logs/runs/)
  - [ ] 4.2: Do NOT migrate files — just point to new location (migration is out of MVP scope)
  - [ ] 4.3: Persist new path immediately via AppSettings.save()
- [ ] Task 5: Write tests
  - [ ] 5.1: `CirrusTests/Services/RcloneServiceTests.swift` — test PATH detection logic, version parsing
  - [ ] 5.2: Test version string parsing handles various rclone version output formats
  - [ ] 5.3: Test `AppSettings` rclone path persistence round-trip

## Dev Notes

### Architecture Compliance

**Layer:** Services (`RcloneService`) + Views (Settings) + Stores (`AppSettings` enhancement).

**File locations:**
```
Cirrus/Cirrus/
├── Services/
│   └── RcloneService.swift              # NEW
├── Stores/
│   └── AppSettings.swift                # MODIFY — add rclone detection
└── Views/MainWindow/Settings/
    └── SettingsTabView.swift            # MODIFY — replace placeholder
CirrusTests/Services/
    └── RcloneServiceTests.swift         # NEW
```

### Technical Requirements

**RcloneService — stateless service, no state storage:**
```swift
struct RcloneService {
    static func detectRclone() throws -> String { /* returns resolved path */ }
    static func version(at path: String) async throws -> String { /* returns version string */ }
    static func downloadAndInstall() async throws -> String { /* returns installed path */ }
}
```
- `detectRclone()`: Use `Process` to run `/usr/bin/which rclone`. Parse stdout for path. If not found, check common locations: `/usr/local/bin/rclone`, `/opt/homebrew/bin/rclone`, `~/.local/bin/rclone`
- `version(at:)`: Run `{path} version`, parse first line (format: "rclone v1.67.0")
- `downloadAndInstall()`: Download from `https://downloads.rclone.org/rclone-current-osx-arm64.zip` (or amd64), unzip, copy binary to `~/.local/bin/rclone`, `chmod +x`. Detect architecture via `ProcessInfo.processInfo.machineArchitecture` or `#if arch(arm64)`
- `RcloneService` is the SOLE boundary for rclone interaction. No other component spawns rclone processes.

**NSOpenPanel for file/folder pickers:**
```swift
func selectFile() -> URL? {
    let panel = NSOpenPanel()
    panel.canChooseFiles = true
    panel.canChooseDirectories = false
    panel.allowedContentTypes = [.unixExecutable]
    return panel.runModal() == .OK ? panel.url : nil
}
```

**rclone 1.60+ compatibility (NFR13):** Version check should warn if detected version is below 1.60.

### Enforcement Rules

- `RcloneService` is stateless — all state lives in `AppSettings`
- Use `Process` API for shell execution — never `system()` or `popen()`
- Never hardcode rclone path — always read from `AppSettings.settings.rclonePath`
- All errors throw `CirrusError` — no silent failures (NFR8)
- File pickers use `NSOpenPanel` — not custom UI

### Dependencies

- **Depends on:** Story 1.1 (AppSettings, CirrusError, AtomicFileWriter), Story 1.3 (SettingsTabView placeholder)
- **Does NOT depend on:** Stories 2.2-2.5 or Epics 3-5

### References

- [Source: architecture.md#Core Architectural Decisions] — RcloneService as sole external boundary
- [Source: architecture.md#Project Structure & Boundaries] — Services layer
- [Source: architecture.md#Error Handling Strategy] — CirrusError cases
- [Source: epics.md#Story 2.1] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
