import AppKit

protocol StatusBarControllerDelegate: AnyObject {
    func statusBarDidToggleEnabled(_ enabled: Bool)
    func statusBarDidRequestQuit()
}

/// Manages the menu bar status item with enable/disable toggle and quit.
final class StatusBarController {

    weak var delegate: StatusBarControllerDelegate?

    private let statusItem: NSStatusItem
    private var isEnabled = true

    init() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)

        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "textformat.abc", accessibilityDescription: Constants.appName)
        }

        setupMenu()
    }

    private func setupMenu() {
        let menu = NSMenu()

        let toggleItem = NSMenuItem(title: "Enabled", action: #selector(toggleEnabled(_:)), keyEquivalent: "")
        toggleItem.target = self
        toggleItem.state = isEnabled ? .on : .off
        menu.addItem(toggleItem)

        menu.addItem(.separator())

        let quitItem = NSMenuItem(title: "Quit \(Constants.appName)", action: #selector(quit(_:)), keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(quitItem)

        statusItem.menu = menu
    }

    @objc private func toggleEnabled(_ sender: NSMenuItem) {
        isEnabled.toggle()
        sender.state = isEnabled ? .on : .off
        delegate?.statusBarDidToggleEnabled(isEnabled)
    }

    @objc private func quit(_ sender: Any) {
        delegate?.statusBarDidRequestQuit()
    }
}
