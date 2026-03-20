---
stepsCompleted: ['step-01-init', 'step-02-discovery', 'step-02b-vision', 'step-02c-executive-summary', 'step-03-success', 'step-04-journeys', 'step-05-domain', 'step-06-innovation', 'step-07-project-type', 'step-08-scoping', 'step-09-functional', 'step-10-nonfunctional', 'step-11-polish']
inputDocuments: ['_bmad-output/brainstorming/brainstorming-session-2026-02-27.md']
workflowType: 'prd'
documentCounts:
  briefs: 0
  research: 0
  brainstorming: 1
  projectDocs: 0
classification:
  projectType: desktop_app
  domain: general
  complexity: low
  projectContext: greenfield
---

# Product Requirements Document - Cirrus

**Author:** Sane
**Date:** 2026-02-27

## Executive Summary

A native macOS menu bar application for managing rclone file synchronization jobs through reusable profiles. The app replaces hand-written shell scripts with a GUI-driven workflow for configuring, executing, scheduling, and monitoring rclone operations (sync, copy, move, delete) across any rclone-supported remote. Target users are developers, power users, and sysadmins who manage personal file sync jobs across multiple cloud providers and remote storage backends. The core problem: rclone's power is locked behind repetitive CLI boilerplate, and the friction of manual script execution means sync jobs get skipped or forgotten.

### What Makes This Special

The only native desktop rclone GUI that treats rclone as a multi-remote orchestration tool rather than a single-service sync client. Existing alternatives fall into two camps: rclone's own web GUI (designed for server management, not personal desktop use) and third-party wrappers (typically locked to a single service like Google Drive). This app covers the full breadth of rclone's remote support — S3, Google Drive, Backblaze B2, SFTP, and 40+ other backends — in a single native interface with consistent profile configuration, cron-based scheduling, and per-profile execution visibility. The core insight: the tool's primary value is friction reduction. Collapsing the distance between "I should sync" to "it's syncing" from multi-step CLI workflows to two clicks in a menu bar dropdown is what makes sync jobs actually happen.

## Project Classification

- **Type:** Native macOS desktop application (menu bar / system tray)
- **Domain:** General utility — personal file sync management
- **Complexity:** Low — thin orchestration layer wrapping rclone CLI
- **Context:** Greenfield — new product, no existing codebase
- **Tech Stack:** Swift + SwiftUI, full native per platform (future Windows support planned)

## Success Criteria

### User Success

- Users replace shell scripts with app-managed profiles and don't reach back to the terminal for rclone operations
- A new user can create their first profile and execute it within 5 minutes of opening the app
- Users trust the app enough to set up scheduled syncs and stop thinking about them
- The tray popup provides enough information to triage job status without opening the main GUI

### Business Success

- Open-source adoption: active GitHub stars, forks, and community contributions
- Users recommend the tool as the go-to native rclone GUI for personal multi-remote sync management
- Positive community feedback on UX polish and reliability — the app feels native, not like a wrapper

### Technical Success

- Jobs never silently fail — every execution is logged and its status is surfaced in the UI
- The UI remains responsive with 50+ profiles configured and multiple jobs running concurrently
- Log capture is complete and reliable — no truncated output, no missed errors
- Scheduled jobs fire on time when the app is running, with no drift or missed triggers

### Measurable Outcomes

- Profile creation from pasted rclone command takes under 30 seconds
- Tray popup opens instantly (< 200ms) regardless of profile count
- Job status updates in real-time with no perceptible lag
- Zero data loss from app-side bugs — if rclone executes correctly, the app reflects that correctly

## Product Scope & Development Strategy

### MVP Philosophy

Problem-solving MVP — deliver the minimum feature set that makes users stop writing shell scripts. The app must handle the full lifecycle: configure a profile, execute it, schedule it, and view what happened.

**Resource Requirements:** Solo developer (Sane). Swift + SwiftUI experience required. No backend infrastructure needed.

### MVP (Phase 1)

**Core User Journeys Supported:**
- Fiona's "My First Sync" (profile creation, execution, scheduling)
- Pete's "Replacing My Shell Scripts" (paste command parsing, bulk setup, cron scheduling)
- Sam's "Monday Morning Triage" (status badges, history, log viewer)

**Capabilities:**
- Tray popup mini dashboard (start/cancel/status/last run/elapsed time/history link)
- Profile CRUD (manual form + paste rclone command parsing)
- rclone actions with descriptions (sync, copy, move, delete)
- Job execution with log capture (JSON index + raw log files per execution)
- History tab with per-profile runs, log viewer with syntax highlighting (red for errors, yellow for warnings)
- Live log streaming for running jobs
- Scheduling (visual cron builder + raw cron expression input)
- rclone discovery/installation flow (check PATH, manual locate, or auto-install to ~/.local/bin)
- Network detection (prevent offline job starts)
- Launch at startup (Login Item)
- Settings (rclone path, rclone version display, config location)
- Individual JSON per profile + app settings JSON storage
- User-configurable config location (default ~/.config/yourapp/)

### Growth (Phase 2)

- User-defined groups and custom profile ordering in tray popup
- Remote path browsing via `rclone lsd` in destination selection
- Start/Cancel controls on the history tab
- Resume missed scheduled jobs on app launch
- Restart interrupted jobs on app launch
- Sparkle framework for update notifications

### Vision (Phase 3)

- Windows native port (WinUI or similar)
- Linux native port
- Profile import/export for sharing configurations
- Community-maintained profile templates for common sync patterns

### Risk Mitigation

**Technical Risks:**
- *rclone command parsing edge cases* — rclone has many flags and complex syntax. Mitigation: Parse common patterns (source, dest, action, excludes, common flags). Anything unparseable falls back to manual entry. Don't try to support 100% of rclone's syntax.
- *SwiftUI menu bar popup complexity* — Custom popups anchored to menu bar items can be tricky (window positioning, focus behavior). Mitigation: Research existing open-source SwiftUI menu bar apps for proven patterns.
- *Process lifecycle management* — Tracking child processes, handling cleanup on quit, preventing orphans. Mitigation: Build a robust ProcessManager early and test edge cases (force quit, system sleep, crash recovery).

**Market Risks:**
- *Small niche audience* — rclone users who want a GUI are a subset of rclone users. Mitigation: Open-source and built for personal use first. Community adoption is a bonus, not a requirement.

**Resource Risks:**
- *Solo developer scope creep* — Temptation to add features before core is solid. Mitigation: Strict MVP boundaries. Ship the core loop (create → execute → schedule → view logs) before adding any growth features.

## User Journeys

### Journey 1: First-Timer Fiona — "My First Sync"

**Who she is:** Fiona is a photographer who just learned about rclone from a YouTube tutorial. She set up a Google Drive remote and wants to back up her `~/Documents` folder. She's not deeply technical but can follow instructions.

**Opening Scene:** Fiona has been manually dragging files to Google Drive in her browser. It's slow, she forgets to do it, and she's lost work before. She heard rclone can automate this but the terminal intimidates her. She finds this app and installs it.

**Rising Action:** A menu bar icon appears. She clicks it — an empty popup with a friendly "Create your first profile" button. She clicks it and the main GUI opens. She picks a source folder with a native folder picker. She selects `gdrive:` from the remote dropdown, clicks Browse to find her `Backups/` folder. She sees action choices with clear descriptions — reads that `copy` uploads files without touching existing ones. That feels safe. She adds a couple ignore patterns for `.DS_Store` and `*.tmp`. She clicks the "Test" button — a dry-run executes and shows her exactly what would be copied. She sees her files listed. Confidence builds.

**Climax:** She saves the profile and hits Start from the profile list. The tray icon shows activity. She clicks it — her profile shows an elapsed timer and a live log streaming output. She watches her files uploading in real-time. It finishes. A green checkmark appears next to her profile.

**Resolution:** The next day she clicks the tray icon, sees her profile with "Last successful run: yesterday at 3:42 PM." She clicks Start again. Two clicks, syncing. She never opens the terminal. A week later she sets up a schedule — daily at 2am. She stops thinking about backups entirely.

**Requirements revealed:** Empty state UX, folder picker, remote browser, action descriptions, Test (dry-run) button, live log streaming, tray status badges, profile creation flow, scheduling setup.

### Journey 2: Power User Pete — "Replacing My Shell Scripts"

**Who he is:** Pete is a developer with 15 rclone remotes — S3 buckets, multiple Google Drives, Backblaze B2, SFTP servers. He has ~20 shell scripts for different sync jobs. He runs different ones at different frequencies and is tired of maintaining scripts.

**Opening Scene:** Pete's `~/scripts/sync/` folder has 20 shell scripts he's accumulated over two years. Some are daily, some weekly, some on-demand. He can never remember which flag combinations each one uses. He just broke a sync because he copy-pasted the wrong exclude pattern between scripts.

**Rising Action:** Pete installs the app. rclone is already in his PATH — detected instantly. He clicks the tray icon, hits "Create your first profile." Instead of filling out the form manually, he pastes his first shell script command: `rclone sync ~/projects/webapp s3:my-bucket/webapp-backup --exclude "node_modules/**" --exclude ".git/**" --dry-run`. The parser fills in every field — source, destination, action, ignore patterns, and the dry-run flag. He reviews, removes the dry-run flag, saves. 30 seconds per profile.

**Climax:** Twenty minutes later, all 20 profiles are migrated. He sets up cron schedules — types `0 2 * * *` for his daily backups, `0 4 * * 0` for weeklies. The visual cron builder confirms his expressions visually. He deletes his `~/scripts/sync/` folder.

**Resolution:** Pete's tray popup shows all 20 profiles with status badges. Dailies ran last night — all green. He clicks one to check the history. Clean. He hasn't opened a terminal for rclone in a month. When he adds a new S3 bucket, he creates a profile in under a minute.

**Requirements revealed:** rclone command parsing, cron expression input, visual cron builder, bulk profile creation efficiency, remote auto-discovery, profile list with status overview.

### Journey 3: Sysadmin Sam — "Monday Morning Triage"

**Who she is:** Sam manages backups for critical project data. She has 15 profiles syncing databases exports, shared project folders, and client deliverables. Reliability is her top priority — if a sync fails, she needs to know immediately and fix it fast.

**Opening Scene:** It's Monday morning. Sam was off all weekend. Her sync profiles ran on schedule — or did they? She needs to audit the weekend's activity in under two minutes.

**Rising Action:** Sam clicks the tray icon. Her profiles are listed with status badges. She scans — 12 green, 2 red, 1 yellow. The two red ones failed. She clicks the History link on the first red profile. The history tab opens with that profile selected in the dropdown — she can see the dropdown has status indicators next to every profile name. The failed run is at the top of the list: "Saturday 02:00 — Failed."

**Climax:** She clicks the failed log entry. A popup shows the raw rclone output with syntax highlighting. Red-highlighted lines immediately jump out: "Failed to copy: connection refused." Network issue Saturday night. She clicks Start to re-run the profile right from the history tab. The live log streams in real-time — files transferring, progress visible. It completes. Green checkmark.

**Resolution:** She checks the second failed profile — same network issue, same fix. The yellow one was interrupted (her machine rebooted during a macOS update). She re-runs it. All green in under 5 minutes. She switches to a different profile in the dropdown to spot-check a successful run — logs look clean. Monday morning triage: done.

**Requirements revealed:** Status badges in tray and history dropdown, per-profile history with status filtering, log viewer with syntax highlighting, Start/Cancel from history tab, live log streaming, reliable scheduling.

### Journey 4: Edge Cases — "When Things Go Wrong"

**Scenario A — rclone Not Found:** A user installs the app but doesn't have rclone. On first launch, the app detects rclone is missing. It presents two options: "Point me to your rclone binary" with a file picker, or "Download and install rclone" which installs to `~/.local/bin`. The user picks download. rclone installs. The app stores the path in settings and proceeds normally.

**Scenario B — Job Fails Mid-Execution:** A sync job is running when the network drops. The rclone process exits with an error. The app captures the complete stderr output, marks the job as "Failed" in the log index, updates the tray status badge to red. The raw log preserves the exact error. The user sees the red badge, investigates via history, and can re-run when ready.

**Scenario C — Editing a Running Profile:** A user notices a typo in an ignore pattern while a job is actively running. They open the profile editor and fix it. The app warns: "Changes will not affect the currently running job." The running job completes using the original config snapshot. The next execution uses the updated config.

**Scenario D — App Quit During Scheduled Jobs:** A user clicks Quit. The app warns: "Quitting will stop all scheduled jobs. Currently running jobs will be interrupted." The user confirms. On next launch, the app starts silently in the tray, detects missed schedules and interrupted jobs, and resumes/restarts them.

**Requirements revealed:** rclone discovery/install flow, error capture and status reporting, config snapshot at execution time, edit-while-running warning, quit warning with scheduling context, launch recovery for missed/interrupted jobs.

### Journey Requirements Summary

| Capability Area | Revealed By |
|---|---|
| Profile creation (manual form) | Fiona |
| Profile creation (paste rclone command) | Pete |
| Test/dry-run during configuration | Fiona |
| Tray popup with status badges | Fiona, Sam |
| Live log streaming | Fiona, Sam |
| History tab with per-profile logs | Sam |
| Log viewer with syntax highlighting | Sam |
| Cron scheduling (visual + raw) | Pete |
| rclone discovery/installation | Edge Cases |
| Config snapshot at execution | Edge Cases |
| Start/Cancel from multiple entry points | Sam, Edge Cases |
| Quit warning with scheduling context | Edge Cases |
| Launch recovery (missed/interrupted) | Edge Cases |
| Remote browsing for destination | Fiona |
| Action descriptions | Fiona |
| Empty state with onboarding CTA | Fiona |

## Desktop App Specific Requirements

### Project-Type Overview

Native macOS menu bar application built with Swift + SwiftUI. The app runs as a persistent menu bar item, providing a custom popup for quick job management and a full windowed GUI for configuration and history. Future platform support planned for Windows, then Linux — each as fully native implementations.

### Platform Support

| Platform | Stack | Status |
|---|---|---|
| macOS | Swift + SwiftUI | MVP |
| Windows | Native (TBD — WinUI, WPF, or similar) | Future |
| Linux | Native (TBD) | Future (after Windows) |

**macOS minimum version:** Target current macOS and one version back (Ventura+)

### System Integration

- **Menu bar item:** Persistent tray icon with custom SwiftUI popup (not native NSMenu)
- **Launch at startup:** Register as Login Item so the app starts automatically on boot
- **Window management:** Closing the main GUI window keeps the app running in the tray. Explicit Quit button required to fully exit.
- **Process management:** App shells out to rclone via Swift `Process` API, capturing stdout/stderr for log streaming and storage

### Update Strategy

- **MVP:** Manual updates — user downloads new version from GitHub releases
- **Future consideration:** Sparkle framework or similar for in-app update notifications

### Offline & Network Handling

- **Network detection:** Check network availability before starting a job. If offline, prevent job execution with clear messaging.
- **Mid-job network failure:** rclone handles transient failures via its own retry mechanisms (`--retries`, `--retries-sleep`). If the job ultimately fails, the app captures the full error output, marks the job as failed, and surfaces the status badge. User re-runs manually when ready.
- **No app-level resume:** The app does not attempt to resume partial rclone jobs. rclone's own `--resume` flag can be added as a profile flag by the user if needed for large file transfers.

### Implementation Considerations

- **Sandboxing:** If distributing outside the App Store, the app needs access to arbitrary filesystem paths (source folders) and network (rclone execution). App Sandbox may need to be disabled or use security-scoped bookmarks for folder access.
- **rclone process lifecycle:** Each job spawns a child `Process`. The app must track all running processes, handle cleanup on app quit, and ensure no orphaned rclone processes persist.
- **Config snapshot:** Profile config is snapshotted at job start time. Edits to a profile while its job is running do not affect the in-flight execution.

## Functional Requirements

### rclone Setup & Configuration

- FR1: User can configure the path to the rclone executable
- FR2: App can automatically detect rclone in the system PATH on launch
- FR3: User can download and install rclone to `~/.local/bin` from within the app
- FR4: User can view the installed rclone version in settings
- FR5: User can configure the storage location for profiles and app settings
- FR6: App can discover configured rclone remotes via `rclone listremotes`
- FR7: User can manually add remote names that aren't auto-discovered

### Profile Management

- FR8: User can create a new profile by filling out a manual form (source, destination, action, ignore patterns, flags)
- FR9: User can create a new profile by pasting an rclone command that is parsed into form fields
- FR10: User can select a local source folder using a native folder picker
- FR11: User can select a destination remote from a dropdown of discovered remotes
- FR12: User can specify a path on the destination remote via text input
- FR13: User can select an rclone action (sync, copy, move, delete) with descriptions of each action's behavior
- FR14: User can add, edit, and remove one or more ignore patterns per profile
- FR15: User can configure common rclone flags (e.g., --dry-run, --verbose) per profile
- FR16: User can edit an existing profile's configuration
- FR17: User can delete a profile
- FR18: User can execute a dry-run ("Test") during profile creation or editing to preview what would happen
- FR19: App warns the user when editing a profile that has a currently running job

### Job Execution

- FR20: User can start a profile's rclone job from the tray popup
- FR21: User can start a profile's rclone job from the main GUI profile list
- FR22: User can cancel a running job from the tray popup
- FR23: User can cancel a running job from the main GUI
- FR24: App can execute multiple jobs concurrently with no enforced limit
- FR25: App snapshots the profile configuration at job start time so mid-run edits do not affect the running job
- FR26: App assembles rclone commands using direct args for flags and filter files for ignore patterns
- FR27: App prevents job execution when no network connection is detected
- FR28: App tracks all running rclone child processes and cleans them up on app quit

### Scheduling

- FR29: User can assign a cron-based schedule to a profile
- FR30: User can define schedules using a visual cron builder UI
- FR31: User can define schedules by entering a raw cron expression
- FR32: User can remove a schedule from a profile (on-demand only)
- FR33: App executes scheduled jobs automatically when the app is running
- FR34: App warns the user on quit that scheduled jobs will stop running

### Logging & History

- FR35: App captures complete stdout and stderr from every rclone execution
- FR36: App stores a JSON log index with metadata per execution (profile, timestamp, status, duration)
- FR37: App stores raw log output as individual files per execution
- FR38: User can view per-profile run history sorted by most recent first
- FR39: User can see the status of each historical run (successful, failed, canceled, interrupted)
- FR40: User can open a log viewer that displays the raw output of any historical run
- FR41: Log viewer highlights error lines with red background and warning lines with yellow background
- FR42: User can view live-streaming log output for currently running jobs

### Tray Popup Dashboard

- FR43: App displays a persistent menu bar icon
- FR44: User can click the menu bar icon to open a custom popup
- FR45: Tray popup displays all configured profiles with status badges (green/red/yellow)
- FR46: Tray popup displays last successful run date/time for idle profiles
- FR47: Tray popup displays elapsed run time for currently running profiles
- FR48: Tray popup provides a Start button per idle profile
- FR49: Tray popup provides a Cancel button per running profile
- FR50: Tray popup provides a History link per profile that opens the history tab
- FR51: Tray popup displays a "Create your first profile" button when no profiles exist
- FR52: User can open the main GUI from the tray popup

### Main GUI

- FR53: Main GUI displays a profile list showing name, source, destination, last run status/time, next scheduled run, and Start/Cancel button
- FR54: User can navigate to a History tab from the main GUI
- FR55: History tab provides a profile dropdown with status indicators (green/red/yellow) next to each profile name
- FR56: User can switch between profiles in the history tab via the dropdown
- FR57: User can start or cancel a job from the history tab
- FR58: User can view live log output for a running job from the history tab

### App Lifecycle

- FR59: App registers as a Login Item to launch at system startup
- FR60: Closing the main GUI window keeps the app running in the menu bar
- FR61: User can fully quit the app via an explicit Quit button
- FR62: App starts silently in the menu bar tray on launch

## Non-Functional Requirements

### Performance

- NFR1: Tray popup opens within 200ms of clicking the menu bar icon, regardless of profile count
- NFR2: Profile list in the main GUI renders within 300ms with 50+ profiles
- NFR3: Live log streaming updates the UI within 100ms of rclone output
- NFR4: Job status badge updates within 1 second of job completion or failure
- NFR5: Profile creation from a pasted rclone command parses and populates fields within 1 second
- NFR6: The app's idle memory footprint remains under 100MB with no running jobs
- NFR7: UI remains responsive (no frame drops or hangs) while multiple jobs execute concurrently

### Reliability

- NFR8: Every rclone execution produces a complete log entry — no silent failures, no missing logs
- NFR9: Scheduled jobs fire within 5 seconds of their scheduled time when the app is running
- NFR10: No orphaned rclone child processes remain after app quit or crash
- NFR11: Profile JSON files are written atomically to prevent corruption from crashes or power loss
- NFR12: The JSON log index remains consistent with raw log files — no phantom entries or missing files

### Integration

- NFR13: App supports rclone versions 1.60+ (current stable and recent releases)
- NFR14: App handles rclone output encoding correctly (UTF-8 stdout/stderr)
- NFR15: App gracefully handles unexpected rclone exit codes with appropriate status mapping
- NFR16: Filter files generated for --filter-from are valid rclone filter syntax

### Accessibility

- NFR17: All interactive elements are accessible via macOS VoiceOver (leveraging SwiftUI's built-in accessibility)
- NFR18: All status indicators use both color and icon/shape (not color-only) for color-blind users

### UX Quality

- NFR19: The tray popup and main GUI follow macOS Human Interface Guidelines for native look and feel
- NFR20: All destructive actions (delete profile, cancel running job, quit app) require confirmation
- NFR21: Error messages are user-facing and actionable — no raw stack traces or internal error codes exposed to users
