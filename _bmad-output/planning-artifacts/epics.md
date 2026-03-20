---
stepsCompleted: [1, 2, 3, 4]
status: 'complete'
completedAt: '2026-02-27'
inputDocuments: ['_bmad-output/planning-artifacts/prd.md', '_bmad-output/planning-artifacts/architecture.md', '_bmad-output/planning-artifacts/ux-design-specification.md']
---

# Cirrus - Epic Breakdown

## Overview

This document provides the complete epic and story breakdown for Cirrus, decomposing the requirements from the PRD, UX Design, and Architecture into implementable stories.

## Requirements Inventory

### Functional Requirements

FR1: User can configure the path to the rclone executable
FR2: App can automatically detect rclone in the system PATH on launch
FR3: User can download and install rclone to `~/.local/bin` from within the app
FR4: User can view the installed rclone version in settings
FR5: User can configure the storage location for profiles and app settings
FR6: App can discover configured rclone remotes via `rclone listremotes`
FR7: User can manually add remote names that aren't auto-discovered
FR8: User can create a new profile by filling out a manual form (source, destination, action, ignore patterns, flags)
FR9: User can create a new profile by pasting an rclone command that is parsed into form fields
FR10: User can select a local source folder using a native folder picker
FR11: User can select a destination remote from a dropdown of discovered remotes
FR12: User can specify a path on the destination remote via text input
FR13: User can select an rclone action (sync, copy, move, delete) with descriptions of each action's behavior
FR14: User can add, edit, and remove one or more ignore patterns per profile
FR15: User can configure common rclone flags (e.g., --dry-run, --verbose) per profile
FR16: User can edit an existing profile's configuration
FR17: User can delete a profile
FR18: User can execute a dry-run ("Test") during profile creation or editing to preview what would happen
FR19: App warns the user when editing a profile that has a currently running job
FR20: User can start a profile's rclone job from the tray popup
FR21: User can start a profile's rclone job from the main GUI profile list
FR22: User can cancel a running job from the tray popup
FR23: User can cancel a running job from the main GUI
FR24: App can execute multiple jobs concurrently with no enforced limit
FR25: App snapshots the profile configuration at job start time so mid-run edits do not affect the running job
FR26: App assembles rclone commands using direct args for flags and filter files for ignore patterns
FR27: App prevents job execution when no network connection is detected
FR28: App tracks all running rclone child processes and cleans them up on app quit
FR29: User can assign a cron-based schedule to a profile
FR30: User can define schedules using a visual cron builder UI
FR31: User can define schedules by entering a raw cron expression
FR32: User can remove a schedule from a profile (on-demand only)
FR33: App executes scheduled jobs automatically when the app is running
FR34: App warns the user on quit that scheduled jobs will stop running
FR35: App captures complete stdout and stderr from every rclone execution
FR36: App stores a JSON log index with metadata per execution (profile, timestamp, status, duration)
FR37: App stores raw log output as individual files per execution
FR38: User can view per-profile run history sorted by most recent first
FR39: User can see the status of each historical run (successful, failed, canceled, interrupted)
FR40: User can open a log viewer that displays the raw output of any historical run
FR41: Log viewer highlights error lines with red background and warning lines with yellow background
FR42: User can view live-streaming log output for currently running jobs
FR43: App displays a persistent menu bar icon
FR44: User can click the menu bar icon to open a custom popup
FR45: Tray popup displays all configured profiles with status badges (green/red/yellow)
FR46: Tray popup displays last successful run date/time for idle profiles
FR47: Tray popup displays elapsed run time for currently running profiles
FR48: Tray popup provides a Start button per idle profile
FR49: Tray popup provides a Cancel button per running profile
FR50: Tray popup provides a History link per profile that opens the history tab
FR51: Tray popup displays a "Create your first profile" button when no profiles exist
FR52: User can open the main GUI from the tray popup
FR53: Main GUI displays a profile list showing name, source, destination, last run status/time, next scheduled run, and Start/Cancel button
FR54: User can navigate to a History tab from the main GUI
FR55: History tab provides a profile dropdown with status indicators (green/red/yellow) next to each profile name
FR56: User can switch between profiles in the history tab via the dropdown
FR57: User can start or cancel a job from the history tab
FR58: User can view live log output for a running job from the history tab
FR59: App registers as a Login Item to launch at system startup
FR60: Closing the main GUI window keeps the app running in the menu bar
FR61: User can fully quit the app via an explicit Quit button
FR62: App starts silently in the menu bar tray on launch

### NonFunctional Requirements

NFR1: Tray popup opens within 200ms of clicking the menu bar icon, regardless of profile count
NFR2: Profile list in the main GUI renders within 300ms with 50+ profiles
NFR3: Live log streaming updates the UI within 100ms of rclone output
NFR4: Job status badge updates within 1 second of job completion or failure
NFR5: Profile creation from a pasted rclone command parses and populates fields within 1 second
NFR6: The app's idle memory footprint remains under 100MB with no running jobs
NFR7: UI remains responsive (no frame drops or hangs) while multiple jobs execute concurrently
NFR8: Every rclone execution produces a complete log entry — no silent failures, no missing logs
NFR9: Scheduled jobs fire within 5 seconds of their scheduled time when the app is running
NFR10: No orphaned rclone child processes remain after app quit or crash
NFR11: Profile JSON files are written atomically to prevent corruption from crashes or power loss
NFR12: The JSON log index remains consistent with raw log files — no phantom entries or missing files
NFR13: App supports rclone versions 1.60+ (current stable and recent releases)
NFR14: App handles rclone output encoding correctly (UTF-8 stdout/stderr)
NFR15: App gracefully handles unexpected rclone exit codes with appropriate status mapping
NFR16: Filter files generated for --filter-from are valid rclone filter syntax
NFR17: All interactive elements are accessible via macOS VoiceOver (leveraging SwiftUI's built-in accessibility)
NFR18: All status indicators use both color and icon/shape (not color-only) for color-blind users
NFR19: The tray popup and main GUI follow macOS Human Interface Guidelines for native look and feel
NFR20: All destructive actions (delete profile, cancel running job, quit app) require confirmation
NFR21: Error messages are user-facing and actionable — no raw stack traces or internal error codes exposed to users

### Additional Requirements

**From Architecture:**
- Starter template: Xcode macOS App template with SwiftUI, Swift Testing, deployment target macOS 14.0 (Sonoma) — this is the first implementation story
- Disable App Sandbox entitlement (required for filesystem access and process spawning)
- Add Login Items capability for launch-at-startup
- Set `LSUIElement = true` in Info.plist (hide Dock icon)
- Configure Developer ID signing for outside-App-Store distribution
- Custom `NSStatusItem + NSWindow + NSHostingView` approach for tray popup (not MenuBarExtra)
- 5 `@MainActor @Observable` manager classes: ProfileStore, JobManager, ScheduleManager, LogStore, AppSettings
- `AtomicFileWriter` utility for crash-safe file persistence (write-to-temp + rename)
- Shared `JSONEncoder.cirrus` / `JSONDecoder.cirrus` extensions for all serialization
- `CirrusError` enum with `LocalizedError` conformance for typed error handling
- `RcloneService` as sole external integration boundary — no other component spawns processes
- Process management: SIGTERM → wait → SIGKILL sequence on app quit
- Config snapshot via value-type Profile struct copy at job start
- File storage layout: `~/.config/cirrus/` with `profiles/`, `logs/runs/`, `settings.json`, `logs/index.json`
- `NetworkMonitor` (NWPathMonitor wrapper) for connectivity checking before job execution

**From UX Design:**
- VoiceOver: All custom views annotated with `.accessibilityLabel` and `.accessibilityValue`
- Keyboard navigation: Tab through interactive elements, Enter to activate, Escape to dismiss popups
- Color independence: Status badges use color + SF Symbol shape (checkmark/xmark/clock/circle)
- Reduced motion: Respect `accessibilityReduceMotion` — disable badge animations, highlight flash, auto-scroll
- Font scaling: Respect macOS Dynamic Type / text size accessibility settings
- Empty state UX: "Create your first profile" CTA on first launch (not a blank screen)
- Status badge component: Reusable across tray popup, profile list, and history dropdown
- Error empty states: Clear messaging with action buttons for error recovery
- Animation standards: SwiftUI `.default` curve, 0.2s for state transitions, 0.3s for navigation transitions
- Tray popup: `NSWindow` with `.accessibilityRole(.popover)` for VoiceOver announcement

### FR Coverage Map

FR1: Epic 2 - Configure rclone executable path
FR2: Epic 2 - Auto-detect rclone in PATH
FR3: Epic 2 - Download and install rclone
FR4: Epic 2 - View rclone version in settings
FR5: Epic 2 - Configure storage location
FR6: Epic 2 - Discover rclone remotes
FR7: Epic 2 - Manually add remote names
FR8: Epic 2 - Create profile via manual form
FR9: Epic 2 - Create profile via paste command
FR10: Epic 2 - Select source folder with picker
FR11: Epic 2 - Select destination remote from dropdown
FR12: Epic 2 - Specify remote path via text input
FR13: Epic 2 - Select rclone action with descriptions
FR14: Epic 2 - Manage ignore patterns
FR15: Epic 2 - Configure rclone flags
FR16: Epic 2 - Edit existing profile
FR17: Epic 2 - Delete profile
FR18: Epic 2 - Execute dry-run Test
FR19: Epic 2 - Warn when editing running profile
FR20: Epic 3 - Start job from tray popup
FR21: Epic 3 - Start job from main GUI
FR22: Epic 3 - Cancel job from tray popup
FR23: Epic 3 - Cancel job from main GUI
FR24: Epic 3 - Concurrent job execution
FR25: Epic 3 - Config snapshot at job start
FR26: Epic 3 - Command assembly with filter files
FR27: Epic 3 - Network check before execution
FR28: Epic 3 - Process cleanup on quit
FR29: Epic 5 - Assign cron schedule to profile
FR30: Epic 5 - Visual cron builder UI
FR31: Epic 5 - Raw cron expression input
FR32: Epic 5 - Remove schedule from profile
FR33: Epic 5 - Automatic scheduled execution
FR34: Epic 5 - Quit warning for scheduled jobs
FR35: Epic 3 - Capture stdout/stderr
FR36: Epic 3 - JSON log index per execution
FR37: Epic 3 - Raw log files per execution
FR38: Epic 4 - Per-profile run history
FR39: Epic 4 - Historical run status display
FR40: Epic 4 - Log viewer for historical runs
FR41: Epic 4 - Syntax-highlighted log viewer
FR42: Epic 3 - Live log streaming
FR43: Epic 1 - Persistent menu bar icon
FR44: Epic 3 - Click menu bar to open popup
FR45: Epic 3 - Profile status badges in popup
FR46: Epic 3 - Last run date/time in popup
FR47: Epic 3 - Elapsed time for running jobs
FR48: Epic 3 - Start button per idle profile
FR49: Epic 3 - Cancel button per running profile
FR50: Epic 3 - History link per profile
FR51: Epic 3 - Empty state CTA in popup
FR52: Epic 1 - Open main GUI from tray
FR53: Epic 3 - Main GUI profile list with controls
FR54: Epic 4 - History tab navigation
FR55: Epic 4 - Profile dropdown with status indicators
FR56: Epic 4 - Switch profiles in history dropdown
FR57: Epic 4 - Start/cancel from history tab
FR58: Epic 4 - Live log on history tab
FR59: Epic 1 - Login Item registration
FR60: Epic 1 - Close window keeps app running
FR61: Epic 1 - Quit via explicit button
FR62: Epic 1 - Silent menu bar launch

## Epic List

### Epic 1: App Foundation & Menu Bar Shell
Users can install and run Cirrus as a persistent menu bar application with proper window lifecycle management. The app launches silently in the menu bar on startup, users can open/close the main window, and properly quit the app.
**FRs covered:** FR43, FR52, FR59, FR60, FR61, FR62

### Epic 2: rclone Integration & Profile Management
Users can set up rclone (auto-detect, manual locate, or install), create sync profiles via manual form or paste-to-create, edit them, delete them, and test with dry-run. Settings for rclone path, version display, and config location are available.
**FRs covered:** FR1-FR19

### Epic 3: Job Execution & Tray Dashboard
Users can start and cancel sync jobs from both the tray popup and main GUI, see real-time status badges, elapsed time, and live log streaming. Network detection prevents offline execution. Process cleanup on quit prevents orphaned processes.
**FRs covered:** FR20-FR28, FR35-FR37, FR42, FR44-FR51, FR53

### Epic 4: History & Log Viewer
Users can review per-profile run history, see status of each historical run, open a syntax-highlighted log viewer for any past execution, and start/cancel jobs directly from the history tab.
**FRs covered:** FR38-FR41, FR54-FR58

### Epic 5: Scheduling
Users can automate sync jobs with cron-based schedules using a visual cron builder or raw cron expressions. Quit warning reminds users that scheduled jobs need the app running.
**FRs covered:** FR29-FR34

---

## Epic 1: App Foundation & Menu Bar Shell

Users can install and run Cirrus as a persistent menu bar application with proper window lifecycle management. The app launches silently in the menu bar on startup, users can open/close the main window, and properly quit the app.

### Story 1.1: Xcode Project Initialization & Core Utilities

As a developer,
I want the Cirrus Xcode project created with proper configuration and core utilities,
So that all subsequent stories have a solid foundation to build on.

**Acceptance Criteria:**

**Given** the developer creates a new macOS App project in Xcode
**When** the project is configured with SwiftUI, Swift Testing, and bundle ID `com.sane.cirrus`
**Then** the project compiles and runs with deployment target macOS 14.0 (Sonoma)
**And** App Sandbox entitlement is disabled
**And** `LSUIElement` is set to `true` in Info.plist
**And** Login Items capability is added
**And** Developer ID signing is configured

**Given** the project is initialized
**When** the developer inspects the Utilities directory
**Then** `CirrusError` enum exists with all error cases and `LocalizedError` conformance
**And** `JSONEncoder.cirrus` and `JSONDecoder.cirrus` extensions exist with ISO 8601 dates, prettyPrinted, sortedKeys
**And** `AtomicFileWriter` exists with write-to-temp + atomic rename via `FileManager.replaceItem(at:withItemAt:)`

**Given** the project is initialized
**When** the developer inspects Models and Stores
**Then** `AppSettingsModel` struct exists with `Codable` conformance (rclone path, config directory, theme)
**And** `AppSettings` `@MainActor @Observable` class exists with load/save and `configDirectoryURL` provider
**And** default config directory is `~/.config/cirrus/`

### Story 1.2: Menu Bar Icon & Tray Popup Shell

As a user,
I want to see a Cirrus icon in my menu bar that opens a popup when clicked,
So that I can access the app quickly from anywhere on my Mac.

**Acceptance Criteria:**

**Given** the app is launched
**When** the app finishes starting
**Then** a grayscale template image appears in the macOS menu bar (FR43)
**And** no Dock icon is visible (FR62)

**Given** the menu bar icon is visible
**When** the user clicks the menu bar icon
**Then** a borderless floating popup panel appears anchored below the icon
**And** the popup uses `NSVisualEffectView` for vibrancy/translucency
**And** the popup has `.accessibilityRole(.popover)` for VoiceOver

**Given** the popup is open
**When** the user clicks outside the popup, presses Escape, or re-clicks the menu bar icon
**Then** the popup dismisses

**Given** the popup is open
**When** the user views the popup content
**Then** an "Open Cirrus" button is visible (FR52)
**And** a "Quit" button is visible (FR61)

### Story 1.3: Main Window & App Lifecycle

As a user,
I want to open the full Cirrus window and have the app persist in my menu bar,
So that closing the window doesn't stop my background operations.

**Acceptance Criteria:**

**Given** the tray popup is open
**When** the user clicks "Open Cirrus"
**Then** the main GUI window opens with a TabView containing Profiles, History, and Settings tabs (FR52)
**And** the tray popup dismisses

**Given** the main GUI window is open
**When** the user closes the window (⌘W or red close button)
**Then** the window closes but the app continues running in the menu bar (FR60)
**And** the menu bar icon remains visible and clickable

**Given** the user clicks "Quit" in the tray popup
**When** the quit action is triggered
**Then** a confirmation dialog appears (NFR20)
**And** if confirmed, the app terminates completely (FR61)

**Given** the app is configured as a Login Item
**When** the Mac starts up or the user logs in
**Then** the app launches automatically and appears silently in the menu bar (FR59, FR62)

---

## Epic 2: rclone Integration & Profile Management

Users can set up rclone (auto-detect, manual locate, or install), create sync profiles via manual form or paste-to-create, edit them, delete them, and test with dry-run. Settings for rclone path, version display, and config location are available.

### Story 2.1: rclone Discovery & App Settings

As a user,
I want Cirrus to find my rclone installation automatically and let me configure app settings,
So that I can get started quickly without manual path configuration.

**Acceptance Criteria:**

**Given** the app launches for the first time
**When** rclone is installed and available in the system PATH
**Then** the app auto-detects the rclone binary path (FR2)
**And** stores the resolved path in AppSettings

**Given** rclone is not found in PATH
**When** the user opens the Settings tab
**Then** the user can manually locate the rclone binary via a file picker (FR1)
**And** the user is offered the option to download and install rclone to `~/.local/bin` (FR3)

**Given** rclone is configured
**When** the user views the Settings tab
**Then** the installed rclone version is displayed (FR4)
**And** the current rclone binary path is shown and editable (FR1)
**And** the config storage location is shown with an option to change it (FR5)

**Given** the user changes the config storage location
**When** the new path is confirmed
**Then** AppSettings persists the new location
**And** all subsequent profile and log operations use the new directory

### Story 2.2: Profile Data Model & Persistence

As a developer,
I want the Profile data model and persistence layer implemented,
So that profiles can be created, stored, and retrieved reliably.

**Acceptance Criteria:**

**Given** the Profile model is defined
**When** a Profile struct is created
**Then** it contains all required fields: id (UUID), name, sourcePath, remoteName, remotePath, action (RcloneAction enum), ignorePatterns, extraFlags, schedule (optional), groupName (optional), sortOrder, createdAt, updatedAt
**And** it conforms to `Codable` and `Identifiable`

**Given** the ProfileStore is initialized
**When** profiles are saved
**Then** each profile is written as an individual JSON file in `{configDir}/profiles/{uuid}.json`
**And** writes use `AtomicFileWriter` for crash-safe persistence (NFR11)
**And** JSON encoding uses `JSONEncoder.cirrus` with ISO 8601 dates

**Given** the ProfileStore is initialized
**When** the app launches
**Then** all profiles are loaded from the profiles directory
**And** invalid JSON files are skipped with a logged warning (not a crash)

**Given** a profile is deleted via ProfileStore
**When** the delete completes
**Then** the profile's JSON file is removed from disk
**And** the profile is removed from the in-memory array

### Story 2.3: Remote Discovery & Profile Creation Form

As a user,
I want to create a sync profile by selecting a source folder, destination remote, action, and options,
So that I can configure exactly how my files should be synced.

**Acceptance Criteria:**

**Given** the user opens the profile creation form
**When** the remote dropdown is displayed
**Then** it is populated with remotes discovered via `rclone listremotes` (FR6)
**And** the user can manually type a remote name not in the list (FR7)

**Given** the user is filling out the profile form
**When** they select a source folder
**Then** a native macOS folder picker opens and the selected path populates the source field (FR10)

**Given** the user selects a destination
**When** they pick a remote from the dropdown
**Then** they can specify a path on the remote via a text input field (FR11, FR12)

**Given** the user selects an rclone action
**When** they view the action selector (sync/copy/move/delete)
**Then** each action has a one-line description explaining its behavior and consequences (FR13)

**Given** the user configures ignore patterns
**When** they add, edit, or remove patterns
**Then** the patterns list updates dynamically with add/remove controls (FR14)

**Given** the user configures flags
**When** they enter extra rclone flags (e.g., `--verbose`, `--dry-run`)
**Then** the flags are stored as a string on the profile (FR15)

**Given** all required fields are filled (name, source, remote, action)
**When** the user clicks Save
**Then** the profile is persisted via ProfileStore and appears in the profile list (FR8)

### Story 2.4: Paste-to-Create Profile

As a power user,
I want to paste an rclone command and have it automatically parsed into a profile,
So that I can migrate my existing shell scripts in seconds.

**Acceptance Criteria:**

**Given** the user is on the profile creation screen
**When** they switch to the "Paste Command" input mode
**Then** a text area appears for pasting an rclone command (FR9)

**Given** the user pastes a valid rclone command (e.g., `rclone sync ~/docs gdrive:backup --exclude "*.tmp"`)
**When** they trigger parsing
**Then** the source path, remote name, remote path, action, ignore patterns, and flags are extracted and populate the profile form fields (FR9)
**And** parsing completes within 1 second (NFR5)

**Given** the user pastes a command with `--exclude` or `--filter` flags
**When** the command is parsed
**Then** exclude patterns are extracted into the ignore patterns list
**And** remaining flags are placed in the extra flags field

**Given** the user pastes a command that cannot be fully parsed
**When** parsing encounters unknown syntax
**Then** the parseable fields are populated and unparseable portions are placed in extra flags
**And** the user can manually adjust any field before saving

**Given** the parsed fields populate the form
**When** the user reviews and clicks Save
**Then** the profile is created identically to a manually-created profile

### Story 2.5: Profile Edit, Delete & Dry-Run

As a user,
I want to edit, delete, and test my profiles,
So that I can refine configurations and verify they work before running live syncs.

**Acceptance Criteria:**

**Given** a profile exists in the profile list
**When** the user selects edit
**Then** the profile form opens pre-populated with all current values (FR16)
**And** the user can modify any field and save changes

**Given** the user edits a profile that has a currently running job
**When** the edit form opens
**Then** a warning is displayed: "Changes will not affect the currently running job" (FR19)

**Given** the user wants to delete a profile
**When** they click the delete action
**Then** a confirmation dialog appears (NFR20)
**And** upon confirmation, the profile and its JSON file are removed (FR17)

**Given** the user is creating or editing a profile
**When** they click the "Test" button
**Then** a dry-run execution is triggered with the current form values (FR18)
**And** the rclone output is displayed so the user can preview what would happen
**And** no files are actually transferred or modified

---

## Epic 3: Job Execution & Tray Dashboard

Users can start and cancel sync jobs from both the tray popup and main GUI, see real-time status badges, elapsed time, and live log streaming. Network detection prevents offline execution. Process cleanup on quit prevents orphaned processes.

### Story 3.1: Job Execution Engine & Log Capture

As a user,
I want my rclone jobs to execute reliably with complete log capture,
So that every sync operation is tracked and I never lose visibility into what happened.

**Acceptance Criteria:**

**Given** a job is started for a profile
**When** JobManager receives the start request
**Then** the profile configuration is snapshotted as a value-type copy (FR25)
**And** RcloneService assembles the command with direct args for flags and a temp filter file for ignore patterns (FR26)
**And** a `Process` is spawned with stdout and stderr `Pipe` attached

**Given** a job is running
**When** rclone produces output
**Then** stdout and stderr are captured completely via `readabilityHandler` on a background queue (FR35)
**And** output chunks are appended to a raw log file at `{configDir}/logs/runs/{profileId}_{timestamp}.log` (FR37)
**And** a `LogEntry` is created in the JSON log index with profile ID, timestamp, status, and duration (FR36)

**Given** multiple jobs are started concurrently
**When** they execute simultaneously
**Then** each job runs independently with its own Process, Pipe, and log file (FR24)
**And** the UI remains responsive (NFR7)

**Given** the app is quit while jobs are running
**When** the quit sequence begins
**Then** all running processes receive SIGTERM (FR28)
**And** after 2 seconds, any remaining processes receive SIGKILL (NFR10)
**And** log files for interrupted jobs are finalized with "interrupted" status

**Given** a job completes (success or failure)
**When** the `terminationHandler` fires
**Then** remaining pipe data is read and flushed to the log file
**And** the LogEntry is updated with final status and duration
**And** the temp filter file is cleaned up
**And** job status updates within 1 second (NFR4)

### Story 3.2: Tray Popup Dashboard

As a user,
I want a tray popup that shows all my profiles with status and lets me start/cancel jobs,
So that I can manage syncs with two clicks without opening the full app.

**Acceptance Criteria:**

**Given** profiles exist and the user clicks the menu bar icon
**When** the tray popup opens
**Then** all profiles are displayed with status badges (green checkmark/red xmark/yellow clock) (FR45)
**And** idle profiles show the last successful run date/time (FR46)
**And** running profiles show elapsed run time (FR47)
**And** the popup opens within 200ms (NFR1)

**Given** a profile is idle
**When** the user views its row in the popup
**Then** a Start button is available (FR48)
**And** a History link is available that opens the history tab in the main GUI (FR50)

**Given** a profile has a running job
**When** the user views its row in the popup
**Then** a Cancel button replaces the Start button (FR49)
**And** the elapsed time updates in real-time

**Given** the user clicks Start on a profile
**When** the network is unavailable
**Then** the job is prevented from starting with a clear message (FR27)

**Given** the user clicks Start on a profile
**When** the network is available
**Then** the job starts via JobManager (FR20)
**And** the status badge transitions to running state

**Given** the user clicks Cancel on a running profile
**When** the cancel is confirmed
**Then** the job is cancelled via JobManager (FR22)

**Given** no profiles exist
**When** the user opens the tray popup
**Then** a "Create your first profile" button is displayed (FR51)
**And** clicking it opens the main GUI to the profile creation form

**Given** the popup is open
**When** the user clicks "Open Cirrus"
**Then** the main GUI window opens (FR52)

**Given** status badges are displayed
**When** a user with color vision deficiency views them
**Then** each badge uses both color AND an SF Symbol shape (checkmark/xmark/clock/circle) (NFR18)

### Story 3.3: Main GUI Profile Controls & Live Log

As a user,
I want to start and cancel jobs from the main GUI and see live output,
So that I have full control and visibility from the management interface.

**Acceptance Criteria:**

**Given** the user navigates to the Profiles tab
**When** profiles exist
**Then** the profile list displays each profile with: name, source, destination, last run status/time, next scheduled run, and a Start/Cancel button (FR53)
**And** the list renders within 300ms with 50+ profiles (NFR2)

**Given** a profile is idle in the main GUI
**When** the user clicks Start
**Then** the job begins executing via JobManager (FR21)
**And** the status badge updates to running

**Given** a profile has a running job in the main GUI
**When** the user clicks Cancel
**Then** a confirmation dialog appears (NFR20)
**And** upon confirmation, the job is cancelled (FR23)

**Given** a job is currently running
**When** the user views the running job
**Then** live log output streams in real-time via LiveLogView (FR42)
**And** log updates appear within 100ms of rclone output (NFR3)

---

## Epic 4: History & Log Viewer

Users can review per-profile run history, see status of each historical run, open a syntax-highlighted log viewer for any past execution, and start/cancel jobs directly from the history tab.

### Story 4.1: History Tab & Per-Profile Run History

As a user,
I want to view the run history for each profile,
So that I can audit past sync operations and quickly identify failures.

**Acceptance Criteria:**

**Given** the user navigates to the History tab
**When** the tab loads
**Then** a profile dropdown is displayed at the top with all profiles listed (FR54)
**And** each profile name in the dropdown has a status indicator (green/red/yellow) next to it (FR55)

**Given** the user selects a profile from the dropdown
**When** the profile's history loads
**Then** all historical runs are displayed sorted by most recent first (FR38)
**And** each run shows: date/time, status (successful/failed/canceled/interrupted), and duration (FR39)

**Given** the user switches to a different profile via the dropdown
**When** the selection changes
**Then** the run history updates to show the newly selected profile's runs (FR56)

**Given** a profile has no run history
**When** it is selected in the dropdown
**Then** an empty state message is displayed explaining no runs have been executed yet

### Story 4.2: Log Viewer & History Controls

As a user,
I want to view syntax-highlighted logs and control jobs from the history tab,
So that I can diagnose failures and re-run jobs without switching screens.

**Acceptance Criteria:**

**Given** the user views a profile's run history
**When** they click on a historical run entry
**Then** a log viewer sheet opens displaying the raw rclone output for that run (FR40)

**Given** the log viewer is displaying output
**When** the log contains error lines (e.g., "ERROR", "Failed to")
**Then** those lines are highlighted with a red background (FR41)
**And** warning lines are highlighted with a yellow background (FR41)

**Given** the user is on the history tab
**When** a profile is selected
**Then** Start and Cancel controls are available (FR57)
**And** Start is shown for idle profiles, Cancel for running profiles

**Given** a job is currently running for the selected profile
**When** the user views the history tab
**Then** live log output streams at the top of the history view (FR58)
**And** updates appear within 100ms of rclone output (NFR3)

**Given** the user clicks Start from the history tab
**When** the job begins
**Then** the live log view activates and streams output in real-time

---

## Epic 5: Scheduling

Users can automate sync jobs with cron-based schedules using a visual cron builder or raw cron expressions. Quit warning reminds users that scheduled jobs need the app running.

### Story 5.1: Cron Scheduling Engine

As a user,
I want to assign schedules to my profiles so jobs run automatically,
So that I can set up syncs once and stop thinking about them.

**Acceptance Criteria:**

**Given** a profile exists
**When** the user assigns a cron schedule to it
**Then** the schedule is stored as a `CronSchedule` (expression + enabled flag) on the profile (FR29)
**And** the profile JSON is updated with the schedule

**Given** a profile has an active schedule
**When** the scheduled time arrives and the app is running
**Then** ScheduleManager triggers job execution via JobManager (FR33)
**And** the job fires within 5 seconds of the scheduled time (NFR9)

**Given** a profile has a schedule
**When** the user removes the schedule
**Then** the profile reverts to on-demand-only execution (FR32)
**And** the schedule field is cleared from the profile JSON

**Given** the CronParser utility receives a cron expression
**When** the expression is evaluated
**Then** the next fire date is calculated correctly for standard 5-field cron syntax
**And** invalid expressions return a `CirrusError.invalidCronExpression` error

### Story 5.2: Cron Builder UI & Quit Warning

As a user,
I want a visual cron builder for easy schedule creation and a warning before quitting,
So that I can set schedules without memorizing cron syntax and never accidentally stop my automated syncs.

**Acceptance Criteria:**

**Given** the user opens the schedule configuration for a profile
**When** the cron builder UI is displayed
**Then** a visual builder allows selecting frequency (minute, hour, day, day-of-week, month) via dropdown controls (FR30)
**And** a raw cron expression text field is available for direct input (FR31)
**And** both inputs produce the same CronSchedule — editing one updates the other

**Given** the user enters a cron expression (visual or raw)
**When** the expression is valid
**Then** a human-readable summary is displayed (e.g., "Every day at 2:00 AM")
**And** the next scheduled run time is shown

**Given** the user enters an invalid cron expression
**When** validation runs
**Then** an error message is displayed explaining the issue (NFR21)
**And** the schedule cannot be saved until corrected

**Given** profiles have active schedules
**When** the user clicks Quit
**Then** the quit confirmation dialog warns: "Quitting will stop all scheduled jobs" (FR34)
**And** the number of active schedules is shown in the warning
**And** the user must confirm before the app terminates
