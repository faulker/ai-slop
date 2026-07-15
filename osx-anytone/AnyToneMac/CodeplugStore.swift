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
    @Published private(set) var contacts: [DumpContact] = []
    @Published private(set) var groupLists: [DumpGroupList] = []
    @Published private(set) var radioIds: [DumpRadioID] = []

    /// True when the work file holds changes the document doesn't have yet.
    @Published private(set) var isDirty = false

    @Published private(set) var statusMessage: String?
    @Published var errorMessage: String?

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
        do {
            try Recovery.ensureDirectory()
            try replaceFile(at: Recovery.workURL, withContentsOf: url)
            load(try RadioCore.dump(binPath: Recovery.workURL.path), url: url)
            isDirty = false
            Recovery.clearManifest()
            setStatus("Opened \(url.lastPathComponent): \(channels.count) channels, "
                + "\(zones.count) zones, \(contacts.count) contacts, "
                + "\(groupLists.count) group lists, \(radioIds.count) radio IDs")
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Populate the records from a parsed dump. Split out of `open` so it can be
    /// driven from a synthetic dump without the FFI or a real file.
    func load(_ dump: CodeplugDump, url: URL) {
        channels = dump.channels
        zones = dump.zones
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
        do {
            try FileManager.default.replaceItem(at: url, withItemAt: Recovery.workURL,
                                                backupItemName: nil, options: [],
                                                resultingItemURL: nil)
            // `replaceItem` consumes the work file, so re-seed it from the saved
            // document to keep editing without reopening.
            try replaceFile(at: Recovery.workURL, withContentsOf: url)
            isDirty = false
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
        stage({ $0.channels.append(e) }, "Updated channel \(formatSlot(ch.index))")
    }

    func update(_ z: DumpZone) {
        guard let old = zones.first(where: { $0.index == z.index }), old != z else { return }
        var e = ZoneEdit(index: z.index)
        if z.name != old.name { e.name = z.name }
        if z.channels != old.channels { e.members = z.channels }
        stage({ $0.zones.append(e) }, "Updated zone \(formatSlot(z.index))")
    }

    func update(_ c: DumpContact) {
        guard let old = contacts.first(where: { $0.index == c.index }), old != c else { return }
        var e = ContactEdit(index: c.index)
        if c.name != old.name { e.name = c.name }
        if c.number != old.number { e.number = c.number }
        if c.callType != old.callType { e.callType = editValue(c.callType) }
        stage({ $0.contacts.append(e) }, "Updated contact \(formatSlot(c.index))")
    }

    func update(_ g: DumpGroupList) {
        guard let old = groupLists.first(where: { $0.index == g.index }), old != g else { return }
        var e = GroupListEdit(index: g.index)
        if g.name != old.name { e.name = g.name }
        if g.members != old.members { e.members = g.members }
        stage({ $0.groupLists.append(e) }, "Updated group list \(formatSlot(g.index))")
    }

    func update(_ r: DumpRadioID) {
        guard let old = radioIds.first(where: { $0.index == r.index }), old != r else { return }
        var e = RadioIDEdit(index: r.index)
        if r.name != old.name { e.name = r.name }
        if r.number != old.number { e.number = r.number }
        stage({ $0.radioIds.append(e) }, "Updated radio ID \(formatSlot(r.index))")
    }

    // MARK: - Add / remove

    /// Each add returns the slot the core placed the record in, so the caller can
    /// select it. The FFI doesn't report the new index, so it's recovered by
    /// diffing the slot set across the reload.

    func addChannel() -> Int? {
        addedSlot(of: \.channels, noun: "channel") { $0.addChannels.append(ChannelEdit()) }
    }

    func addZone() -> Int? {
        addedSlot(of: \.zones, noun: "zone") { $0.addZones.append(ZoneEdit()) }
    }

    func addContact() -> Int? {
        addedSlot(of: \.contacts, noun: "contact") { $0.addContacts.append(ContactEdit(callType: "group")) }
    }

    func addGroupList() -> Int? {
        addedSlot(of: \.groupLists, noun: "group list") { $0.addGroupLists.append(GroupListEdit()) }
    }

    func addRadioId() -> Int? {
        addedSlot(of: \.radioIds, noun: "radio ID") { $0.addRadioIds.append(RadioIDEdit()) }
    }

    func removeChannel(_ i: Int) { stage({ $0.removeChannels.append(i) }, "Removed channel \(formatSlot(i))") }
    func removeZone(_ i: Int) { stage({ $0.removeZones.append(i) }, "Removed zone \(formatSlot(i))") }
    func removeContact(_ i: Int) { stage({ $0.removeContacts.append(i) }, "Removed contact \(formatSlot(i))") }
    func removeGroupList(_ i: Int) { stage({ $0.removeGroupLists.append(i) }, "Removed group list \(formatSlot(i))") }
    func removeRadioId(_ i: Int) { stage({ $0.removeRadioIds.append(i) }, "Removed radio ID \(formatSlot(i))") }

    // MARK: - Move

    /// Relocate a record to a free slot. The core rejects an occupied or
    /// out-of-range target, which surfaces as an error and leaves the work file
    /// untouched.

    func moveChannel(_ from: Int, to: Int) { stage({ $0.moveChannels.append(MoveOp(from: from, to: to)) }, "Moved channel to #\(formatSlot(to))") }
    func moveZone(_ from: Int, to: Int) { stage({ $0.moveZones.append(MoveOp(from: from, to: to)) }, "Moved zone to #\(formatSlot(to))") }
    func moveContact(_ from: Int, to: Int) { stage({ $0.moveContacts.append(MoveOp(from: from, to: to)) }, "Moved contact to #\(formatSlot(to))") }
    func moveGroupList(_ from: Int, to: Int) { stage({ $0.moveGroupLists.append(MoveOp(from: from, to: to)) }, "Moved group list to #\(formatSlot(to))") }
    func moveRadioId(_ from: Int, to: Int) { stage({ $0.moveRadioIds.append(MoveOp(from: from, to: to)) }, "Moved radio ID to #\(formatSlot(to))") }

    // MARK: - Internals

    /// Apply one operation to the work file and reload from it. The user's
    /// document is untouched; only `save()` writes there.
    private func stage(_ op: (inout EditSpec) -> Void, _ describe: String) {
        guard let url = fileURL else { return }
        var spec = EditSpec()
        op(&spec)
        do {
            try RadioCore.applyEdits(input: Recovery.workURL.path, spec: spec,
                                     output: Recovery.workURL.path)
            load(try RadioCore.dump(binPath: Recovery.workURL.path), url: url)
            isDirty = true
            try Recovery.mark(original: url)
            setStatus("\(describe) — unsaved")
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Run `op`, then report which slot appeared in `list`. The core picks the
    /// slot (`first_free`) and the FFI doesn't report it back, so it's recovered
    /// by diffing the slot set. The status is set here rather than in `stage`
    /// because the slot isn't known until the reload has happened.
    private func addedSlot<T: Identifiable>(of list: KeyPath<CodeplugStore, [T]>, noun: String,
                                            _ op: (inout EditSpec) -> Void) -> Int? where T.ID == Int {
        let before = Set(self[keyPath: list].map(\.id))
        stage(op, "Added \(noun)")
        guard let slot = self[keyPath: list].map(\.id).first(where: { !before.contains($0) }) else {
            return nil
        }
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
