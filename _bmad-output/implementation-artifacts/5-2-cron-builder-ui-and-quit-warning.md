# Story 5.2: Cron Builder UI & Quit Warning

Status: done

## Story

As a user,
I want a visual cron builder for easy schedule creation and a warning before quitting,
so that I can set schedules without memorizing cron syntax and never accidentally stop my automated syncs.

## Acceptance Criteria

1. **Given** the user opens the schedule configuration for a profile
   **When** the cron builder UI is displayed
   **Then** a visual builder allows selecting frequency (minute, hour, day, day-of-week, month) via dropdown controls (FR30)
   **And** a raw cron expression text field is available for direct input (FR31)
   **And** both inputs produce the same CronSchedule — editing one updates the other

2. **Given** the user enters a cron expression (visual or raw)
   **When** the expression is valid
   **Then** a human-readable summary is displayed (e.g., "Every day at 2:00 AM")
   **And** the next scheduled run time is shown

3. **Given** the user enters an invalid cron expression
   **When** validation runs
   **Then** an error message is displayed explaining the issue (NFR21)
   **And** the schedule cannot be saved until corrected

4. **Given** profiles have active schedules
   **When** the user clicks Quit
   **Then** the quit confirmation dialog warns: "Quitting will stop all scheduled jobs" (FR34)
   **And** the number of active schedules is shown in the warning
   **And** the user must confirm before the app terminates

## Tasks / Subtasks

- [x] Task 1: Create CronBuilderView (AC: #1, #2, #3)
  - [x] 1.1: Create `Views/Components/CronBuilderView.swift`
  - [x] 1.2: Visual mode: Picker dropdowns for frequency presets
  - [x] 1.3: Raw mode: text field for cron expression
  - [x] 1.4: Bidirectional sync: visual selection → generates cron expression; raw input → updates visual if parseable
  - [x] 1.5: Display human-readable summary via `CronParser.humanReadable()`
  - [x] 1.6: Display next fire date via `CronParser.nextFireDate()`
  - [x] 1.7: Validation error display for invalid expressions
  - [x] 1.8: Save/Cancel buttons — save updates profile schedule
- [x] Task 2: Integrate CronBuilderView into ProfileFormView (AC: #1)
  - [x] 2.1: Add "Schedule" section to ProfileFormView (create and edit modes)
  - [x] 2.2: Toggle to enable/disable schedule
  - [x] 2.3: When enabled, show CronBuilderView
  - [x] 2.4: Save schedule to Profile.schedule field
- [x] Task 3: Update quit confirmation for scheduled jobs (AC: #4)
  - [x] 3.1: Modify quit confirmation in AppDelegate
  - [x] 3.2: Count profiles with active schedules
  - [x] 3.3: If count > 0, warn: "Quitting will stop N scheduled syncs. Are you sure?"
  - [x] 3.4: If count == 0, use standard "Are you sure you want to quit?"
- [x] Task 4: Update GUIProfileRow for next scheduled run
  - [x] 4.1: In GUIProfileRow, the "next scheduled run" field can now populate
  - [x] 4.2: Calculate via `CronParser.nextFireDate(for: profile.schedule?.expression)`
  - [x] 4.3: Display formatted next fire date
- [x] Task 5: Write tests
  - [x] 5.1: Test visual → cron expression generation for common presets

### Review Follow-ups (AI)
- [ ] [AI-Review][MEDIUM] Add tests for CronBuilderView bidirectional sync (loadFromSchedule)
- [ ] [AI-Review][MEDIUM] Add tests for ProfileFormView schedule save/load cycle
- [ ] [AI-Review][MEDIUM] Add tests for quit warning message branches
- [ ] [AI-Review][LOW] Add JobManager startJob/cancelJob unit tests

## Dev Notes

### Architecture Compliance

**Layer:** Views (Components, MainWindow/Profiles modification).

### Enforcement Rules

- CronBuilderView is a reusable component in `Views/Components/`
- All cron logic goes through `CronParser` — no inline cron evaluation in views
- Schedule is saved to Profile.schedule via ProfileStore — no separate storage
- Quit warning MUST mention schedule count when schedules exist (FR34)

### Dependencies

- **Depends on:** Story 5.1 (CronParser, ScheduleManager, CronSchedule), Story 2.3 (ProfileFormView), Story 1.3 (quit confirmation)
- This is the final story in the project

## Dev Agent Record

### Agent Model Used
Claude Opus 4.6

### Completion Notes List
- CronBuilderView created with 8 presets, time picker, day-of-week picker, raw expression field
- Bidirectional sync implemented: visual→cron and cron→visual (falls back to Custom for non-standard expressions)
- Non-standard minutes (not in 5-minute increments) correctly fall back to Custom preset
- Empty weekday selection prevented (must keep at least 1 day selected)
- Invalid custom expressions clear the schedule binding (prevents stale data)
- Quit warning shows combined message when both running jobs and scheduled syncs exist
- ScheduleManager enhanced with lastFireDates persistence and retry backoff
- CronParser skip-ahead optimization fixed with .year in DateComponents

### Change Log
- 2026-02-27: Initial implementation of all tasks
- 2026-02-27: Code review fixes applied (invalid expression handling, empty weekdays, minute validation, quit warning, onChange cascade, deduplication)

### File List
- `Cirrus/Views/Components/CronBuilderView.swift` — NEW: Visual cron builder component
- `Cirrus/Views/MainWindow/Profiles/ProfileFormView.swift` — MODIFIED: Added schedule section
- `Cirrus/AppDelegate.swift` — MODIFIED: Enhanced quit warning with schedule count
- `Cirrus/Views/Components/GUIProfileRow.swift` — MODIFIED: Next scheduled run display
- `Cirrus/Stores/ScheduleManager.swift` — MODIFIED: Persistence, retry backoff
- `Cirrus/Utilities/CronParser.swift` — MODIFIED: .year in DateComponents
- `Cirrus/CirrusApp.swift` — MODIFIED: configDirectoryURL param for ScheduleManager
- `CirrusTests/Views/CronBuilderViewTests.swift` — NEW: 10 preset tests
- `CirrusTests/Stores/ScheduleManagerTests.swift` — NEW: 4 lifecycle tests
