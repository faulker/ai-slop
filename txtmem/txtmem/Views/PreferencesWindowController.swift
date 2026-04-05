import Cocoa
import Carbon.HIToolbox

final class PreferencesWindowController: NSWindowController {
    static let shared = PreferencesWindowController()

    private let quickCaptureRecorder = ShortcutRecorderView()
    private let detailedCaptureRecorder = ShortcutRecorderView()
    private let startAtLoginCheckbox = NSButton(checkboxWithTitle: "Start ThoughtQueue at login", target: nil, action: nil)

    convenience init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 450, height: 240),
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

        startAtLoginCheckbox.translatesAutoresizingMaskIntoConstraints = false
        startAtLoginCheckbox.state = PreferencesManager.shared.startAtLogin ? .on : .off
        startAtLoginCheckbox.target = self
        startAtLoginCheckbox.action = #selector(startAtLoginToggled)

        let noteLabel = NSTextField(wrappingLabelWithString: "Click a shortcut field and press your desired key combination. Changes take effect after restarting the app.")
        noteLabel.font = .systemFont(ofSize: 11)
        noteLabel.textColor = .secondaryLabelColor
        noteLabel.translatesAutoresizingMaskIntoConstraints = false

        contentView.addSubview(quickLabel)
        contentView.addSubview(quickCaptureRecorder)
        contentView.addSubview(detailedLabel)
        contentView.addSubview(detailedCaptureRecorder)
        contentView.addSubview(startAtLoginCheckbox)
        contentView.addSubview(noteLabel)

        NSLayoutConstraint.activate([
            quickLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            quickLabel.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 30),
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

            startAtLoginCheckbox.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            startAtLoginCheckbox.topAnchor.constraint(equalTo: detailedLabel.bottomAnchor, constant: 20),

            noteLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            noteLabel.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -20),
            noteLabel.topAnchor.constraint(equalTo: startAtLoginCheckbox.bottomAnchor, constant: 20),
        ])
    }
    @objc private func startAtLoginToggled(_ sender: NSButton) {
        PreferencesManager.shared.startAtLogin = sender.state == .on
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
            label.stringValue = "Press shortcut…"
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
        if binding.modifiers.contains(.maskControl) { parts.append("⌃") }
        if binding.modifiers.contains(.maskAlternate) { parts.append("⌥") }
        if binding.modifiers.contains(.maskShift) { parts.append("⇧") }
        if binding.modifiers.contains(.maskCommand) { parts.append("⌘") }

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
            0x24: "↩", 0x30: "⇥", 0x31: "␣", 0x33: "⌫",
            0x35: "⎋", 0x7A: "F1", 0x78: "F2", 0x63: "F3",
            0x76: "F4", 0x60: "F5", 0x61: "F6", 0x62: "F7",
            0x64: "F8", 0x65: "F9", 0x6D: "F10", 0x67: "F11",
            0x6F: "F12",
        ]
        return mapping[keyCode] ?? "?"
    }
}
