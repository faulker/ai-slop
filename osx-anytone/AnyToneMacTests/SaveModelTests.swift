import XCTest
import CryptoKit
@testable import AnyToneMac

/// End-to-end tests of the save model, driving the real Rust core against real
/// files. These exist because the central promise of the editor — "nothing
/// touches your codeplug until you press Save" — is a claim about bytes on
/// disk, and only a real file can prove it.
///
/// `ANYTONE_RECOVERY_DIR` points the recovery lifecycle at a temp directory so
/// a test run can't disturb a real unsaved session.
@MainActor
final class SaveModelTests: XCTestCase {

    private var tempDir: URL!
    private var codeplug: URL!

    /// An all-zero image of the exact codeplug size parses as a valid, empty
    /// codeplug — the same synthetic fixture the Rust tests use, so no real
    /// radio data is involved.
    private static let codeplugSize = 1_656_032

    override func setUpWithError() throws {
        try super.setUpWithError()
        tempDir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("anytone-save-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)

        setenv("ANYTONE_RECOVERY_DIR", tempDir.appendingPathComponent("recovery").path, 1)
        Recovery.clear()

        codeplug = tempDir.appendingPathComponent("codeplug.bin")
        try Data(count: Self.codeplugSize).write(to: codeplug)
    }

    override func tearDownWithError() throws {
        Recovery.clear()
        unsetenv("ANYTONE_RECOVERY_DIR")
        try? FileManager.default.removeItem(at: tempDir)
        try super.tearDownWithError()
    }

    private func sha(_ url: URL) throws -> String {
        SHA256.hash(data: try Data(contentsOf: url)).map { String(format: "%02x", $0) }.joined()
    }

    // MARK: - The core promise

    /// Adding a zone must not touch the user's file. This is the regression the
    /// whole work-file design exists to prevent: the previous build wrote the
    /// .bin in place the instant you clicked Add, with no backup and no undo.
    func testAddingAZoneDoesNotWriteTheDocument() throws {
        let before = try sha(codeplug)
        let store = CodeplugStore()
        store.open(url: codeplug)
        XCTAssertNil(store.errorMessage)

        let slot = store.addZone()

        XCTAssertNotNil(slot, "add should report the slot the core chose")
        XCTAssertTrue(store.isDirty)
        XCTAssertEqual(store.zones.count, 1, "the staged zone is visible in the UI")
        XCTAssertEqual(try sha(codeplug), before, "the document must be byte-identical until Save")
    }

    /// Save is what commits, and it must land the staged change.
    func testSaveWritesStagedChangesToTheDocument() throws {
        let before = try sha(codeplug)
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        store.save()

        XCTAssertNil(store.errorMessage)
        XCTAssertFalse(store.isDirty)
        XCTAssertNotEqual(try sha(codeplug), before, "Save must write the document")

        // Reopening from disk proves the change persisted, not just that bytes moved.
        let reopened = CodeplugStore()
        reopened.open(url: codeplug)
        XCTAssertEqual(reopened.zones.count, 1)
        XCTAssertFalse(reopened.isDirty)
    }

    /// Editing a record stages through the core and survives a save.
    func testEditingAZoneNameStagesThenPersists() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        let before = try sha(codeplug)

        var zone = try XCTUnwrap(store.zones.first)
        zone.name = "Local Simplex"
        store.update(zone)

        XCTAssertEqual(store.zones.first?.name, "Local Simplex")
        XCTAssertEqual(try sha(codeplug), before, "a field edit must not write the document either")

        store.save()
        let reopened = CodeplugStore()
        reopened.open(url: codeplug)
        XCTAssertEqual(reopened.zones.first?.name, "Local Simplex")
    }

    /// Discard throws the work file away and returns to what's on disk.
    func testDiscardRevertsToTheSavedDocument() throws {
        let before = try sha(codeplug)
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        XCTAssertTrue(store.isDirty)

        store.discardChanges()

        XCTAssertFalse(store.isDirty)
        XCTAssertEqual(store.zones.count, 0, "the staged zone is gone")
        XCTAssertEqual(try sha(codeplug), before)
    }

    /// Saving with nothing staged must not rewrite the file.
    func testSaveWithNoChangesIsANoOp() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        let before = try sha(codeplug)

        store.save()

        XCTAssertEqual(try sha(codeplug), before)
    }

    // MARK: - Crash recovery

    /// After a change, the recovery manifest describes the session. This is what
    /// a fresh launch finds after a crash — no clean shutdown runs in between.
    func testUnsavedChangesLeaveARecoverableSession() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        var zone = try XCTUnwrap({ _ = store.addZone(); return store.zones.first }())
        zone.name = "Repeaters"
        store.update(zone)

        let pending = try XCTUnwrap(Recovery.pending(), "a crash must leave a recoverable session")
        XCTAssertEqual(pending.originalPath, codeplug.path)

        // A brand-new store, as if the app had just relaunched.
        let relaunched = CodeplugStore()
        relaunched.checkForRecovery()
        let manifest = try XCTUnwrap(relaunched.pendingRecovery)
        relaunched.restoreRecovery(manifest)

        XCTAssertNil(relaunched.errorMessage)
        XCTAssertEqual(relaunched.zones.first?.name, "Repeaters", "the edit survived the crash")
        XCTAssertTrue(relaunched.isDirty, "restored work is still unsaved")
        XCTAssertEqual(relaunched.fileURL, codeplug)
    }

    /// A clean save leaves nothing to recover — otherwise every launch would
    /// prompt about work the user already committed.
    func testSavingClearsTheRecoverableSession() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        XCTAssertNotNil(Recovery.pending())

        store.save()

        XCTAssertNil(Recovery.pending())
    }

    func testDiscardingRecoveryClearsIt() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()

        let relaunched = CodeplugStore()
        relaunched.checkForRecovery()
        XCTAssertNotNil(relaunched.pendingRecovery)
        relaunched.discardRecovery()

        XCTAssertNil(relaunched.pendingRecovery)
        XCTAssertNil(Recovery.pending())
    }

    /// A manifest pointing at a file that's since been moved or deleted is
    /// stale: Save would have nowhere to write, so it must not be offered.
    func testRecoveryIsDroppedWhenTheOriginalIsGone() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        try FileManager.default.removeItem(at: codeplug)

        XCTAssertNil(Recovery.pending())
    }

    // MARK: - Editing continues after a save

    /// `replaceItem` consumes the work file, so the store re-seeds it. Without
    /// that, the first edit after a save would fail.
    func testEditingWorksAfterASave() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        store.save()

        let slot = store.addZone()

        XCTAssertNotNil(slot, "a second add after saving must still work")
        XCTAssertNil(store.errorMessage)
        XCTAssertEqual(store.zones.count, 2)
        XCTAssertTrue(store.isDirty)
    }

    /// Removing is staged like everything else.
    func testRemoveIsStagedNotWritten() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        store.save()
        let saved = try sha(codeplug)

        store.removeZone(try XCTUnwrap(store.zones.first).index)

        XCTAssertEqual(store.zones.count, 0)
        XCTAssertEqual(try sha(codeplug), saved, "the removal must not reach the document")
        store.save()
        XCTAssertNotEqual(try sha(codeplug), saved)
    }

    /// A rejected move surfaces an error and leaves the staged state alone,
    /// rather than silently corrupting the work file.
    func testMoveToAnOccupiedSlotIsRejected() throws {
        let store = CodeplugStore()
        store.open(url: codeplug)
        _ = store.addZone()
        _ = store.addZone()
        XCTAssertEqual(store.zones.count, 2)

        store.moveZone(0, to: 1)

        XCTAssertNotNil(store.errorMessage, "moving onto an occupied slot must report an error")
        XCTAssertEqual(store.zones.count, 2)
    }
}
