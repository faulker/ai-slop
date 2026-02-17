import AppKit

protocol StatusBarControllerDelegate: AnyObject {
    func statusBarDidToggleEnabled(_ enabled: Bool)
    func statusBarDidRequestSettings()
    func statusBarDidRequestDumpAXTree()
    func statusBarDidRequestQuit()
}

/// Manages the menu bar status item with enable/disable toggle and quit.
final class StatusBarController {

    weak var delegate: StatusBarControllerDelegate?

    private let statusItem: NSStatusItem
    private var isEnabled = true
    private var statusMenuItem: NSMenuItem?

    init() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)

        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "textformat.abc", accessibilityDescription: Constants.appName)
        }

        setupMenu()
    }

    private func setupMenu() {
        let menu = NSMenu()

        let statusLine = NSMenuItem(title: "Engine: Starting...", action: nil, keyEquivalent: "")
        statusLine.isEnabled = false
        menu.addItem(statusLine)
        self.statusMenuItem = statusLine

        menu.addItem(.separator())

        let toggleItem = NSMenuItem(title: "Enabled", action: #selector(toggleEnabled(_:)), keyEquivalent: "")
        toggleItem.target = self
        toggleItem.state = isEnabled ? .on : .off
        menu.addItem(toggleItem)

        let settingsItem = NSMenuItem(title: "Settings...", action: #selector(openSettings(_:)), keyEquivalent: ",")
        settingsItem.target = self
        menu.addItem(settingsItem)

        let dumpItem = NSMenuItem(title: "Dump AX Tree", action: #selector(dumpAXTree(_:)), keyEquivalent: "")
        dumpItem.target = self
        menu.addItem(dumpItem)

        menu.addItem(.separator())

        let quitItem = NSMenuItem(title: "Quit \(Constants.appName)", action: #selector(quit(_:)), keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(quitItem)

        statusItem.menu = menu
    }

    /// Updates the status bar icon and menu status text based on engine state.
    /// Must be called on the main thread (AppKit requirement).
    func updateState(_ state: TextMonitorCoordinator.EngineState) {
        dispatchPrecondition(condition: .onQueue(.main))
        let iconName: String
        let statusText: String

        switch state {
        case .initializing:
            iconName = "textformat.abc"
            statusText = "Engine: Starting..."
        case .ready:
            iconName = "textformat.abc"
            statusText = "Engine: Ready"
        case .degraded(let retryCount):
            iconName = "exclamationmark.triangle"
            statusText = "Engine: Degraded — retrying (\(retryCount + 1)/\(Constants.engineRetryDelays.count))..."
        case .failed:
            iconName = "xmark.circle"
            statusText = "Engine: Failed — restart app"
        }

        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: iconName, accessibilityDescription: Constants.appName)
        }
        statusMenuItem?.title = statusText
    }

    @objc private func toggleEnabled(_ sender: NSMenuItem) {
        isEnabled.toggle()
        sender.state = isEnabled ? .on : .off
        delegate?.statusBarDidToggleEnabled(isEnabled)
    }

    @objc private func openSettings(_ sender: Any) {
        delegate?.statusBarDidRequestSettings()
    }

    @objc private func dumpAXTree(_ sender: Any) {
        delegate?.statusBarDidRequestDumpAXTree()
    }

    @objc private func quit(_ sender: Any) {
        delegate?.statusBarDidRequestQuit()
    }

    #if DEBUG
    var statusMenuItemTitleForTesting: String? {
        statusMenuItem?.title
    }

    var menuForTesting: NSMenu? {
        statusItem.menu
    }
    #endif
}
