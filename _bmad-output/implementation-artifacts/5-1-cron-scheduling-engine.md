# Story 5.1: Cron Scheduling Engine

Status: ready-for-dev

## Story

As a user,
I want to assign schedules to my profiles so jobs run automatically,
so that I can set up syncs once and stop thinking about them.

## Acceptance Criteria

1. **Given** a profile exists
   **When** the user assigns a cron schedule to it
   **Then** the schedule is stored as a `CronSchedule` (expression + enabled flag) on the profile (FR29)
   **And** the profile JSON is updated with the schedule

2. **Given** a profile has an active schedule
   **When** the scheduled time arrives and the app is running
   **Then** ScheduleManager triggers job execution via JobManager (FR33)
   **And** the job fires within 5 seconds of the scheduled time (NFR9)

3. **Given** a profile has a schedule
   **When** the user removes the schedule
   **Then** the profile reverts to on-demand-only execution (FR32)
   **And** the schedule field is cleared from the profile JSON

4. **Given** the CronParser utility receives a cron expression
   **When** the expression is evaluated
   **Then** the next fire date is calculated correctly for standard 5-field cron syntax
   **And** invalid expressions return a `CirrusError.invalidCronExpression` error

## Tasks / Subtasks

- [ ] Task 1: Implement CronParser (AC: #4)
  - [ ] 1.1: Create `Utilities/CronParser.swift`
  - [ ] 1.2: Parse standard 5-field cron: minute, hour, day-of-month, month, day-of-week
  - [ ] 1.3: Support: `*`, specific values (`5`), ranges (`1-5`), step values (`*/15`), lists (`1,3,5`)
  - [ ] 1.4: `static func nextFireDate(for expression: String, after date: Date) -> Date` — calculate next fire time
  - [ ] 1.5: `static func validate(_ expression: String) -> Bool` — check syntax
  - [ ] 1.6: `static func humanReadable(_ expression: String) -> String` — e.g., "Every day at 2:00 AM"
  - [ ] 1.7: Throw `CirrusError.invalidCronExpression` for invalid input
- [ ] Task 2: Implement ScheduleManager (AC: #2)
  - [ ] 2.1: Create `Stores/ScheduleManager.swift` — `@MainActor @Observable`
  - [ ] 2.2: `func start()` — begin evaluation loop
  - [ ] 2.3: Evaluation loop: every 30 seconds, check all profiles with active schedules
  - [ ] 2.4: For each scheduled profile, compute next fire date via CronParser
  - [ ] 2.5: If fire date has passed and job not already running → trigger via JobManager
  - [ ] 2.6: Track last fire time per profile to prevent double-firing
  - [ ] 2.7: `func stop()` — cancel evaluation loop
- [ ] Task 3: Wire ScheduleManager to app lifecycle
  - [ ] 3.1: Create ScheduleManager in CirrusApp/AppDelegate
  - [ ] 3.2: Inject via `.environment()`
  - [ ] 3.3: Call `scheduleManager.start()` on app launch
  - [ ] 3.4: Call `scheduleManager.stop()` on app quit
- [ ] Task 4: Add schedule CRUD to ProfileStore (AC: #1, #3)
  - [ ] 4.1: Profile already has `schedule: CronSchedule?` field (Story 2.2)
  - [ ] 4.2: Setting `profile.schedule = CronSchedule(expression: "0 2 * * *", enabled: true)` + save
  - [ ] 4.3: Removing: `profile.schedule = nil` + save
  - [ ] 4.4: Profile JSON automatically includes/excludes schedule via Codable
- [ ] Task 5: Write tests
  - [ ] 5.1: `CirrusTests/Utilities/CronParserTests.swift` — extensive parameterized tests:
    - `0 2 * * *` → daily at 2:00 AM
    - `*/15 * * * *` → every 15 minutes
    - `0 9 * * 1-5` → weekdays at 9:00 AM
    - `30 8,17 * * *` → 8:30 AM and 5:30 PM
    - Invalid: `60 * * * *`, `* * * *`, `abc`
  - [ ] 5.2: Test `nextFireDate` returns correct dates across boundaries (midnight, month-end)
  - [ ] 5.3: Test `humanReadable` returns sensible descriptions
  - [ ] 5.4: Test `validate` rejects invalid expressions
  - [ ] 5.5: Test ScheduleManager fires jobs within 5 seconds of schedule time (NFR9)

## Dev Notes

### Architecture Compliance

**Layer:** Utilities (CronParser) + Stores (ScheduleManager).

**File locations:**
```
Cirrus/Cirrus/
├── Utilities/
│   └── CronParser.swift                 # NEW
└── Stores/
    └── ScheduleManager.swift            # NEW
CirrusTests/Utilities/
    └── CronParserTests.swift            # NEW
```

### Technical Requirements

**CronParser — pure utility, no state:**
```swift
struct CronParser {
    static func nextFireDate(for expression: String, after date: Date = Date()) throws -> Date { ... }
    static func validate(_ expression: String) -> Bool { ... }
    static func humanReadable(_ expression: String) -> String { ... }
}
```
- Standard 5-field cron: `minute hour day-of-month month day-of-week`
- Use `Calendar.current` for date calculations
- Evaluate: consider using a lightweight Swift cron library OR implement ~100-150 lines of cron evaluation (architecture notes either approach is acceptable)
- Test extensively — cron edge cases are subtle (leap years, DST transitions, month boundaries)

**ScheduleManager evaluation loop:**
```swift
@MainActor @Observable
final class ScheduleManager {
    private var evaluationTask: Task<Void, Never>?
    private var lastFireDates: [UUID: Date] = [:]

    func start() {
        evaluationTask = Task {
            while !Task.isCancelled {
                await evaluateSchedules()
                try? await Task.sleep(for: .seconds(30))
            }
        }
    }

    private func evaluateSchedules() async {
        for profile in profileStore.profiles where profile.schedule?.enabled == true {
            guard let schedule = profile.schedule else { continue }
            guard let nextFire = try? CronParser.nextFireDate(for: schedule.expression) else { continue }

            let lastFired = lastFireDates[profile.id] ?? .distantPast
            if nextFire <= Date() && nextFire > lastFired && !jobManager.isRunning(for: profile.id) {
                jobManager.startJob(for: profile)
                lastFireDates[profile.id] = nextFire
            }
        }
    }
}
```

**Schedule accuracy (NFR9):** 30-second evaluation loop means worst case is 30 seconds late. To hit <5 seconds, consider:
- Calculate next fire time for all profiles, find the soonest
- Sleep until that time (minus buffer), then evaluate
- Or use a 5-second evaluation interval (acceptable CPU cost for a menu bar app)

**Inter-manager communication:** ScheduleManager calls `jobManager.startJob()` directly — no NotificationCenter.

### Enforcement Rules

- CronParser is pure — no state, no side effects. Ideal for parameterized testing.
- ScheduleManager is `@MainActor @Observable`
- ScheduleManager calls JobManager directly — no notifications
- Invalid cron expressions throw `CirrusError.invalidCronExpression`
- Evaluation loop uses `Task.sleep` — NOT `DispatchSourceTimer` or `Timer.scheduledTimer`

### Dependencies

- **Depends on:** Story 2.2 (Profile with CronSchedule), Story 3.1 (JobManager)
- **Does NOT depend on:** Story 5.2

### References

- [Source: architecture.md#State Management Architecture] — ScheduleManager → JobManager dependency
- [Source: architecture.md#Concurrency & Threading Patterns] — Task.sleep for timers
- [Source: epics.md#Story 5.1] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
