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

            // Rounded square outline
            let inset = s * 0.08
            let outlineRect = NSRect(x: inset, y: inset, width: s - inset * 2, height: s - inset * 2)
            let outlineRadius = s * 0.18
            let outlinePath = NSBezierPath(roundedRect: outlineRect, xRadius: outlineRadius, yRadius: outlineRadius)
            NSColor.controlTextColor.setStroke()
            outlinePath.lineWidth = 1.2
            outlinePath.stroke()

            // Curly quotes in center
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
        menu.addItem(NSMenuItem(title: "Preferences…", action: #selector(openPreferences), keyEquivalent: ","))
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
