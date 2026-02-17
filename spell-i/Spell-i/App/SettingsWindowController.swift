import AppKit

/// Presents a simple settings window for user-configurable options.
final class SettingsWindowController {

    private var window: NSWindow?

    func showWindow() {
        if let existing = window {
            existing.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }

        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 360, height: 140),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        w.title = "\(Constants.appName) Settings"
        w.isReleasedWhenClosed = false
        w.center()

        let contentView = NSView(frame: w.contentView!.bounds)
        contentView.autoresizingMask = [.width, .height]

        // --- Traversal depth row ---
        let depthLabel = NSTextField(labelWithString: "Max Traversal Depth:")
        depthLabel.frame = NSRect(x: 20, y: 88, width: 160, height: 20)
        contentView.addSubview(depthLabel)

        let depthField = NSTextField(frame: NSRect(x: 186, y: 86, width: 60, height: 24))
        depthField.integerValue = Settings.shared.maxTraversalDepth
        depthField.alignment = .center
        depthField.formatter = onlyIntegerFormatter(min: 4, max: 30)
        depthField.target = self
        depthField.action = #selector(depthFieldChanged(_:))
        contentView.addSubview(depthField)

        let stepper = NSStepper(frame: NSRect(x: 250, y: 86, width: 19, height: 24))
        stepper.minValue = 4
        stepper.maxValue = 30
        stepper.increment = 1
        stepper.integerValue = Settings.shared.maxTraversalDepth
        stepper.valueWraps = false
        stepper.target = self
        stepper.action = #selector(stepperChanged(_:))
        stepper.tag = 1
        contentView.addSubview(stepper)

        let hint = NSTextField(labelWithString: "Range: 4–30. Higher values help with Chromium/Electron apps.")
        hint.frame = NSRect(x: 20, y: 58, width: 320, height: 16)
        hint.font = NSFont.systemFont(ofSize: 11)
        hint.textColor = .secondaryLabelColor
        contentView.addSubview(hint)

        // --- Default button ---
        let defaultsButton = NSButton(title: "Reset to Default", target: self, action: #selector(resetDefaults(_:)))
        defaultsButton.frame = NSRect(x: 20, y: 16, width: 130, height: 28)
        defaultsButton.bezelStyle = .rounded
        contentView.addSubview(defaultsButton)

        w.contentView = contentView
        window = w

        w.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    // MARK: - Actions

    @objc private func stepperChanged(_ sender: NSStepper) {
        Settings.shared.maxTraversalDepth = sender.integerValue
        // Sync the text field
        if let contentView = window?.contentView {
            for subview in contentView.subviews {
                if let field = subview as? NSTextField, field.isEditable {
                    field.integerValue = sender.integerValue
                }
            }
        }
    }

    @objc private func depthFieldChanged(_ sender: NSTextField) {
        let value = min(max(sender.integerValue, 4), 30)
        Settings.shared.maxTraversalDepth = value
        sender.integerValue = value
        // Sync stepper
        if let contentView = window?.contentView {
            for subview in contentView.subviews {
                if let stepper = subview as? NSStepper {
                    stepper.integerValue = value
                }
            }
        }
    }

    @objc private func resetDefaults(_ sender: Any) {
        Settings.shared.maxTraversalDepth = Constants.maxTraversalDepth
        if let contentView = window?.contentView {
            for subview in contentView.subviews {
                if let field = subview as? NSTextField, field.isEditable {
                    field.integerValue = Constants.maxTraversalDepth
                }
                if let stepper = subview as? NSStepper {
                    stepper.integerValue = Constants.maxTraversalDepth
                }
            }
        }
    }

    // MARK: - Helpers

    private func onlyIntegerFormatter(min: Int, max: Int) -> NumberFormatter {
        let f = NumberFormatter()
        f.numberStyle = .none
        f.minimum = NSNumber(value: min)
        f.maximum = NSNumber(value: max)
        f.allowsFloats = false
        return f
    }
}
