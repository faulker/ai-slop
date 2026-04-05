import Cocoa
import os

private let log = Logger(subsystem: "com.txtmem.app", category: "ClaudeIntegration")

final class ClaudeIntegration {
    static let shared = ClaudeIntegration()

    private let claudeBundleId = "com.anthropic.claudefordesktop"

    private init() {}

    func sendToClaude(entry: Entry) {
        log.info("Sending entry \(entry.id) to Claude")
        NSLog("[ThoughtQueue] Sending entry %lld to Claude", entry.id)

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(entry.text, forType: .string)

        let workspace = NSWorkspace.shared
        guard let claudeURL = workspace.urlForApplication(withBundleIdentifier: claudeBundleId) else {
            log.error("Claude desktop app not found")
            showError("Claude desktop app not found. Please install it first.")
            return
        }

        let config = NSWorkspace.OpenConfiguration()
        config.activates = true

        workspace.openApplication(at: claudeURL, configuration: config) { [weak self] _, error in
            if let error = error {
                DispatchQueue.main.async {
                    NSLog("[ThoughtQueue] Failed to open Claude: %@", error.localizedDescription)
                    self?.showError("Failed to open Claude: \(error.localizedDescription)")
                }
                return
            }

            NSLog("[ThoughtQueue] Claude activated, waiting before new chat...")

            // Longer delay for Electron app to come to foreground
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                NSLog("[ThoughtQueue] Simulating Cmd+1 to switch to Chat tab")
                self?.simulateKeystroke(keyCode: 0x12, flags: [.maskCommand])  // Cmd+1

                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    NSLog("[ThoughtQueue] Simulating Cmd+Shift+O for new chat")
                    self?.simulateKeystroke(keyCode: 0x1F, flags: [.maskCommand, .maskShift])  // Cmd+Shift+O

                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                        NSLog("[ThoughtQueue] Simulating Cmd+V to paste")
                        self?.simulateKeystroke(keyCode: 0x09, flags: [.maskCommand])  // Cmd+V

                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                            NSPasteboard.general.clearContents()
                            _ = DatabaseManager.shared.markEntryAsSent(id: entry.id)
                            NotificationCenter.default.post(name: .entriesDidChange, object: nil)
                            NSLog("[ThoughtQueue] Entry %lld sent to Claude successfully", entry.id)
                        }
                    }
                }
            }
        }
    }

    private func simulateKeystroke(keyCode: UInt16, flags: CGEventFlags) {
        // Use a separate event source so our own event tap doesn't intercept these
        let source = CGEventSource(stateID: .combinedSessionState)

        guard let keyDown = CGEvent(keyboardEventSource: source, virtualKey: keyCode, keyDown: true),
              let keyUp = CGEvent(keyboardEventSource: source, virtualKey: keyCode, keyDown: false) else {
            NSLog("[ThoughtQueue] Failed to create CGEvent for keyCode %d", keyCode)
            return
        }

        keyDown.flags = flags
        keyUp.flags = flags

        // Post to cgAnnotatedSessionEventTap to bypass our own event tap
        keyDown.post(tap: .cgAnnotatedSessionEventTap)
        keyUp.post(tap: .cgAnnotatedSessionEventTap)
    }

    private func showError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Claude Integration Error"
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.runModal()
    }
}
