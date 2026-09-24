import AppKit
import BlockerCore
import Combine
import SwiftUI

/// Owns the blocker, the optional menu bar icon, and the settings window.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let settings = BlockerSettings()
    private var blocker: Blocker?
    private var statusItem: NSStatusItem?
    private var settingsWindow: NSWindow?
    private var cancellables: Set<AnyCancellable> = []

    /// Starts blocking, sets up the icon, and shows settings on first run.
    func applicationDidFinishLaunching(_ notification: Notification) {
        let settings = settings
        let blocker = Blocker(
            isEnabled: { settings.blockingEnabled },
            onBlocked: { settings.blockedCount += 1 }
        )
        blocker.start()
        self.blocker = blocker

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = Self.menuBarImage()
        let menu = NSMenu()
        menu.delegate = self
        item.menu = menu
        statusItem = item

        settings.$showMenuBarIcon
            .sink { [weak self] visible in self?.statusItem?.isVisible = visible }
            .store(in: &cancellables)
        // Kill any Music that was allowed to start while blocking was off.
        settings.$blockingEnabled
            .dropFirst()
            .filter { $0 }
            .sink { [weak self] _ in self?.blocker?.sweep() }
            .store(in: &cancellables)

        if settings.isFirstLaunch {
            showSettings()
        }
    }

    /// Opening the app again while it runs brings up settings, the way back from hidden mode.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showSettings()
        return false
    }

    /// Rebuilds the menu each time it opens so the checkmarks are current.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()

        let block = NSMenuItem(title: "Block Apple Music", action: #selector(toggleBlocking), keyEquivalent: "")
        block.state = settings.blockingEnabled ? .on : .off
        menu.addItem(block)

        let login = NSMenuItem(title: "Launch at Login", action: #selector(toggleLoginItem), keyEquivalent: "")
        login.state = LoginItem.isEnabled ? .on : .off
        menu.addItem(login)

        menu.addItem(NSMenuItem(title: "Hide Menu Bar Icon", action: #selector(hideIcon), keyEquivalent: ""))
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Blocked \(settings.blockedCount) this session", action: nil, keyEquivalent: ""))
        menu.addItem(NSMenuItem(title: "Settings…", action: #selector(showSettings), keyEquivalent: ","))
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))

        for item in menu.items where item.action != nil && item.action != #selector(NSApplication.terminate(_:)) {
            item.target = self
        }
    }

    @objc private func toggleBlocking() {
        settings.blockingEnabled.toggle()
    }

    @objc private func toggleLoginItem() {
        do {
            try LoginItem.set(!LoginItem.isEnabled)
        } catch {
            let alert = NSAlert(error: error)
            alert.messageText = "Couldn't change the login item"
            NSApp.activate()
            alert.runModal()
        }
    }

    @objc private func hideIcon() {
        settings.showMenuBarIcon = false
    }

    /// Shows the settings window, creating it on first use.
    @objc private func showSettings() {
        if settingsWindow == nil {
            let window = NSWindow(contentViewController: NSHostingController(rootView: SettingsView(settings: settings)))
            window.title = "Disable iMusic"
            window.styleMask = [.titled, .closable]
            window.isReleasedWhenClosed = false
            window.center()
            settingsWindow = window
        }
        NSApp.activate()
        settingsWindow?.makeKeyAndOrderFront(nil)
    }

    /// Picks a "no music" symbol, falling back if this macOS lacks it.
    private static func menuBarImage() -> NSImage? {
        for name in ["music.note.slash", "speaker.slash", "nosign"] {
            if let image = NSImage(systemSymbolName: name, accessibilityDescription: "Disable iMusic") {
                image.isTemplate = true
                return image
            }
        }
        return nil
    }
}
