import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Where an unsaved session lives on disk, so a crash can't cost the user their
/// work. The work file holds every staged change; the manifest records which
/// document it belongs to.
///
/// This is a plain namespace rather than a type because there is exactly one
/// recovery slot: the app is effectively single-document.
enum Recovery {
    /// `~/Library/Application Support/AnyToneMac/Recovery`, unless
    /// `ANYTONE_RECOVERY_DIR` overrides it. The override exists so tests can
    /// exercise the real recovery lifecycle without touching — or destroying —
    /// a genuine unsaved session belonging to the installed app.
    static var directory: URL {
        if let override = ProcessInfo.processInfo.environment["ANYTONE_RECOVERY_DIR"] {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("AnyToneMac/Recovery", isDirectory: true)
    }

    static var workURL: URL { directory.appendingPathComponent("work.bin") }
    static var manifestURL: URL { directory.appendingPathComponent("manifest.json") }

    /// Points a recovered work file back at the document it was edited from.
    struct Manifest: Codable {
        let originalPath: String
        let modifiedAt: Date
    }

    /// Create the recovery directory if it isn't there yet.
    static func ensureDirectory() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    /// The pending session, if a previous run left one behind and its original
    /// document still exists. A manifest pointing at a deleted or moved file is
    /// treated as stale and cleared, since Save would have nowhere to go.
    static func pending() -> Manifest? {
        guard let data = try? Data(contentsOf: manifestURL),
              let manifest = try? JSONDecoder().decode(Manifest.self, from: data),
              FileManager.default.fileExists(atPath: workURL.path),
              FileManager.default.fileExists(atPath: manifest.originalPath)
        else {
            clear()
            return nil
        }
        return manifest
    }

    /// Record that `workURL` holds unsaved changes to `original`.
    static func mark(original: URL) throws {
        try ensureDirectory()
        let manifest = Manifest(originalPath: original.path, modifiedAt: Date())
        try JSONEncoder().encode(manifest).write(to: manifestURL, options: .atomic)
    }

    /// Drop the "there is unsaved work here" marker, keeping the work file.
    /// Used after open and save: the work file is the live staging buffer and
    /// must survive, but there is nothing to recover at that moment.
    static func clearManifest() {
        try? FileManager.default.removeItem(at: manifestURL)
    }

    /// Drop the pending session entirely, work file included. Only for
    /// discarding recovered work — during normal editing the work file is the
    /// staging buffer and deleting it would strand the session.
    static func clear() {
        clearManifest()
        try? FileManager.default.removeItem(at: workURL)
    }
}

/// State for the offline codeplug editor.
///
/// The user's document is never written until `save()`. Every staged change is
/// applied to a work file (`Recovery.workURL`) instead, and the in-memory
/// records are always a dump of that work file. This keeps the Rust core as the
/// single source of truth for slot allocation and record encoding — the Swift
/// side never has to model a half-applied edit.
///
/// Applying edits repeatedly to the work file is lossless: `Codeplug::serialize`
/// starts from the parsed raw image and patches only the slots it touched, so an
/// untouched record round-trips byte-for-byte.
@MainActor
final class CodeplugStore: ObservableObject {
    /// The user's document. Written only by `save()`.
    @Published private(set) var fileURL: URL?
    @Published private(set) var channels: [DumpChannel] = []
    @Published private(set) var zones: [DumpZone] = []
    @Published private(set) var scanLists: [DumpScanList] = []
    /// Read-only APRS settings (no edit path — the block is never written).
    @Published private(set) var aprs: DumpAprs?
    @Published private(set) var contacts: [DumpContact] = []
    @Published private(set) var groupLists: [DumpGroupList] = []
    @Published private(set) var radioIds: [DumpRadioID] = []

    /// True when the work file holds changes the document doesn't have yet.
    @Published private(set) var isDirty = false

    /// Slots holding staged, unsaved changes, per section. Drives the per-row
    /// indicator in the tables.
    @Published private(set) var unsavedSlots: [CodeplugSection: Set<Int>] = [:]

    /// Sections holding any staged change. This can't be derived from
    /// `unsavedSlots`: a removal is a real unsaved change but leaves no row
    /// behind to mark, and the sidebar still has to show the section as edited.
    @Published private(set) var unsavedSections: Set<CodeplugSection> = []

    @Published private(set) var statusMessage: String?
    @Published var errorMessage: String?

    /// Whether an undo or redo is currently available. Drives the Edit-menu
    /// items; the stacks themselves are private.
    @Published private(set) var canUndo = false
    @Published private(set) var canRedo = false

    /// Undo/redo history as full work-file snapshots. Each staged edit pushes the
    /// pre-edit state; undo/redo swap whole snapshots in and out. Snapshotting the
    /// bytes rather than inverting each operation keeps the Rust core the single
    /// source of truth and makes every edit — add, remove, move, bulk — reversible
    /// through one path.
    private var undoStack: [EditSnapshot] = []
    private var redoStack: [EditSnapshot] = []

    /// A previous session's unsaved work, found at launch. The UI offers to
    /// restore it; `nil` once handled.
    @Published var pendingRecovery: Recovery.Manifest?

    /// Cancels the previous auto-clear when a new status arrives, so messages
    /// don't wink out early.
    private var statusClearTask: Task<Void, Never>?

    // MARK: - Opening

    /// Look for unsaved work left by a previous run. Call once at launch.
    func checkForRecovery() {
        pendingRecovery = Recovery.pending()
    }

    /// Adopt a recovered session: the work file already holds the changes, so
    /// dump it as-is and come up dirty.
    func restoreRecovery(_ manifest: Recovery.Manifest) {
        let original = URL(fileURLWithPath: manifest.originalPath)
        do {
            load(try RadioCore.dump(binPath: Recovery.workURL.path), url: original)
            isDirty = true
            // The work file records the result of the recovered edits, not which
            // slots they touched, so every section is flagged and no row is. The
            // alternative — showing nothing as edited — would be a lie.
            unsavedSlots = [:]
            unsavedSections = Set(CodeplugSection.allCases)
            clearHistory()
            pendingRecovery = nil
            setStatus("Restored unsaved changes to \(original.lastPathComponent)")
        } catch {
            errorMessage = "Could not restore the recovered session: \(error.localizedDescription)"
            Recovery.clear()
            pendingRecovery = nil
        }
    }

    /// Throw away a recovered session.
    func discardRecovery() {
        Recovery.clear()
        pendingRecovery = nil
    }

    /// Ask which codeplug .bin to open, then open it. Lives on the store because
    /// the toolbar, the File menu, and the empty state all need it.
    func openWithPanel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [UTType(filenameExtension: "bin") ?? .data]
        panel.allowsMultipleSelection = false
        panel.title = "Open Codeplug"
        if panel.runModal() == .OK, let url = panel.url {
            open(url: url)
        }
    }

    /// Open and parse a codeplug .bin (offline, no radio), seeding the work file
    /// with a pristine copy.
    func open(url: URL) {
        // Reject an incompatible file up front. A codeplug from a different radio
        // model or an older firmware has a different total size, so a length
        // mismatch is a clear "wrong version" signal — and catching it here gives
        // the user a plain-language reason instead of a low-level parse error
        // after the file has already been copied into the work slot.
        if let incompatibility = incompatibilityReason(for: url) {
            errorMessage = incompatibility
            return
        }
        do {
            try Recovery.ensureDirectory()
            try replaceFile(at: Recovery.workURL, withContentsOf: url)
            load(try RadioCore.dump(binPath: Recovery.workURL.path), url: url)
            isDirty = false
            clearUnsaved()
            clearHistory()
            Recovery.clearManifest()
            setStatus("Opened \(url.lastPathComponent): \(channels.count) channels, "
                + "\(zones.count) zones, \(contacts.count) contacts, "
                + "\(groupLists.count) group lists, \(radioIds.count) radio IDs")
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// A plain-language reason `url` is not a codeplug this build can open, or
    /// nil if its size matches what the core expects. Split out of `open` so the
    /// version gate can be exercised in tests without a modal panel.
    func incompatibilityReason(for url: URL) -> String? {
        let expected = RadioCore.expectedCodeplugSize
        guard let size = try? FileManager.default
            .attributesOfItem(atPath: url.path)[.size] as? Int else {
            return nil // Unreadable size: let the normal open path report the I/O error.
        }
        guard size != expected else { return nil }
        return "“\(url.lastPathComponent)” isn’t a compatible codeplug. It is "
            + "\(size) bytes, but this app expects \(expected) bytes for the AnyTone "
            + "D878UV. It looks like it came from a different radio model or an older, "
            + "incompatible firmware version."
    }

    /// Close the open codeplug and return to the empty state. Clears the staging
    /// work file and undo history too, so nothing from this document lingers to
    /// be picked up or half-applied later. Callers guard against discarding
    /// unsaved changes before calling this.
    func close() {
        fileURL = nil
        channels = []
        zones = []
        scanLists = []
        aprs = nil
        contacts = []
        groupLists = []
        radioIds = []
        isDirty = false
        errorMessage = nil
        clearUnsaved()
        clearHistory()
        Recovery.clear()
        setStatus("Closed codeplug")
    }

    /// Populate the records from a parsed dump. Split out of `open` so it can be
    /// driven from a synthetic dump without the FFI or a real file.
    func load(_ dump: CodeplugDump, url: URL) {
        channels = dump.channels
        zones = dump.zones
        scanLists = dump.scanLists
        aprs = dump.aprs
        contacts = dump.contacts
        groupLists = dump.groupLists
        radioIds = dump.radioIds
        fileURL = url
        errorMessage = nil
    }

    // MARK: - Saving

    /// Write the staged work file over the user's document.
    func save() {
        guard let url = fileURL, isDirty else { return }
        write(to: url)
    }

    /// Ask for a destination, write the staged state there, and continue editing
    /// that document instead of the old one.
    ///
    /// Unlike `save()` this runs even with nothing staged: "save this codeplug
    /// under a new name" is a reasonable thing to want from an unmodified file.
    func saveAs() {
        guard fileURL != nil else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [UTType(filenameExtension: "bin") ?? .data]
        panel.nameFieldStringValue = fileURL?.lastPathComponent ?? "codeplug.bin"
        panel.title = "Save Codeplug As"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        write(to: url)
    }

    /// Write the work file's bytes to `url` and adopt it as the open document.
    ///
    /// Copying the bytes rather than `replaceItem`-ing the work file into place
    /// covers both callers with one path: Save As targets a file that may not
    /// exist yet, which `replaceItem` rejects. It also leaves the work file
    /// intact, so editing continues without re-seeding it.
    ///
    /// Internal rather than private so tests can drive a Save As without
    /// standing up a modal panel.
    func write(to url: URL) {
        do {
            try Data(contentsOf: Recovery.workURL).write(to: url, options: .atomic)
            fileURL = url
            isDirty = false
            clearUnsaved()
            Recovery.clearManifest()
            setStatus("Saved \(url.lastPathComponent)")
        } catch {
            errorMessage = "Could not save \(url.lastPathComponent): \(error.localizedDescription)"
        }
    }

    /// Throw away every staged change and reload the document from disk.
    func discardChanges() {
        guard let url = fileURL else { return }
        open(url: url)
        setStatus("Discarded unsaved changes to \(url.lastPathComponent)")
    }

    // MARK: - Field edits

    /// Stage an edited record. Each looks the record up by slot, not by array
    /// position, so a sorted or filtered table can't commit to the wrong row.

    func update(_ ch: DumpChannel) {
        guard let old = channels.first(where: { $0.index == ch.index }), old != ch else { return }
        var e = ChannelEdit(index: ch.index)
        if ch.name != old.name { e.name = ch.name }
        if ch.rxFrequencyHz != old.rxFrequencyHz { e.rxFrequencyHz = ch.rxFrequencyHz }
        if ch.txFrequencyHz != old.txFrequencyHz { e.txFrequencyHz = ch.txFrequencyHz }
        if ch.mode != old.mode { e.mode = editValue(ch.mode) }
        if ch.power != old.power { e.power = editValue(ch.power) }
        if ch.bandwidth != old.bandwidth { e.bandwidth = editValue(ch.bandwidth) }
        if ch.colorCode != old.colorCode { e.colorCode = ch.colorCode }
        if ch.timeSlot != old.timeSlot { e.timeSlot = ch.timeSlot }
        if ch.contactIndex != old.contactIndex { e.contactIndex = ch.contactIndex }
        if ch.radioIdIndex != old.radioIdIndex { e.radioIdIndex = ch.radioIdIndex }
        if ch.groupListIndex != old.groupListIndex { e.groupListIndex = ch.groupListIndex }
        if ch.rxSignalingMode != old.rxSignalingMode { e.rxSignalingMode = snakeEditValue(ch.rxSignalingMode) }
        if ch.txSignalingMode != old.txSignalingMode { e.txSignalingMode = snakeEditValue(ch.txSignalingMode) }
        if ch.rxCtcss != old.rxCtcss { e.rxCtcss = ch.rxCtcss }
        if ch.txCtcss != old.txCtcss { e.txCtcss = ch.txCtcss }
        if ch.rxDcs != old.rxDcs { e.rxDcs = ch.rxDcs }
        if ch.txDcs != old.txDcs { e.txDcs = ch.txDcs }
        if ch.squelchMode != old.squelchMode { e.squelchMode = snakeEditValue(ch.squelchMode) }
        if ch.optionalSignaling != old.optionalSignaling { e.optionalSignaling = snakeEditValue(ch.optionalSignaling) }
        if ch.admit != old.admit { e.admit = snakeEditValue(ch.admit) }
        if ch.scanListIndex != old.scanListIndex { e.scanListIndex = ch.scanListIndex }
        if ch.dtmfIdIndex != old.dtmfIdIndex { e.dtmfIdIndex = ch.dtmfIdIndex }
        if ch.twoToneIdIndex != old.twoToneIdIndex { e.twoToneIdIndex = ch.twoToneIdIndex }
        if ch.fiveToneIdIndex != old.fiveToneIdIndex { e.fiveToneIdIndex = ch.fiveToneIdIndex }
        if ch.twoToneDecodeIndex != old.twoToneDecodeIndex { e.twoToneDecodeIndex = ch.twoToneDecodeIndex }
        if ch.rxOnly != old.rxOnly { e.rxOnly = ch.rxOnly }
        if ch.talkAround != old.talkAround { e.talkAround = ch.talkAround }
        if ch.callConfirm != old.callConfirm { e.callConfirm = ch.callConfirm }
        if ch.workAlone != old.workAlone { e.workAlone = ch.workAlone }
        if ch.simplexTdma != old.simplexTdma { e.simplexTdma = ch.simplexTdma }
        if ch.rxAprs != old.rxAprs { e.rxAprs = ch.rxAprs }
        if stage({ $0.channels.append(e) }, "Updated channel \(formatSlot(ch.index))") {
            markUnsaved(.channels, slots: [ch.index])
        }
    }

    func update(_ sl: DumpScanList) {
        guard let old = scanLists.first(where: { $0.index == sl.index }), old != sl else { return }
        var e = ScanListEdit(index: sl.index)
        if sl.name != old.name { e.name = sl.name }
        if sl.members != old.members { e.members = sl.members }
        if sl.priorityChannelSelect != old.priorityChannelSelect { e.priorityChannelSelect = sl.priorityChannelSelect }
        if sl.priorityChannel1 != old.priorityChannel1 { e.priorityChannel1 = sl.priorityChannel1 }
        if sl.priorityChannel2 != old.priorityChannel2 { e.priorityChannel2 = sl.priorityChannel2 }
        if sl.lookBackA != old.lookBackA { e.lookBackA = sl.lookBackA }
        if sl.lookBackB != old.lookBackB { e.lookBackB = sl.lookBackB }
        if sl.dropoutDelay != old.dropoutDelay { e.dropoutDelay = sl.dropoutDelay }
        if sl.dwellTime != old.dwellTime { e.dwellTime = sl.dwellTime }
        if sl.revertChannel != old.revertChannel { e.revertChannel = sl.revertChannel }
        if stage({ $0.scanLists.append(e) }, "Updated scan list \(formatSlot(sl.index))") {
            markUnsaved(.scanLists, slots: [sl.index])
        }
    }

    func update(_ z: DumpZone) {
        guard let old = zones.first(where: { $0.index == z.index }), old != z else { return }
        var e = ZoneEdit(index: z.index)
        if z.name != old.name { e.name = z.name }
        if z.channels != old.channels { e.members = z.channels }
        if stage({ $0.zones.append(e) }, "Updated zone \(formatSlot(z.index))") {
            markUnsaved(.zones, slots: [z.index])
        }
    }

    func update(_ c: DumpContact) {
        guard let old = contacts.first(where: { $0.index == c.index }), old != c else { return }
        var e = ContactEdit(index: c.index)
        if c.name != old.name { e.name = c.name }
        if c.number != old.number { e.number = c.number }
        if c.callType != old.callType { e.callType = editValue(c.callType) }
        if stage({ $0.contacts.append(e) }, "Updated contact \(formatSlot(c.index))") {
            markUnsaved(.contacts, slots: [c.index])
        }
    }

    func update(_ g: DumpGroupList) {
        guard let old = groupLists.first(where: { $0.index == g.index }), old != g else { return }
        var e = GroupListEdit(index: g.index)
        if g.name != old.name { e.name = g.name }
        if g.members != old.members { e.members = g.members }
        if stage({ $0.groupLists.append(e) }, "Updated group list \(formatSlot(g.index))") {
            markUnsaved(.groupLists, slots: [g.index])
        }
    }

    func update(_ r: DumpRadioID) {
        guard let old = radioIds.first(where: { $0.index == r.index }), old != r else { return }
        var e = RadioIDEdit(index: r.index)
        if r.name != old.name { e.name = r.name }
        if r.number != old.number { e.number = r.number }
        if stage({ $0.radioIds.append(e) }, "Updated radio ID \(formatSlot(r.index))") {
            markUnsaved(.radioIds, slots: [r.index])
        }
    }

    /// Stage a bulk update to multiple channels at once. Each field in
    /// `update` that is non-nil is applied to every listed channel index.
    func bulkUpdateChannels(_ update: BulkChannelUpdate) {
        guard !update.indices.isEmpty else { return }
        var spec = EditSpec()
        for index in update.indices {
            var e = ChannelEdit(index: index)
            if let v = update.mode { e.mode = editValue(v) }
            if let v = update.power { e.power = editValue(v) }
            if let v = update.bandwidth { e.bandwidth = editValue(v) }
            if let v = update.colorCode { e.colorCode = v }
            if let v = update.timeSlot { e.timeSlot = v }
            if let v = update.contactIndex { e.contactIndex = v }
            if let v = update.groupListIndex { e.groupListIndex = v }
            if let v = update.radioIdIndex { e.radioIdIndex = v }
            spec.channels.append(e)
        }
        if bulkStage(spec, "Bulk-updated \(update.indices.count) channels") {
            markUnsaved(.channels, slots: update.indices)
        }
    }

    // MARK: - Add / remove

    /// Each add returns the slot the core placed the record in, so the caller can
    /// select it. The FFI doesn't report the new index, so it's recovered by
    /// diffing the slot set across the reload.

    func addChannel() -> Int? {
        // A fresh channel points at RX group list "None" (0xff) rather than the
        // core's default of slot 0, which would silently attach group list #1.
        addedSlot(of: \.channels, section: .channels, noun: "channel") {
            $0.addChannels.append(ChannelEdit(groupListIndex: 255))
        }
    }

    func addZone() -> Int? {
        addedSlot(of: \.zones, section: .zones, noun: "zone") { $0.addZones.append(ZoneEdit()) }
    }

    func addScanList() -> Int? {
        addedSlot(of: \.scanLists, section: .scanLists, noun: "scan list") { $0.addScanLists.append(ScanListEdit()) }
    }

    func addContact() -> Int? {
        addedSlot(of: \.contacts, section: .contacts, noun: "contact") { $0.addContacts.append(ContactEdit(callType: "group")) }
    }

    func addGroupList() -> Int? {
        addedSlot(of: \.groupLists, section: .groupLists, noun: "group list") { $0.addGroupLists.append(GroupListEdit()) }
    }

    func addRadioId() -> Int? {
        addedSlot(of: \.radioIds, section: .radioIds, noun: "radio ID") { $0.addRadioIds.append(RadioIDEdit()) }
    }

    func removeChannel(_ i: Int) {
        if stage({ $0.removeChannels.append(i) }, "Removed channel \(formatSlot(i))") {
            markRemoved(.channels, slot: i)
        }
    }

    func removeZone(_ i: Int) {
        if stage({ $0.removeZones.append(i) }, "Removed zone \(formatSlot(i))") {
            markRemoved(.zones, slot: i)
        }
    }

    func removeScanList(_ i: Int) {
        if stage({ $0.removeScanLists.append(i) }, "Removed scan list \(formatSlot(i))") {
            markRemoved(.scanLists, slot: i)
        }
    }

    func removeContact(_ i: Int) {
        if stage({ $0.removeContacts.append(i) }, "Removed contact \(formatSlot(i))") {
            markRemoved(.contacts, slot: i)
        }
    }

    func removeGroupList(_ i: Int) {
        if stage({ $0.removeGroupLists.append(i) }, "Removed group list \(formatSlot(i))") {
            markRemoved(.groupLists, slot: i)
        }
    }

    func removeRadioId(_ i: Int) {
        if stage({ $0.removeRadioIds.append(i) }, "Removed radio ID \(formatSlot(i))") {
            markRemoved(.radioIds, slot: i)
        }
    }

    // MARK: - Move

    /// Relocate a record to a free slot. The core rejects an occupied or
    /// out-of-range target, which surfaces as an error and leaves the work file
    /// untouched.

    func moveChannel(_ from: Int, to: Int) {
        if stage({ $0.moveChannels.append(MoveOp(from: from, to: to)) }, "Moved channel to #\(formatSlot(to))") {
            markMoved(.channels, from: from, to: to)
        }
    }

    func moveZone(_ from: Int, to: Int) {
        if stage({ $0.moveZones.append(MoveOp(from: from, to: to)) }, "Moved zone to #\(formatSlot(to))") {
            markMoved(.zones, from: from, to: to)
        }
    }

    func moveScanList(_ from: Int, to: Int) {
        if stage({ $0.moveScanLists.append(MoveOp(from: from, to: to)) }, "Moved scan list to #\(formatSlot(to))") {
            markMoved(.scanLists, from: from, to: to)
        }
    }

    func moveContact(_ from: Int, to: Int) {
        if stage({ $0.moveContacts.append(MoveOp(from: from, to: to)) }, "Moved contact to #\(formatSlot(to))") {
            markMoved(.contacts, from: from, to: to)
        }
    }

    func moveGroupList(_ from: Int, to: Int) {
        if stage({ $0.moveGroupLists.append(MoveOp(from: from, to: to)) }, "Moved group list to #\(formatSlot(to))") {
            markMoved(.groupLists, from: from, to: to)
        }
    }

    func moveRadioId(_ from: Int, to: Int) {
        if stage({ $0.moveRadioIds.append(MoveOp(from: from, to: to)) }, "Moved radio ID to #\(formatSlot(to))") {
            markMoved(.radioIds, from: from, to: to)
        }
    }

    // MARK: - Unsaved-change tracking

    /// True when `slot` in `section` holds a staged change the document doesn't
    /// have yet. Backs the per-row indicator in the tables.
    func isUnsaved(_ section: CodeplugSection, slot: Int) -> Bool {
        unsavedSlots[section]?.contains(slot) ?? false
    }

    /// True when `section` holds any staged change. Backs the sidebar dot.
    func hasUnsavedChanges(_ section: CodeplugSection) -> Bool {
        unsavedSections.contains(section)
    }

    private func markUnsaved(_ section: CodeplugSection, slots: [Int]) {
        unsavedSlots[section, default: []].formUnion(slots)
        unsavedSections.insert(section)
    }

    /// A removed record has no row left to mark, so only the section is flagged.
    private func markRemoved(_ section: CodeplugSection, slot: Int) {
        unsavedSlots[section]?.remove(slot)
        unsavedSections.insert(section)
    }

    /// The record's edited state travels with it: the vacated slot is clean (it
    /// holds nothing now) and the target slot carries the change.
    private func markMoved(_ section: CodeplugSection, from: Int, to: Int) {
        unsavedSlots[section]?.remove(from)
        markUnsaved(section, slots: [to])
    }

    private func clearUnsaved() {
        unsavedSlots = [:]
        unsavedSections = []
    }

    // MARK: - Undo / redo

    /// A full staged-state snapshot: the work-file bytes plus the unsaved-change
    /// bookkeeping that rides alongside them. Restoring one returns the editor to
    /// exactly the state it was in when the snapshot was taken.
    private struct EditSnapshot {
        let workData: Data
        let isDirty: Bool
        let unsavedSlots: [CodeplugSection: Set<Int>]
        let unsavedSections: Set<CodeplugSection>
    }

    /// Reverse the last staged edit, if any.
    func undo() {
        guard let url = fileURL, let previous = undoStack.popLast() else { return }
        if let current = snapshot() { redoStack.append(current) }
        restore(previous, url: url, verb: "Undo")
        refreshUndoRedo()
    }

    /// Re-apply the last undone edit, if any.
    func redo() {
        guard let url = fileURL, let next = redoStack.popLast() else { return }
        if let current = snapshot() { undoStack.append(current) }
        restore(next, url: url, verb: "Redo")
        refreshUndoRedo()
    }

    /// Snapshot the current staged state. Returns nil if the work file can't be
    /// read, in which case the edit simply isn't made undoable.
    private func snapshot() -> EditSnapshot? {
        guard let workData = try? Data(contentsOf: Recovery.workURL) else { return nil }
        return EditSnapshot(workData: workData, isDirty: isDirty,
                            unsavedSlots: unsavedSlots, unsavedSections: unsavedSections)
    }

    /// Write a snapshot's bytes back to the work file and reload from them,
    /// restoring the change bookkeeping and recovery marker to match.
    private func restore(_ snap: EditSnapshot, url: URL, verb: String) {
        do {
            try snap.workData.write(to: Recovery.workURL, options: .atomic)
            load(try RadioCore.dump(binPath: Recovery.workURL.path), url: url)
            isDirty = snap.isDirty
            unsavedSlots = snap.unsavedSlots
            unsavedSections = snap.unsavedSections
            if isDirty {
                try? Recovery.mark(original: url)
            } else {
                Recovery.clearManifest()
            }
            setStatus(verb)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Drop all history. Called when a new document is opened or recovered, since
    /// a snapshot only makes sense against the document it was taken from.
    private func clearHistory() {
        undoStack.removeAll()
        redoStack.removeAll()
        refreshUndoRedo()
    }

    private func refreshUndoRedo() {
        canUndo = !undoStack.isEmpty
        canRedo = !redoStack.isEmpty
    }

    // MARK: - Internals

    /// Apply one operation to the work file and reload from it. The user's
    /// document is untouched; only `save()` writes there. Returns false when the
    /// core rejected the edit, so the caller doesn't record a change that didn't
    /// happen.
    @discardableResult
    private func stage(_ op: (inout EditSpec) -> Void, _ describe: String) -> Bool {
        var spec = EditSpec()
        op(&spec)
        return bulkStage(spec, describe)
    }

    /// Apply a pre-built edit spec to the work file and reload. Used directly by
    /// bulk operations, which build the full spec themselves.
    @discardableResult
    private func bulkStage(_ spec: EditSpec, _ describe: String) -> Bool {
        guard let url = fileURL else { return false }
        // Capture the pre-edit state before touching the work file, so a
        // successful edit can be pushed onto the undo stack.
        let pre = snapshot()
        do {
            try RadioCore.applyEdits(input: Recovery.workURL.path, spec: spec,
                                     output: Recovery.workURL.path)
            load(try RadioCore.dump(binPath: Recovery.workURL.path), url: url)
            isDirty = true
            try Recovery.mark(original: url)
            if let pre {
                undoStack.append(pre)
                redoStack.removeAll()
                refreshUndoRedo()
            }
            setStatus("\(describe) — unsaved")
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    /// Run `op`, then report which slot appeared in `list`. The core picks the
    /// slot (`first_free`) and the FFI doesn't report it back, so it's recovered
    /// by diffing the slot set. The status and the unsaved mark are set here
    /// rather than in `stage` because the slot isn't known until the reload has
    /// happened.
    private func addedSlot<T: Identifiable>(of list: KeyPath<CodeplugStore, [T]>,
                                            section: CodeplugSection, noun: String,
                                            _ op: (inout EditSpec) -> Void) -> Int? where T.ID == Int {
        let before = Set(self[keyPath: list].map(\.id))
        guard stage(op, "Added \(noun)") else { return nil }
        guard let slot = self[keyPath: list].map(\.id).first(where: { !before.contains($0) }) else {
            return nil
        }
        markUnsaved(section, slots: [slot])
        setStatus("Added \(noun) \(formatSlot(slot)) — unsaved")
        return slot
    }

    /// Show a status message, clearing it after a few seconds so stale text
    /// doesn't linger for the rest of the session.
    private func setStatus(_ message: String) {
        statusMessage = message
        statusClearTask?.cancel()
        statusClearTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 6_000_000_000)
            guard !Task.isCancelled else { return }
            self?.statusMessage = nil
        }
    }

    /// Copy `source` over `destination`, replacing whatever was there.
    private func replaceFile(at destination: URL, withContentsOf source: URL) throws {
        try? FileManager.default.removeItem(at: destination)
        try FileManager.default.copyItem(at: source, to: destination)
    }
}
