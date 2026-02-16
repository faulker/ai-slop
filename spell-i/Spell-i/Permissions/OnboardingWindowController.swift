import AppKit

/// First-launch onboarding window that guides the user through granting
/// Accessibility permission and explains how Spell-i works.
final class OnboardingWindowController: NSWindowController, NSWindowDelegate {

    /// Called when onboarding is complete (permission granted).
    var onComplete: (() -> Void)?

    private var pollTimer: Timer?
    private var permissionWasGranted = false

    convenience init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 250),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "Welcome to Spell-i"
        window.isReleasedWhenClosed = false
        window.level = .floating
        window.center()

        self.init(window: window)
        window.delegate = self
        setupUI()
        setupNotifications()
    }

    private func setupNotifications() {
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(checkPermission),
            name: NSApplication.didBecomeActiveNotification,
            object: nil
        )
    }

    private func setupUI() {
        guard let contentView = window?.contentView else { return }

        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 32),
            stack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -32),
            stack.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
        ])

        let titleLabel = NSTextField(wrappingLabelWithString: "Spell-i needs Accessibility access")
        titleLabel.font = .systemFont(ofSize: 18, weight: .bold)
        titleLabel.alignment = .center
        stack.addArrangedSubview(titleLabel)

        let descLabel = NSTextField(wrappingLabelWithString:
            "Spell-i reads text from other applications to provide real-time spell checking. " +
            "This requires Accessibility permission in System Settings.")
        descLabel.alignment = .center
        descLabel.preferredMaxLayoutWidth = 336
        stack.addArrangedSubview(descLabel)

        let privacyLabel = NSTextField(wrappingLabelWithString: "No data leaves your Mac — everything is checked offline.")
        privacyLabel.font = .systemFont(ofSize: 12, weight: .medium)
        privacyLabel.textColor = .secondaryLabelColor
        privacyLabel.alignment = .center
        stack.addArrangedSubview(privacyLabel)

        let button = NSButton(title: "Open System Settings", target: self, action: #selector(openSettings(_:)))
        button.bezelStyle = .rounded
        button.controlSize = .large
        button.contentTintColor = NSColor(red: 0.15, green: 0.45, blue: 0.20, alpha: 1.0) // Darker green for contrast
        stack.addArrangedSubview(button)

        let retryButton = NSButton(title: "I've Granted Access", target: self, action: #selector(retryPermissionCheck(_:)))
        retryButton.bezelStyle = .rounded
        retryButton.controlSize = .regular
        stack.addArrangedSubview(retryButton)
    }

    // MARK: - Public

    func showOnboarding() {
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        startPolling()
    }

    // MARK: - Actions

    @objc private func openSettings(_ sender: Any) {
        AccessibilityPermissionChecker.openAccessibilitySettings()
    }

    @objc private func retryPermissionCheck(_ sender: Any) {
        if AccessibilityPermissionChecker.isAccessibilityEnabled() {
            permissionGranted()
        } else {
            // Show inline feedback that permission still isn't detected
            let alert = NSAlert()
            alert.messageText = "Permission Not Detected"
            alert.informativeText = "Spell-i still can't detect Accessibility access. Try removing and re-adding Spell-i in System Settings > Privacy & Security > Accessibility, then click this button again."
            alert.alertStyle = .warning
            alert.addButton(withTitle: "OK")
            alert.runModal()
        }
    }

    @objc private func checkPermission() {
        if AccessibilityPermissionChecker.isAccessibilityEnabled() {
            permissionGranted()
        }
    }

    // MARK: - Polling

    private func startPolling() {
        pollTimer?.invalidate()
        pollTimer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak self] _ in
            self?.checkPermission()
        }
    }

    private func permissionGranted() {
        pollTimer?.invalidate()
        pollTimer = nil
        permissionWasGranted = true
        close()
        onComplete?()
    }

    // MARK: - NSWindowDelegate

    func windowWillClose(_ notification: Notification) {
        guard !permissionWasGranted else { return }
        pollTimer?.invalidate()
        pollTimer = nil
        NSApp.terminate(nil)
    }

    deinit {
        pollTimer?.invalidate()
    }
}
