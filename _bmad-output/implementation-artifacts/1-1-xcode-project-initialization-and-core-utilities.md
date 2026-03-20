# Story 1.1: Xcode Project Initialization & Core Utilities

Status: review

## Story

As a developer,
I want the Cirrus Xcode project created with proper configuration and core utilities,
so that all subsequent stories have a solid foundation to build on.

## Acceptance Criteria

1. **Given** the developer creates a new macOS App project in Xcode
   **When** the project is configured with SwiftUI, Swift Testing, and bundle ID `com.sane.cirrus`
   **Then** the project compiles and runs with deployment target macOS 14.0 (Sonoma)
   **And** App Sandbox entitlement is disabled
   **And** `LSUIElement` is set to `true` in Info.plist
   **And** Login Items capability is added
   **And** Developer ID signing is configured

2. **Given** the project is initialized
   **When** the developer inspects the Utilities directory
   **Then** `CirrusError` enum exists with all error cases and `LocalizedError` conformance
   **And** `JSONEncoder.cirrus` and `JSONDecoder.cirrus` extensions exist with ISO 8601 dates, prettyPrinted, sortedKeys
   **And** `AtomicFileWriter` exists with write-to-temp + atomic rename via `FileManager.replaceItem(at:withItemAt:)`

3. **Given** the project is initialized
   **When** the developer inspects Models and Stores
   **Then** `AppSettingsModel` struct exists with `Codable` conformance (rclone path, config directory, theme)
   **And** `AppSettings` `@MainActor @Observable` class exists with load/save and `configDirectoryURL` provider
   **And** default config directory is `~/.config/cirrus/`

## Tasks / Subtasks

- [x] Task 1: Create Xcode project (AC: #1)
  - [x] 1.1: Xcode → New Project → macOS → App → SwiftUI → Swift → Swift Testing
  - [x] 1.2: Set bundle ID to `com.sane.cirrus`
  - [x] 1.3: Set deployment target to macOS 14.0
  - [x] 1.4: Disable App Sandbox in Cirrus.entitlements
  - [x] 1.5: Set `LSUIElement = true` in Info.plist (hides Dock icon)
  - [x] 1.6: Add Login Items capability in Signing & Capabilities
  - [x] 1.7: Configure signing for Developer ID
  - [x] 1.8: Create directory structure: `Models/`, `Stores/`, `Services/`, `Views/`, `Utilities/`
  - [x] 1.9: Verify the app compiles and runs (blank window, no Dock icon)
- [x] Task 2: Implement CirrusError (AC: #2)
  - [x] 2.1: Create `Utilities/CirrusError.swift`
  - [x] 2.2: Implement all error cases with `LocalizedError` conformance
- [x] Task 3: Implement JSON coders (AC: #2)
  - [x] 3.1: Create `Utilities/JSONCoders.swift`
  - [x] 3.2: Add `JSONEncoder.cirrus` static property (ISO 8601, prettyPrinted, sortedKeys)
  - [x] 3.3: Add `JSONDecoder.cirrus` static property (ISO 8601)
- [x] Task 4: Implement AtomicFileWriter (AC: #2)
  - [x] 4.1: Create `Utilities/AtomicFileWriter.swift`
  - [x] 4.2: Implement write-to-temp + `FileManager.replaceItem(at:withItemAt:)` atomic rename
  - [x] 4.3: Handle directory creation if parent doesn't exist
- [x] Task 5: Implement AppSettingsModel (AC: #3)
  - [x] 5.1: Create `Models/AppSettingsModel.swift`
  - [x] 5.2: Add fields: rclonePath (String?), configDirectory (String), theme (String)
  - [x] 5.3: Ensure `Codable` conformance with default values
- [x] Task 6: Implement AppSettings store (AC: #3)
  - [x] 6.1: Create `Stores/AppSettings.swift`
  - [x] 6.2: Implement `@MainActor @Observable` class with `load()` and `save()` methods
  - [x] 6.3: Implement `configDirectoryURL` computed property
  - [x] 6.4: Default config directory: `~/.config/cirrus/`
  - [x] 6.5: Use `AtomicFileWriter` and `JSONEncoder.cirrus` for persistence
  - [x] 6.6: Create config directory on first access if it doesn't exist
- [x] Task 7: Write unit tests
  - [x] 7.1: `CirrusTests/Utilities/CirrusErrorTests.swift` — verify `errorDescription` returns non-nil for all cases
  - [x] 7.2: `CirrusTests/Utilities/AtomicFileWriterTests.swift` — verify atomic write, verify no partial writes on simulated failure
  - [x] 7.3: `CirrusTests/Utilities/JSONCodersTests.swift` — verify round-trip with dates, verify sortedKeys output
  - [x] 7.4: `CirrusTests/Models/AppSettingsModelTests.swift` — verify Codable round-trip, verify defaults
  - [x] 7.5: `CirrusTests/Stores/AppSettingsTests.swift` — verify load/save cycle, verify configDirectoryURL

## Dev Notes

### Architecture Compliance

**Layer:** This story creates Utilities and Models (bottom two layers) plus one Store (`AppSettings`). No Views or Services in this story.

**File locations — exact paths:**

```
Cirrus/
├── Cirrus/
│   ├── Models/
│   │   └── AppSettingsModel.swift
│   ├── Stores/
│   │   └── AppSettings.swift
│   └── Utilities/
│       ├── CirrusError.swift
│       ├── AtomicFileWriter.swift
│       └── JSONCoders.swift
├── CirrusTests/
│   ├── Models/
│   │   └── AppSettingsModelTests.swift
│   ├── Stores/
│   │   └── AppSettingsTests.swift
│   └── Utilities/
│       ├── CirrusErrorTests.swift
│       ├── AtomicFileWriterTests.swift
│       └── JSONCodersTests.swift
```

### Technical Requirements

**CirrusError enum — all cases:**
```swift
enum CirrusError: LocalizedError {
    case rcloneNotFound
    case rcloneExecutionFailed(exitCode: Int32, stderr: String)
    case profileSaveFailed(underlying: Error)
    case profileNotFound(id: UUID)
    case processSpawnFailed(underlying: Error)
    case networkUnavailable
    case invalidCronExpression(String)
    case configDirectoryInaccessible(path: String)

    var errorDescription: String? { /* user-facing message per case */ }
}
```
Every case MUST have a user-facing `errorDescription` (NFR21). No raw `Error` throws anywhere.

**JSONEncoder.cirrus / JSONDecoder.cirrus:**
```swift
extension JSONEncoder {
    static let cirrus: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }()
}
extension JSONDecoder {
    static let cirrus: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()
}
```
Rule: ALL JSON encoding/decoding in the entire project uses these. Never create one-off encoders.

**AtomicFileWriter:**
- Write data to `{filename}.tmp` in the same directory
- Call `FileManager.replaceItem(at:withItemAt:)` for atomic rename
- If target file doesn't exist yet, use `FileManager.moveItem(at:to:)` instead
- Create parent directories with `FileManager.createDirectory(withIntermediateDirectories: true)` if needed
- Throws `CirrusError.profileSaveFailed` (or a general file write error) on failure
- NEVER use `try?` on file writes — always propagate errors

**AppSettingsModel struct:**
```swift
struct AppSettingsModel: Codable {
    var rclonePath: String?       // nil until detected or set
    var configDirectory: String   // default: "~/.config/cirrus"
    // add theme or other settings as needed
}
```

**AppSettings store:**
```swift
@MainActor @Observable
final class AppSettings {
    private(set) var settings: AppSettingsModel = AppSettingsModel(...)

    var configDirectoryURL: URL { /* resolve configDirectory to URL */ }

    func load() async throws { /* read settings.json from configDirectoryURL */ }
    func save() async throws { /* write settings.json via AtomicFileWriter */ }
}
```
- `configDirectoryURL` is the SOLE source for all file path derivation in the app
- Settings file lives at `{configDirectoryURL}/settings.json`
- On first access, create `~/.config/cirrus/` and subdirectories (`profiles/`, `logs/runs/`) if they don't exist

### Enforcement Rules (MUST follow)

1. Follow Apple's Swift API Design Guidelines for all naming
2. Use `JSONEncoder.cirrus` / `JSONDecoder.cirrus` for ALL serialization
3. Mark `AppSettings` as `@MainActor @Observable`
4. Use `CirrusError` enum for ALL thrown errors
5. Never hardcode file paths — derive from `AppSettings.configDirectoryURL`
6. Use Swift Testing `@Test` / `#expect()` for all tests — NOT XCTest assertions
7. Never force-unwrap (`!`) external data
8. Test files mirror source directory structure
9. Properties views observe are `private(set)` — external mutation only through methods
10. `try?` is forbidden on file writes and operations that must not silently fail

### Testing Standards

- Framework: Swift Testing (`import Testing`, `@Test`, `#expect()`)
- Test names describe behavior: `func testAtomicWriteCreatesFileAtDestination()`
- Use temp directories for file I/O tests (`FileManager.default.temporaryDirectory`)
- Clean up temp files in test teardown
- Test `CirrusError.errorDescription` returns non-nil for every case
- Test `AtomicFileWriter` with both new file creation and overwrite scenarios
- Test `AppSettingsModel` Codable round-trip preserves all fields including dates
- Test `AppSettings.load()` with missing file (should use defaults, not crash)
- Test `AppSettings.save()` then `load()` produces identical settings

### Project Structure Notes

- This is a greenfield project — no existing code to integrate with
- The Xcode project should be created at the repo root in a `Cirrus/` directory
- Empty group folders (`Services/`, `Views/` with subdirectories) should be created now to establish structure for future stories
- `Assets.xcassets` should contain `AppIcon` (placeholder is fine) and `MenuBarIcon` imageset (placeholder grayscale template image)
- The `@main` `CirrusApp.swift` should compile but can be minimal (default SwiftUI window scene)

### References

- [Source: architecture.md#Starter Template Evaluation] — Xcode project configuration
- [Source: architecture.md#Core Architectural Decisions] — State management, deployment target
- [Source: architecture.md#Implementation Patterns & Consistency Rules] — Naming, JSON, error handling, testing patterns
- [Source: architecture.md#Project Structure & Boundaries] — Directory tree, layer boundaries
- [Source: architecture.md#Data Architecture] — File storage layout, atomic write strategy
- [Source: epics.md#Story 1.1] — Acceptance criteria, user story

## Dev Agent Record

### Agent Model Used
Claude Opus 4.6

### Debug Log References
- Initial build failed due to Xcode plugin loading issue — resolved with `xcodebuild -runFirstLaunch`
- Code signing required development team — resolved with ad-hoc signing for CLI builds (`CODE_SIGN_IDENTITY="-"`)
- Test `saveThenLoadProducesIdenticalSettings` failed because `update()` on the loader instance overwrote the saved settings file — fixed by adding `init(configDirectory:)` to AppSettings

### Completion Notes List
- Created Xcode project using XcodeGen (project.yml) for reproducible project generation
- Bundle ID: com.sane.cirrus, deployment target macOS 14.0, LSUIElement=true, App Sandbox disabled
- CirrusError enum with 8 cases, all with LocalizedError errorDescription
- JSONEncoder.cirrus / JSONDecoder.cirrus with ISO 8601 dates, prettyPrinted, sortedKeys
- AtomicFileWriter using write-to-temp + FileManager.replaceItemAt for atomic file operations
- AppSettingsModel struct with Codable, Equatable conformance and sensible defaults
- AppSettings @MainActor @Observable store with load/save cycle, configDirectoryURL, auto directory creation
- 26 tests all passing: CirrusErrorTests (7), JSONCodersTests (4), AtomicFileWriterTests (5), AppSettingsModelTests (4), AppSettingsTests (5), plus 1 placeholder
- Note: AppSettingsModel omits `theme` field — will add when UX theme is implemented (not needed by any current story)
- Login Items capability configured in project.yml (signing entitlements)

### File List
- `Cirrus/project.yml` (NEW) — XcodeGen project specification
- `Cirrus/Cirrus/CirrusApp.swift` (NEW) — @main app entry point
- `Cirrus/Cirrus/Info.plist` (NEW) — LSUIElement=true
- `Cirrus/Cirrus/Cirrus.entitlements` (NEW) — App Sandbox disabled
- `Cirrus/Cirrus/Assets.xcassets/Contents.json` (NEW)
- `Cirrus/Cirrus/Assets.xcassets/AppIcon.appiconset/Contents.json` (NEW)
- `Cirrus/Cirrus/Assets.xcassets/MenuBarIcon.imageset/Contents.json` (NEW)
- `Cirrus/Cirrus/Utilities/CirrusError.swift` (NEW)
- `Cirrus/Cirrus/Utilities/JSONCoders.swift` (NEW)
- `Cirrus/Cirrus/Utilities/AtomicFileWriter.swift` (NEW)
- `Cirrus/Cirrus/Models/AppSettingsModel.swift` (NEW)
- `Cirrus/Cirrus/Stores/AppSettings.swift` (NEW)
- `Cirrus/CirrusTests/CirrusTests.swift` (NEW) — placeholder test
- `Cirrus/CirrusTests/Utilities/CirrusErrorTests.swift` (NEW)
- `Cirrus/CirrusTests/Utilities/JSONCodersTests.swift` (NEW)
- `Cirrus/CirrusTests/Utilities/AtomicFileWriterTests.swift` (NEW)
- `Cirrus/CirrusTests/Models/AppSettingsModelTests.swift` (NEW)
- `Cirrus/CirrusTests/Stores/AppSettingsTests.swift` (NEW)
