import Cocoa
import Carbon.HIToolbox
import UniformTypeIdentifiers

final class PreferencesWindowController: NSWindowController {
    static let shared = PreferencesWindowController()

    private let quickCaptureRecorder = ShortcutRecorderView()
    private let detailedCaptureRecorder = ShortcutRecorderView()
    private let startAtLoginCheckbox = NSButton(checkboxWithTitle: "Start ThoughtQueue at login", target: nil, action: nil)

    convenience init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 450, height: 340),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "ThoughtQueue Preferences"
        window.center()

        self.init(window: window)
        setupUI()
    }

    private func setupUI() {
        guard let contentView = window?.contentView else { return }

        // --- Start at login ---

        startAtLoginCheckbox.translatesAutoresizingMaskIntoConstraints = false
        startAtLoginCheckbox.state = PreferencesManager.shared.startAtLogin ? .on : .off
        startAtLoginCheckbox.target = self
        startAtLoginCheckbox.action = #selector(startAtLoginToggled)

        let divider1 = NSBox()
        divider1.boxType = .separator
        divider1.translatesAutoresizingMaskIntoConstraints = false

        // --- Hotkeys ---

        let quickLabel = NSTextField(labelWithString: "Quick Capture:")
        quickLabel.font = .systemFont(ofSize: 13)
        quickLabel.translatesAutoresizingMaskIntoConstraints = false

        let detailedLabel = NSTextField(labelWithString: "Detailed Capture:")
        detailedLabel.font = .systemFont(ofSize: 13)
        detailedLabel.translatesAutoresizingMaskIntoConstraints = false

        quickCaptureRecorder.translatesAutoresizingMaskIntoConstraints = false
        detailedCaptureRecorder.translatesAutoresizingMaskIntoConstraints = false

        quickCaptureRecorder.keyBinding = PreferencesManager.shared.quickCaptureKey
        detailedCaptureRecorder.keyBinding = PreferencesManager.shared.detailedCaptureKey

        quickCaptureRecorder.onChanged = { binding in
            PreferencesManager.shared.quickCaptureKey = binding
        }
        detailedCaptureRecorder.onChanged = { binding in
            PreferencesManager.shared.detailedCaptureKey = binding
        }

        let noteLabel = NSTextField(wrappingLabelWithString: "Click a shortcut field and press your desired key combination. Changes take effect after restarting the app.")
        noteLabel.font = .systemFont(ofSize: 11)
        noteLabel.textColor = .secondaryLabelColor
        noteLabel.translatesAutoresizingMaskIntoConstraints = false

        // --- Backup / Restore ---

        let divider2 = NSBox()
        divider2.boxType = .separator
        divider2.translatesAutoresizingMaskIntoConstraints = false

        let backupLabel = NSTextField(labelWithString: "Data:")
        backupLabel.font = .systemFont(ofSize: 13)
        backupLabel.translatesAutoresizingMaskIntoConstraints = false

        let backupBtn = NSButton(title: "Backup\u{2026}", target: self, action: #selector(backupData))
        backupBtn.bezelStyle = .rounded
        backupBtn.translatesAutoresizingMaskIntoConstraints = false

        let restoreBtn = NSButton(title: "Restore\u{2026}", target: self, action: #selector(restoreData))
        restoreBtn.bezelStyle = .rounded
        restoreBtn.translatesAutoresizingMaskIntoConstraints = false

        let backupNote = NSTextField(wrappingLabelWithString: "Backup saves all entries and categories to a JSON file. Restore imports entries and categories from a backup.")
        backupNote.font = .systemFont(ofSize: 11)
        backupNote.textColor = .secondaryLabelColor
        backupNote.translatesAutoresizingMaskIntoConstraints = false

        // --- Add subviews ---

        contentView.addSubview(startAtLoginCheckbox)
        contentView.addSubview(divider1)
        contentView.addSubview(quickLabel)
        contentView.addSubview(quickCaptureRecorder)
        contentView.addSubview(detailedLabel)
        contentView.addSubview(detailedCaptureRecorder)
        contentView.addSubview(noteLabel)
        contentView.addSubview(divider2)
        contentView.addSubview(backupLabel)
        contentView.addSubview(backupBtn)
        contentView.addSubview(restoreBtn)
        contentView.addSubview(backupNote)

        NSLayoutConstraint.activate([
            startAtLoginCheckbox.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            startAtLoginCheckbox.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 20),

            divider1.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            divider1.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -20),
            divider1.topAnchor.constraint(equalTo: startAtLoginCheckbox.bottomAnchor, constant: 16),

            quickLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            quickLabel.topAnchor.constraint(equalTo: divider1.bottomAnchor, constant: 16),
            quickLabel.widthAnchor.constraint(equalToConstant: 140),

            quickCaptureRecorder.leadingAnchor.constraint(equalTo: quickLabel.trailingAnchor, constant: 8),
            quickCaptureRecorder.centerYAnchor.constraint(equalTo: quickLabel.centerYAnchor),
            quickCaptureRecorder.widthAnchor.constraint(equalToConstant: 200),
            quickCaptureRecorder.heightAnchor.constraint(equalToConstant: 24),

            detailedLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            detailedLabel.topAnchor.constraint(equalTo: quickLabel.bottomAnchor, constant: 20),
            detailedLabel.widthAnchor.constraint(equalToConstant: 140),

            detailedCaptureRecorder.leadingAnchor.constraint(equalTo: detailedLabel.trailingAnchor, constant: 8),
            detailedCaptureRecorder.centerYAnchor.constraint(equalTo: detailedLabel.centerYAnchor),
            detailedCaptureRecorder.widthAnchor.constraint(equalToConstant: 200),
            detailedCaptureRecorder.heightAnchor.constraint(equalToConstant: 24),

            noteLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            noteLabel.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -20),
            noteLabel.topAnchor.constraint(equalTo: detailedLabel.bottomAnchor, constant: 16),

            divider2.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            divider2.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -20),
            divider2.topAnchor.constraint(equalTo: noteLabel.bottomAnchor, constant: 16),

            backupLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            backupLabel.topAnchor.constraint(equalTo: divider2.bottomAnchor, constant: 16),

            backupBtn.leadingAnchor.constraint(equalTo: backupLabel.trailingAnchor, constant: 12),
            backupBtn.centerYAnchor.constraint(equalTo: backupLabel.centerYAnchor),

            restoreBtn.leadingAnchor.constraint(equalTo: backupBtn.trailingAnchor, constant: 8),
            restoreBtn.centerYAnchor.constraint(equalTo: backupLabel.centerYAnchor),

            backupNote.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            backupNote.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -20),
            backupNote.topAnchor.constraint(equalTo: backupLabel.bottomAnchor, constant: 10),
        ])
    }

    @objc private func startAtLoginToggled(_ sender: NSButton) {
        PreferencesManager.shared.startAtLogin = sender.state == .on
    }

    // MARK: - Backup

    @objc private func backupData() {
        let entries = DatabaseManager.shared.fetchEntries()
        let categories = DatabaseManager.shared.fetchCategories()

        let formatter = ISO8601DateFormatter()

        let catData: [[String: Any]] = categories.map { cat in
            [
                "id": cat.id,
                "name": cat.name,
                "createdAt": formatter.string(from: cat.createdAt),
            ]
        }

        let entryData: [[String: Any]] = entries.map { entry in
            var dict: [String: Any] = [
                "id": entry.id,
                "text": entry.text,
                "isSent": entry.isSent,
                "createdAt": formatter.string(from: entry.createdAt),
                "updatedAt": formatter.string(from: entry.updatedAt),
            ]
            if let catId = entry.categoryId {
                dict["categoryId"] = catId
            }
            if let catName = entry.categoryName {
                dict["categoryName"] = catName
            }
            return dict
        }

        let backup: [String: Any] = [
            "version": 1,
            "exportedAt": formatter.string(from: Date()),
            "categories": catData,
            "entries": entryData,
        ]

        let savePanel = NSSavePanel()
        savePanel.allowedContentTypes = [UTType.json]
        savePanel.nameFieldStringValue = "thoughtqueue-backup.json"

        if savePanel.runModal() == .OK, let url = savePanel.url {
            do {
                let jsonData = try JSONSerialization.data(withJSONObject: backup, options: [.prettyPrinted, .sortedKeys])
                try jsonData.write(to: url)
                ToastWindow.show(message: "Backed up \(entries.count) entries, \(categories.count) categories")
            } catch {
                NSLog("[ThoughtQueue] Backup failed: %@", error.localizedDescription)
                ToastWindow.show(message: "Backup failed")
            }
        }
    }

    // MARK: - Restore

    @objc private func restoreData() {
        let openPanel = NSOpenPanel()
        openPanel.allowedContentTypes = [UTType.json]
        openPanel.allowsMultipleSelection = false

        guard openPanel.runModal() == .OK, let url = openPanel.url else { return }

        do {
            let data = try Data(contentsOf: url)
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                ToastWindow.show(message: "Invalid backup file")
                return
            }

            let alert = NSAlert()
            alert.messageText = "Restore from Backup?"
            alert.informativeText = "This will import categories and entries from the backup. Existing data will not be deleted — duplicates may be created."
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Restore")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return }

            let formatter = ISO8601DateFormatter()
            var categoryIdMap: [Int64: Int64] = [:]  // old ID -> new ID

            // Restore categories first
            if let cats = json["categories"] as? [[String: Any]] {
                for cat in cats {
                    guard let name = cat["name"] as? String else { continue }
                    let oldId = (cat["id"] as? Int64) ?? (cat["id"] as? Int).map(Int64.init) ?? 0
                    if let newCat = DatabaseManager.shared.createCategory(name: name) {
                        categoryIdMap[oldId] = newCat.id
                    } else {
                        // Category may already exist — find it
                        let existing = DatabaseManager.shared.fetchCategories()
                        if let match = existing.first(where: { $0.name == name }) {
                            categoryIdMap[oldId] = match.id
                        }
                    }
                }
            }

            // Restore entries
            var importedCount = 0
            if let entries = json["entries"] as? [[String: Any]] {
                for entry in entries {
                    guard let text = entry["text"] as? String, !text.isEmpty else { continue }
                    let oldCatId = (entry["categoryId"] as? Int64) ?? (entry["categoryId"] as? Int).map(Int64.init)
                    let newCatId = oldCatId.flatMap { categoryIdMap[$0] }

                    if let _ = DatabaseManager.shared.createEntry(text: text, categoryId: newCatId) {
                        importedCount += 1

                        // Restore sent status if it was sent
                        if let isSent = entry["isSent"] as? Bool, isSent {
                            let entries = DatabaseManager.shared.fetchEntries()
                            if let latest = entries.first {
                                _ = DatabaseManager.shared.markEntryAsSent(id: latest.id)
                            }
                        }
                    }
                }
            }

            NotificationCenter.default.post(name: .entriesDidChange, object: nil)
            ToastWindow.show(message: "Restored \(importedCount) entries")

        } catch {
            NSLog("[ThoughtQueue] Restore failed: %@", error.localizedDescription)
            ToastWindow.show(message: "Restore failed: invalid file")
        }
    }
}

// MARK: - Shortcut Recorder

final class ShortcutRecorderView: NSView {
    var keyBinding: KeyBinding = .defaultQuickCapture {
        didSet { updateDisplay() }
    }
    var onChanged: ((KeyBinding) -> Void)?

    private let label = NSTextField(labelWithString: "")
    private var isRecording = false

    override init(frame: NSRect) {
        super.init(frame: frame)
        setupUI()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func setupUI() {
        wantsLayer = true
        layer?.borderWidth = 1
        layer?.borderColor = NSColor.separatorColor.cgColor
        layer?.cornerRadius = 4

        label.font = .systemFont(ofSize: 12)
        label.alignment = .center
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)

        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: centerXAnchor),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])

        updateDisplay()
    }

    private func updateDisplay() {
        if isRecording {
            label.stringValue = "Press shortcut\u{2026}"
            layer?.borderColor = NSColor.controlAccentColor.cgColor
        } else {
            label.stringValue = describeBinding(keyBinding)
            layer?.borderColor = NSColor.separatorColor.cgColor
        }
    }

    override func mouseDown(with event: NSEvent) {
        isRecording = true
        updateDisplay()
        window?.makeFirstResponder(self)
    }

    override var acceptsFirstResponder: Bool { true }

    override func keyDown(with event: NSEvent) {
        guard isRecording else { super.keyDown(with: event); return }

        let flags = event.modifierFlags.intersection([.command, .shift, .option, .control])
        guard !flags.isEmpty else { return }

        var cgFlags = CGEventFlags()
        if flags.contains(.command) { cgFlags.insert(.maskCommand) }
        if flags.contains(.shift) { cgFlags.insert(.maskShift) }
        if flags.contains(.option) { cgFlags.insert(.maskAlternate) }
        if flags.contains(.control) { cgFlags.insert(.maskControl) }

        let binding = KeyBinding(keyCode: Int64(event.keyCode), modifiers: cgFlags)
        keyBinding = binding
        isRecording = false
        updateDisplay()
        onChanged?(binding)
    }

    private func describeBinding(_ binding: KeyBinding) -> String {
        var parts: [String] = []
        if binding.modifiers.contains(.maskControl) { parts.append("\u{2303}") }
        if binding.modifiers.contains(.maskAlternate) { parts.append("\u{2325}") }
        if binding.modifiers.contains(.maskShift) { parts.append("\u{21E7}") }
        if binding.modifiers.contains(.maskCommand) { parts.append("\u{2318}") }

        let keyName = keyCodeToString(UInt16(binding.keyCode))
        parts.append(keyName)

        return parts.joined()
    }

    private func keyCodeToString(_ keyCode: UInt16) -> String {
        let mapping: [UInt16: String] = [
            0x00: "A", 0x01: "S", 0x02: "D", 0x03: "F", 0x04: "H",
            0x05: "G", 0x06: "Z", 0x07: "X", 0x08: "C", 0x09: "V",
            0x0B: "B", 0x0C: "Q", 0x0D: "W", 0x0E: "E", 0x0F: "R",
            0x10: "Y", 0x11: "T", 0x12: "1", 0x13: "2", 0x14: "3",
            0x15: "4", 0x17: "5", 0x16: "6", 0x1A: "7", 0x1C: "8",
            0x19: "9", 0x1D: "0", 0x1E: "]", 0x1F: "O", 0x20: "U",
            0x21: "[", 0x22: "I", 0x23: "P", 0x25: "L", 0x26: "J",
            0x28: "K", 0x2C: "/", 0x2D: "N", 0x2E: "M",
            0x24: "\u{21A9}", 0x30: "\u{21E5}", 0x31: "\u{2423}", 0x33: "\u{232B}",
            0x35: "\u{238B}", 0x7A: "F1", 0x78: "F2", 0x63: "F3",
            0x76: "F4", 0x60: "F5", 0x61: "F6", 0x62: "F7",
            0x64: "F8", 0x65: "F9", 0x6D: "F10", 0x67: "F11",
            0x6F: "F12",
        ]
        return mapping[keyCode] ?? "?"
    }
}
