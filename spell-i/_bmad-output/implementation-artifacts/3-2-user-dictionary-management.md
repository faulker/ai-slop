# Story 3.2: User Dictionary Management

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a user,
I want to add words like "kubectl" to my personal dictionary so they're never flagged again,
so that false positives decrease over time and the app learns my vocabulary.

## Acceptance Criteria

1. **Add Word Flow:** When the user clicks "Add to Dictionary" in the popup, the word is added to the Harper `MutableDictionary` and the underline disappears instantly for all instances of that word. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.2]
2. **Global Recognition:** Once added, the word is never flagged as an error in any application. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.2]
3. **Persistence:** The user dictionary is stored at `~/Library/Application Support/Spell-i/dictionary.txt` with one word per line. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.2]
4. **Atomic Writes:** Dictionary writes are atomic (temp file + rename) to prevent data loss. [Source: _bmad-output/planning-artifacts/architecture.md#Decision Priority Analysis]
5. **Session Recovery:** Previously added words are loaded and recognized when the app relaunches. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.2]
6. **Robustness:** If the Application Support directory doesn't exist, the app creates it and an empty `dictionary.txt` on first launch. [Source: _bmad-output/planning-artifacts/epics.md#Story 3.2]

## Tasks / Subtasks

- [x] **Task 1: Rust Dictionary Persistence (AC: 3, 4, 5, 6)**
  - [x] Create `spell-i-engine/src/user_dict.rs`.
  - [x] Implement `UserDict` struct with `load()` and `save()` methods.
  - [x] Implement atomic file writing logic in Rust.
  - [x] Integrate `UserDict` into `SpellEngine` initialization.
- [x] **Task 2: FFI Bridge Extension (AC: 1)**
  - [x] Add `add_user_word` and `remove_user_word` to the `ffi` block in `lib.rs`.
  - [x] Implement these methods in `SpellEngine` to update both the in-memory linter and the file.
- [x] **Task 3: Coordinator Integration (AC: 1, 2)**
  - [x] Implement `addWordToDictionary(_:)` in `TextMonitorCoordinator`.
  - [x] Dispatch the call to the serial background queue.
  - [x] Trigger a re-lint immediately after adding a word.
- [x] **Task 4: Performance Check (AC: 1)**
  - [x] Verify that dictionary addition results in immediate UI updates.

## Dev Notes

- **Architecture:** The user dictionary lives in Rust for performance and direct access by the linter. [Source: _bmad-output/planning-artifacts/architecture.md#User Dictionary Persistence]
- **Privacy:** The dictionary is the only data persisted by Spell-i. Ensure no other metadata or text is saved.
- **Reliability:** Handle file I/O errors gracefully (never-fail principle).

### Project Structure Notes

- `spell-i-engine/src/user_dict.rs`
- `spell-i-engine/src/lib.rs`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`

### References

- [Source: _bmad-output/planning-artifacts/architecture.md]
- [Source: _bmad-output/planning-artifacts/epics.md]
- [Source: _bmad-output/planning-artifacts/prd.md]

## Dev Agent Record

### Agent Model Used

Gemini 2.0 Flash

### Debug Log References

- `UserDict` unit tests verified atomic write and case-insensitive duplication handling.
- `SpellEngine` re-initializes linter upon dictionary modification.
- `TextMonitorCoordinator` successfully dispatches dictionary updates to the background queue.

### Completion Notes List

- Persistent user dictionary implemented in Rust.
- Atomic file operations ensure no data loss on unexpected quits.
- FFI bridge extended to support word addition and removal.
- Coordinator updated to trigger re-lints for immediate visual feedback after additions.
- **Code Review Fixes:**
  - Hardened dictionary persistence with explicit `flush()` and `sync_all()` calls to ensure data integrity during atomic renames.
  - Optimized in-memory word storage using `HashSet` for O(1) duplicate checks and lookups.
  - Improved error handling for directory creation and file I/O within the Rust engine.

### File List

- `spell-i-engine/src/user_dict.rs`
- `spell-i-engine/src/lib.rs`
- `Spell-i/TextMonitoring/TextMonitorCoordinator.swift`

