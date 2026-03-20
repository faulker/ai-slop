# Story 2.3: Remote Discovery & Profile Creation Form

Status: ready-for-dev

## Story

As a user,
I want to create a sync profile by selecting a source folder, destination remote, action, and options,
so that I can configure exactly how my files should be synced.

## Acceptance Criteria

1. **Given** the user opens the profile creation form
   **When** the remote dropdown is displayed
   **Then** it is populated with remotes discovered via `rclone listremotes` (FR6)
   **And** the user can manually type a remote name not in the list (FR7)

2. **Given** the user is filling out the profile form
   **When** they select a source folder
   **Then** a native macOS folder picker opens and the selected path populates the source field (FR10)

3. **Given** the user selects a destination
   **When** they pick a remote from the dropdown
   **Then** they can specify a path on the remote via a text input field (FR11, FR12)

4. **Given** the user selects an rclone action
   **When** they view the action selector (sync/copy/move/delete)
   **Then** each action has a one-line description explaining its behavior and consequences (FR13)

5. **Given** the user configures ignore patterns
   **When** they add, edit, or remove patterns
   **Then** the patterns list updates dynamically with add/remove controls (FR14)

6. **Given** the user configures flags
   **When** they enter extra rclone flags (e.g., `--verbose`, `--dry-run`)
   **Then** the flags are stored as a string on the profile (FR15)

7. **Given** all required fields are filled (name, source, remote, action)
   **When** the user clicks Save
   **Then** the profile is persisted via ProfileStore and appears in the profile list (FR8)

## Tasks / Subtasks

- [ ] Task 1: Add listremotes to RcloneService (AC: #1)
  - [ ] 1.1: Add `listRemotes(rclonePath:) async throws -> [String]` to `RcloneService`
  - [ ] 1.2: Parse `rclone listremotes` output (colon-terminated names, one per line)
  - [ ] 1.3: Strip trailing colons from remote names
- [ ] Task 2: Create ProfileFormView (AC: #1-7)
  - [ ] 2.1: Create `Views/MainWindow/Profiles/ProfileFormView.swift`
  - [ ] 2.2: Profile name text field (required)
  - [ ] 2.3: Source folder field with "Browse" button → `NSOpenPanel` folder picker (FR10)
  - [ ] 2.4: Remote name combo box — dropdown of discovered remotes + manual text entry (FR6, FR7)
  - [ ] 2.5: Remote path text field (FR12)
  - [ ] 2.6: Action selector with descriptions (FR13) — use segmented control or picker
  - [ ] 2.7: Ignore patterns list with add/remove buttons (FR14)
  - [ ] 2.8: Extra flags text field (FR15)
  - [ ] 2.9: Save button — validates required fields, creates Profile, calls `profileStore.save()`
  - [ ] 2.10: Cancel button — dismisses form
- [ ] Task 3: Create ActionSelectorView (AC: #4)
  - [ ] 3.1: Create `Views/MainWindow/Profiles/ActionSelectorView.swift`
  - [ ] 3.2: Display each `RcloneAction` case with description:
    - sync: "Make destination identical to source, deleting extra files"
    - copy: "Copy files from source to destination, skipping existing"
    - move: "Move files from source to destination, deleting from source"
    - delete: "Delete files from destination that match the patterns"
- [ ] Task 4: Wire profile creation to ProfileListView (AC: #7)
  - [ ] 4.1: Update `ProfileListView.swift` — replace placeholder with actual profile list
  - [ ] 4.2: Add "New Profile" button that opens `ProfileFormView` as a sheet
  - [ ] 4.3: Display profiles from `ProfileStore` with name, source, destination
- [ ] Task 5: Write tests
  - [ ] 5.1: `CirrusTests/Services/RcloneServiceTests.swift` — test listRemotes parsing (add to existing)
  - [ ] 5.2: Test form validation: required fields prevent save
  - [ ] 5.3: Test profile save creates file via ProfileStore

## Dev Notes

### Architecture Compliance

**Layer:** Views (Profiles) + Services (RcloneService enhancement).

**File locations:**
```
Cirrus/Cirrus/
├── Services/
│   └── RcloneService.swift                    # MODIFY — add listRemotes
└── Views/MainWindow/Profiles/
    ├── ProfileListView.swift                  # MODIFY — replace placeholder
    ├── ProfileFormView.swift                  # NEW
    └── ActionSelectorView.swift               # NEW
CirrusTests/Services/
    └── RcloneServiceTests.swift               # MODIFY — add listRemotes tests
```

### Technical Requirements

**Remote discovery — `rclone listremotes`:**
- Output format: one remote per line, colon-terminated: `gdrive:\nmydropbox:\n`
- Strip trailing colon to get clean names: `["gdrive", "mydropbox"]`
- Run via `RcloneService` using the rclone path from `AppSettings`
- Cache results for the duration of the form being open — don't re-run on every keystroke

**Combo box for remote selection:**
- SwiftUI doesn't have a native combo box. Options:
  - `Picker` with `.menu` style + separate text field for manual entry
  - Or wrap `NSComboBox` via `NSViewRepresentable` for true combo box behavior
- The manual text entry (FR7) is critical — user must be able to type remotes not in the list

**Folder picker:**
```swift
let panel = NSOpenPanel()
panel.canChooseFiles = false
panel.canChooseDirectories = true
panel.allowsMultipleSelection = false
if panel.runModal() == .OK { sourcePath = panel.url?.path ?? "" }
```

**Action descriptions (FR13):**
| Action | Description |
|--------|------------|
| sync | Make destination identical to source, deleting extra files |
| copy | Copy files from source to destination, skipping existing |
| move | Move files from source to destination, deleting from source |
| delete | Delete files from destination that match the patterns |

**Ignore patterns dynamic list:**
- Array of `String` in the profile
- UI: List with text field per row + "+" button to add + "-" button per row to remove
- Empty patterns are filtered out on save

**Form validation:** Name, sourcePath, remoteName, and action are required. Show inline validation messages for missing fields. Don't allow save until all required fields have values.

### Enforcement Rules

- Remote discovery goes through `RcloneService` — no direct process spawning in views
- Views access `ProfileStore` via `@Environment` — not init parameters
- Form submission calls `profileStore.save()` — view contains no persistence logic
- `NSOpenPanel` for file/folder pickers — not custom file browsers

### Dependencies

- **Depends on:** Story 2.1 (RcloneService), Story 2.2 (Profile model, ProfileStore), Story 1.3 (ProfileListView placeholder)
- **Does NOT depend on:** Stories 2.4-2.5 or Epics 3-5

### References

- [Source: architecture.md#Data Architecture] — Profile struct fields
- [Source: architecture.md#Project Structure & Boundaries] — Views/MainWindow/Profiles
- [Source: architecture.md#View Patterns] — sheets, no AnyView, @Environment
- [Source: epics.md#Story 2.3] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
