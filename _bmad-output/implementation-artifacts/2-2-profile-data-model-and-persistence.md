# Story 2.2: Profile Data Model & Persistence

Status: ready-for-dev

## Story

As a developer,
I want the Profile data model and persistence layer implemented,
so that profiles can be created, stored, and retrieved reliably.

## Acceptance Criteria

1. **Given** the Profile model is defined
   **When** a Profile struct is created
   **Then** it contains all required fields: id (UUID), name, sourcePath, remoteName, remotePath, action (RcloneAction enum), ignorePatterns, extraFlags, schedule (optional), groupName (optional), sortOrder, createdAt, updatedAt
   **And** it conforms to `Codable` and `Identifiable`

2. **Given** the ProfileStore is initialized
   **When** profiles are saved
   **Then** each profile is written as an individual JSON file in `{configDir}/profiles/{uuid}.json`
   **And** writes use `AtomicFileWriter` for crash-safe persistence (NFR11)
   **And** JSON encoding uses `JSONEncoder.cirrus` with ISO 8601 dates

3. **Given** the ProfileStore is initialized
   **When** the app launches
   **Then** all profiles are loaded from the profiles directory
   **And** invalid JSON files are skipped with a logged warning (not a crash)

4. **Given** a profile is deleted via ProfileStore
   **When** the delete completes
   **Then** the profile's JSON file is removed from disk
   **And** the profile is removed from the in-memory array

## Tasks / Subtasks

- [ ] Task 1: Create Profile model (AC: #1)
  - [ ] 1.1: Create `Models/Profile.swift`
  - [ ] 1.2: Implement `Profile` struct with all fields, `Codable`, `Identifiable`
  - [ ] 1.3: Create `RcloneAction` enum: `.sync`, `.copy`, `.move`, `.delete` with raw String values
  - [ ] 1.4: Add computed property `displayDescription` to `RcloneAction` for UI display
- [ ] Task 2: Create supporting models (AC: #1)
  - [ ] 2.1: Create `Models/JobStatus.swift` — enum: `.idle`, `.running`, `.success`, `.failed`, `.canceled`
  - [ ] 2.2: Create `Models/CronSchedule.swift` — struct with `expression: String`, `enabled: Bool`
- [ ] Task 3: Implement ProfileStore (AC: #2, #3, #4)
  - [ ] 3.1: Create `Stores/ProfileStore.swift` — `@MainActor @Observable` class
  - [ ] 3.2: `private(set) var profiles: [Profile]` — observed by views
  - [ ] 3.3: `func loadAll()` — scan `{configDir}/profiles/` for `.json` files, decode each, skip invalid
  - [ ] 3.4: `func save(_ profile: Profile)` — write to `{configDir}/profiles/{id}.json` via AtomicFileWriter
  - [ ] 3.5: `func delete(_ profile: Profile)` — remove file from disk, remove from array
  - [ ] 3.6: `func profile(for id: UUID) -> Profile?` — lookup by ID
  - [ ] 3.7: Log warnings for invalid JSON files using `os.Logger` — do not crash
- [ ] Task 4: Inject ProfileStore into environment (AC: #3)
  - [ ] 4.1: Create `ProfileStore` instance in `CirrusApp` or `AppDelegate`
  - [ ] 4.2: Inject via `.environment()` modifier on root views
  - [ ] 4.3: Call `profileStore.loadAll()` on app launch
- [ ] Task 5: Write tests
  - [ ] 5.1: `CirrusTests/Models/ProfileTests.swift` — Codable round-trip, all fields preserved, RcloneAction raw values
  - [ ] 5.2: `CirrusTests/Stores/ProfileStoreTests.swift` — save creates file, loadAll reads files, delete removes file, invalid JSON skipped
  - [ ] 5.3: Test that `AtomicFileWriter` is used (verify temp file pattern)

## Dev Notes

### Architecture Compliance

**Layer:** Models (Profile, JobStatus, CronSchedule) + Stores (ProfileStore).

**File locations:**
```
Cirrus/Cirrus/
├── Models/
│   ├── Profile.swift              # NEW
│   ├── JobStatus.swift            # NEW
│   └── CronSchedule.swift         # NEW
├── Stores/
│   └── ProfileStore.swift         # NEW
CirrusTests/
├── Models/
│   └── ProfileTests.swift         # NEW
└── Stores/
    └── ProfileStoreTests.swift    # NEW
```

### Technical Requirements

**Profile struct — exact specification:**
```swift
struct Profile: Codable, Identifiable {
    let id: UUID
    var name: String
    var sourcePath: String
    var remoteName: String
    var remotePath: String
    var action: RcloneAction
    var ignorePatterns: [String]
    var extraFlags: String
    var schedule: CronSchedule?
    var groupName: String?
    var sortOrder: Int
    var createdAt: Date
    var updatedAt: Date
}

enum RcloneAction: String, Codable, CaseIterable {
    case sync, copy, move, delete
}
```
- `id` is `let` (immutable after creation) — use `UUID()` on creation
- `createdAt` is set once on creation, `updatedAt` is set on every save
- `RcloneAction` uses lowercase raw values for JSON: `"sync"`, `"copy"`, `"move"`, `"delete"`

**ProfileStore pattern:**
```swift
@MainActor @Observable
final class ProfileStore {
    private(set) var profiles: [Profile] = []
    private let configDirectoryURL: () -> URL  // injected from AppSettings

    func loadAll() async { /* scan directory, decode, skip invalid */ }
    func save(_ profile: Profile) async throws { /* AtomicFileWriter */ }
    func delete(_ profile: Profile) throws { /* FileManager.removeItem */ }
    func profile(for id: UUID) -> Profile? { profiles.first { $0.id == id } }
}
```
- `ProfileStore` gets `configDirectoryURL` from `AppSettings` — not hardcoded
- Profiles directory: `{configDirectoryURL}/profiles/`
- Individual file: `{configDirectoryURL}/profiles/{uuid}.json`

**File naming:** `{uuid}.json` — lowercase UUID with hyphens. Example: `a1b2c3d4-e5f6-7890-abcd-ef1234567890.json`

**Invalid JSON handling:** On `loadAll()`, iterate all `.json` files. If any fails to decode, log with `os.Logger` and skip. Do NOT crash. Do NOT delete the file.

### Enforcement Rules

- Use `JSONEncoder.cirrus` / `JSONDecoder.cirrus` for ALL profile serialization
- Use `AtomicFileWriter` for ALL profile writes (NFR11)
- `profiles` array is `private(set)` — mutation only through methods
- `ProfileStore` is `@MainActor @Observable`
- Never hardcode paths — derive from `AppSettings.configDirectoryURL`
- `try?` forbidden on profile saves — always propagate errors
- `try?` acceptable on `loadAll()` for individual file failures (skip and continue)

### Dependencies

- **Depends on:** Story 1.1 (AtomicFileWriter, JSONCoders, CirrusError, AppSettings)
- **Does NOT depend on:** Stories 2.3-2.5 or Epics 3-5

### References

- [Source: architecture.md#Data Architecture] — Profile struct, file storage layout
- [Source: architecture.md#State Management Patterns] — @Observable, private(set)
- [Source: architecture.md#JSON & Data Format Patterns] — Encoder/decoder config
- [Source: epics.md#Story 2.2] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
