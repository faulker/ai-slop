# Story 2.4: Paste-to-Create Profile

Status: ready-for-dev

## Story

As a power user,
I want to paste an rclone command and have it automatically parsed into a profile,
so that I can migrate my existing shell scripts in seconds.

## Acceptance Criteria

1. **Given** the user is on the profile creation screen
   **When** they switch to the "Paste Command" input mode
   **Then** a text area appears for pasting an rclone command (FR9)

2. **Given** the user pastes a valid rclone command (e.g., `rclone sync ~/docs gdrive:backup --exclude "*.tmp"`)
   **When** they trigger parsing
   **Then** the source path, remote name, remote path, action, ignore patterns, and flags are extracted and populate the profile form fields (FR9)
   **And** parsing completes within 1 second (NFR5)

3. **Given** the user pastes a command with `--exclude` or `--filter` flags
   **When** the command is parsed
   **Then** exclude patterns are extracted into the ignore patterns list
   **And** remaining flags are placed in the extra flags field

4. **Given** the user pastes a command that cannot be fully parsed
   **When** parsing encounters unknown syntax
   **Then** the parseable fields are populated and unparseable portions are placed in extra flags
   **And** the user can manually adjust any field before saving

5. **Given** the parsed fields populate the form
   **When** the user reviews and clicks Save
   **Then** the profile is created identically to a manually-created profile

## Tasks / Subtasks

- [ ] Task 1: Implement RcloneCommandParser (AC: #2, #3, #4)
  - [ ] 1.1: Create `Services/RcloneCommandParser.swift`
  - [ ] 1.2: Parse command structure: `rclone <action> <source> <remote:path> [flags]`
  - [ ] 1.3: Extract action from known actions: sync, copy, move, delete
  - [ ] 1.4: Extract source path (local path argument)
  - [ ] 1.5: Extract remote:path (split on first colon for remote name and path)
  - [ ] 1.6: Extract `--exclude` and `--filter` flags into ignore patterns
  - [ ] 1.7: Collect remaining flags into extraFlags string
  - [ ] 1.8: Handle edge cases: quoted paths, escaped spaces, multiple exclude flags
  - [ ] 1.9: Return a partially-filled Profile struct (or a parse result DTO)
- [ ] Task 2: Create PasteCommandView (AC: #1, #5)
  - [ ] 2.1: Create `Views/MainWindow/Profiles/PasteCommandView.swift`
  - [ ] 2.2: TextEditor for pasting the rclone command
  - [ ] 2.3: "Parse" button to trigger parsing
  - [ ] 2.4: On successful parse, switch to ProfileFormView with pre-populated fields
  - [ ] 2.5: Show inline error/warning if parsing partially fails
- [ ] Task 3: Add mode switcher to profile creation (AC: #1)
  - [ ] 3.1: Update ProfileFormView or ProfileListView to offer two creation modes: "Manual" and "Paste Command"
  - [ ] 3.2: Use segmented control or toggle to switch between modes
- [ ] Task 4: Write tests
  - [ ] 4.1: `CirrusTests/Services/RcloneCommandParserTests.swift` — extensive parameterized tests
  - [ ] 4.2: Test basic command: `rclone sync ~/docs gdrive:backup`
  - [ ] 4.3: Test with excludes: `rclone copy ~/photos remote:pics --exclude "*.tmp" --exclude ".DS_Store"`
  - [ ] 4.4: Test with flags: `rclone sync /data s3:bucket --verbose --dry-run`
  - [ ] 4.5: Test with quoted paths: `rclone sync "/path with spaces" gdrive:"folder name"`
  - [ ] 4.6: Test partial parse: unknown action preserved, unparseable parts go to extraFlags
  - [ ] 4.7: Test performance: parsing completes in <1s for complex commands (NFR5)

## Dev Notes

### Architecture Compliance

**Layer:** Services (RcloneCommandParser) + Views (PasteCommandView).

**File locations:**
```
Cirrus/Cirrus/
├── Services/
│   └── RcloneCommandParser.swift          # NEW
└── Views/MainWindow/Profiles/
    └── PasteCommandView.swift             # NEW
    └── ProfileFormView.swift              # MODIFY — add paste mode switcher
CirrusTests/Services/
    └── RcloneCommandParserTests.swift     # NEW
```

### Technical Requirements

**RcloneCommandParser — pure function, stateless:**
```swift
struct RcloneCommandParser {
    struct ParseResult {
        var action: RcloneAction?
        var sourcePath: String?
        var remoteName: String?
        var remotePath: String?
        var ignorePatterns: [String]
        var extraFlags: String
        var warnings: [String]     // parsing issues for user display
    }

    static func parse(_ command: String) -> ParseResult { ... }
}
```

**Parsing rules:**
1. Strip leading `rclone` if present
2. First token after `rclone` is the action (sync/copy/move/delete)
3. Next token is source path (may be quoted)
4. Next token is remote:path — split on FIRST colon for remoteName and remotePath
5. `--exclude "pattern"` and `--exclude="pattern"` → add to ignorePatterns
6. `--filter "rule"` → add to ignorePatterns
7. `--filter-from path` → add warning "filter-from not supported, add patterns manually"
8. All other flags → append to extraFlags string
9. Handle shell quoting: single quotes, double quotes, backslash escapes

**Edge cases to handle:**
- Command with no `rclone` prefix (just `sync ~/docs remote:path`)
- Multiple spaces between tokens
- Flags with `=` separator (`--bwlimit=10M`)
- Remote paths with colons in folder names (split on FIRST colon only)
- Empty command → return empty ParseResult with warning

**Performance (NFR5):** Parser is a pure string operation — no I/O, no Process calls. Will easily complete in <1s even for complex commands.

### Enforcement Rules

- `RcloneCommandParser` is stateless — pure function, no side effects
- Parser returns a result struct — views decide how to display warnings
- Parsed profile goes through the same `ProfileStore.save()` path as manual creation
- Heavily unit tested with parameterized tests (this is a pure function — ideal for testing)

### Dependencies

- **Depends on:** Story 2.2 (Profile model, RcloneAction), Story 2.3 (ProfileFormView)
- **Does NOT depend on:** Story 2.5 or Epics 3-5

### References

- [Source: architecture.md#Project Structure & Boundaries] — RcloneCommandParser in Services
- [Source: architecture.md#Testing Patterns] — parameterized tests for pure functions
- [Source: epics.md#Story 2.4] — Acceptance criteria

## Dev Agent Record

### Agent Model Used

### Debug Log References

### Completion Notes List

### File List
