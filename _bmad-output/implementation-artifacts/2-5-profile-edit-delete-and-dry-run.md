# Story 2.5: Profile Edit, Delete & Dry-Run

Status: ready-for-dev

## Story

As a user,
I want to edit, delete, and test my profiles,
so that I can refine configurations and verify they work before running live syncs.

## Acceptance Criteria

1. **Given** a profile exists in the profile list
   **When** the user selects edit
   **Then** the profile form opens pre-populated with all current values (FR16)
   **And** the user can modify any field and save changes

2. **Given** the user edits a profile that has a currently running job
   **When** the edit form opens
   **Then** a warning is displayed: "Changes will not affect the currently running job" (FR19)

3. **Given** the user wants to delete a profile
   **When** they click the delete action
   **Then** a confirmation dialog appears (NFR20)
   **And** upon confirmation, the profile and its JSON file are removed (FR17)

4. **Given** the user is creating or editing a profile
   **When** they click the "Test" button
   **Then** a dry-run execution is triggered with the current form values (FR18)
   **And** the rclone output is displayed so the user can preview what would happen
   **And** no files are actually transferred or modified

## Tasks / Subtasks

- [ ] Task 1: Add edit functionality (AC: #1)
  - [ ] 1.1: Add edit button/action to each profile row in ProfileListView
  - [ ] 1.2: Open ProfileFormView as sheet with profile data pre-populated
  - [ ] 1.3: On save, update `updatedAt` timestamp and call `profileStore.save()`
  - [ ] 1.4: Reuse the same ProfileFormView for both create and edit modes
- [ ] Task 2: Add running job warning (AC: #2)
  - [ ] 2.1: In ProfileFormView, check if profile has a running job (query JobManager when available)
  - [ ] 2.2: Display warning banner: "Changes will not affect the currently running job"
  - [ ] 2.3: Note: JobManager doesn't exist yet (Epic 3). Build the UI hook — check `jobManager?.isRunning(profileId:)` with optional access. Warning will activate once Epic 3 is complete.
- [ ] Task 3: Add delete functionality (AC: #3)
  - [ ] 3.1: Add delete button/swipe action to each profile row
  - [ ] 3.2: Show confirmation alert: "Delete '{profile.name}'? This cannot be undone." (NFR20)
  - [ ] 3.3: On confirm, call `profileStore.delete(profile)`
- [ ] Task 4: Implement dry-run Test (AC: #4)
  - [ ] 4.1: Add "Test" button to ProfileFormView
  - [ ] 4.2: Assemble rclone command from current form values with `--dry-run` flag appended
  - [ ] 4.3: Execute via `RcloneService` — run `Process` and capture stdout/stderr
  - [ ] 4.4: Display output in a sheet or expandable section below the form
  - [ ] 4.5: Show "No files will be modified" indicator clearly
- [ ] Task 5: Write tests
  - [ ] 5.1: Test edit pre-populates all fields correctly
  - [ ] 5.2: Test delete removes file from disk via ProfileStore
  - [ ] 5.3: Test dry-run appends `--dry-run` flag to command
  - [ ] 5.4: Test confirmation dialog appears before delete

## Dev Notes

### Architecture Compliance

**Layer:** Views (Profiles) + Services (RcloneService for dry-run).

**File locations:**
```
Cirrus/Cirrus/
├── Views/MainWindow/Profiles/
│   ├── ProfileListView.swift              # MODIFY — add edit/delete actions
│   └── ProfileFormView.swift              # MODIFY — add edit mode, dry-run, running warning
CirrusTests/
    └── (add to existing test files)
```

### Technical Requirements

**Edit mode for ProfileFormView:**
- Accept optional `Profile?` parameter — if present, it's edit mode
- Pre-populate all form fields from the existing profile
- On save: set `updatedAt = Date()`, call `profileStore.save(existingProfile)`
- Reuse the same form view for create and edit — don't duplicate

**Delete confirmation:**
```swift
.alert("Delete Profile", isPresented: $showDeleteConfirmation) {
    Button("Delete", role: .destructive) { profileStore.delete(profile) }
    Button("Cancel", role: .cancel) { }
} message: {
    Text("Delete '\(profile.name)'? This cannot be undone.")
}
```

**Dry-run execution:**
- Build command from form values: `[rclonePath, action, sourcePath, "\(remoteName):\(remotePath)", "--dry-run"] + extraFlags.split`
- If ignore patterns exist, write a temp filter file via `FilterFileWriter` (not yet created — for now, add `--exclude` flags directly)
- Run `Process`, capture stdout+stderr, display in a scrollable text view
- `FilterFileWriter` will be created in Story 3.1 — for dry-run in this story, convert ignorePatterns to `--exclude` args directly

**FR19 — running profile warning:**
- The UI hook is built now, but `JobManager` doesn't exist until Epic 3
- Use optional binding: `if let jobManager = jobManager, jobManager.isRunning(for: profile.id)`
- Or use a simple boolean property that defaults to `false` until JobManager is injected
- The warning banner will naturally activate when JobManager starts tracking jobs

### Enforcement Rules

- Reuse `ProfileFormView` for create and edit — do NOT create a separate edit form
- Delete MUST show confirmation dialog (NFR20)
- Dry-run MUST append `--dry-run` flag — never run a real sync from the test button
- Running job check is built as a UI hook that activates later — acceptable incomplete feature

### Dependencies

- **Depends on:** Story 2.2 (ProfileStore), Story 2.3 (ProfileFormView, ProfileListView)
- **Partial dependency:** FR19 warning fully activates after Epic 3 (JobManager)
- **Does NOT depend on:** Epics 4-5

### References

- [Source: architecture.md#View Patterns] — alerts, sheets
- [Source: architecture.md#Error Handling Patterns] — user-facing messages
- [Source: epics.md#Story 2.5] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
