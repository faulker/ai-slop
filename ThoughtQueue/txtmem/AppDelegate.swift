import Cocoa
import os

private let log = Logger(subsystem: "com.txtmem.app", category: "AppDelegate")

class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem?
    private let popoverController = PopoverController()
    private var hotkeyManager: HotkeyManager?

    func applicationDidFinishLaunching(_ notification: Notification) {
        log.info("ThoughtQueue launching...")

        NSSetUncaughtExceptionHandler { exception in
            let info = [
                "name": exception.name.rawValue,
                "reason": exception.reason ?? "unknown",
                "stack": exception.callStackSymbols.joined(separator: "\n")
            ]
            NSLog("CRASH: %@", info.description)
        }

        setupMainMenu()

        DatabaseManager.shared.initialize()
        NSLog("[ThoughtQueue] Database initialized")

        setupStatusItem()
        NSLog("[ThoughtQueue] Status item created")

        let trusted = AXIsProcessTrusted()
        NSLog("[ThoughtQueue] Accessibility trusted: %@", trusted ? "YES" : "NO")

        if !trusted {
            NSLog("[ThoughtQueue] Requesting accessibility permission...")
            let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue(): true] as CFDictionary
            AXIsProcessTrustedWithOptions(options)
        }

        let hk = HotkeyManager(onQuickCapture: handleQuickCapture, onDetailedCapture: handleDetailedCapture)
        hk.register()
        hotkeyManager = hk
        NSLog("[ThoughtQueue] Hotkey manager registered, app ready")
    }

    private func setupMainMenu() {
        let mainMenu = NSMenu()

        // App menu
        let appMenuItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(NSMenuItem(title: "About ThoughtQueue", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: ""))
        appMenu.addItem(NSMenuItem.separator())
        let prefsItem = NSMenuItem(title: "Preferences\u{2026}", action: #selector(openPreferences), keyEquivalent: ",")
        prefsItem.target = self
        appMenu.addItem(prefsItem)
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(NSMenuItem(title: "Quit ThoughtQueue", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
        appMenuItem.submenu = appMenu
        mainMenu.addItem(appMenuItem)

        // Edit menu (enables standard text editing shortcuts in text fields)
        let editMenuItem = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(NSMenuItem(title: "Undo", action: Selector(("undo:")), keyEquivalent: "z"))
        editMenu.addItem(NSMenuItem(title: "Redo", action: Selector(("redo:")), keyEquivalent: "Z"))
        editMenu.addItem(NSMenuItem.separator())
        editMenu.addItem(NSMenuItem(title: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x"))
        editMenu.addItem(NSMenuItem(title: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c"))
        editMenu.addItem(NSMenuItem(title: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v"))
        editMenu.addItem(NSMenuItem(title: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a"))
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)

        NSApp.mainMenu = mainMenu
    }

    private func setupStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem = item

        if let button = item.button {
            button.image = makeMenuBarIcon()
            button.action = #selector(handleLeftClick)
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
            button.target = self
        }
    }

    private func makeMenuBarIcon() -> NSImage {
        let size: CGFloat = 18
        let image = NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            let s = rect.size.width

            let inset = s * 0.08
            let outlineRect = NSRect(x: inset, y: inset, width: s - inset * 2, height: s - inset * 2)
            let outlineRadius = s * 0.18
            let outlinePath = NSBezierPath(roundedRect: outlineRect, xRadius: outlineRadius, yRadius: outlineRadius)
            NSColor.controlTextColor.setStroke()
            outlinePath.lineWidth = 1.2
            outlinePath.stroke()

            let font = NSFont(name: "Georgia-Bold", size: s * 0.45) ?? NSFont.systemFont(ofSize: s * 0.45, weight: .heavy)
            let attrs: [NSAttributedString.Key: Any] = [
                .font: font,
                .foregroundColor: NSColor.controlTextColor,
            ]
            let quoteStr = NSAttributedString(string: "\u{201C}\u{201D}", attributes: attrs)
            let strSize = quoteStr.size()
            let x = (s - strSize.width) / 2
            let y = (s - strSize.height) / 2
            quoteStr.draw(at: NSPoint(x: x, y: y))

            return true
        }
        image.isTemplate = true
        return image
    }

    @objc private func handleLeftClick(_ sender: NSStatusBarButton) {
        guard let event = NSApp.currentEvent else { return }

        if event.type == .rightMouseUp {
            showContextMenu()
        } else {
            popoverController.toggle(relativeTo: sender.bounds, of: sender)
        }
    }

    private func showContextMenu() {
        guard let statusItem = statusItem else { return }
        let menu = NSMenu()
        menu.addItem(NSMenuItem(title: "Open ThoughtQueue", action: #selector(openMainWindow), keyEquivalent: ""))
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Preferences\u{2026}", action: #selector(openPreferences), keyEquivalent: ","))
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Quit", action: #selector(NSApp.terminate), keyEquivalent: "q"))
        statusItem.menu = menu
        statusItem.button?.performClick(nil)
        statusItem.menu = nil
    }

    @objc private func openMainWindow() {
        MainWindowController.shared.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    @objc private func openPreferences() {
        PreferencesWindowController.shared.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func handleQuickCapture() {
        CaptureService.shared.quickCapture()
    }

    private func handleDetailedCapture() {
        CaptureService.shared.showDetailedCapture()
    }
}
