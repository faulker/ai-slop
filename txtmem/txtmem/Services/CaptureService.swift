import Cocoa
import os

private let log = Logger(subsystem: "com.txtmem.app", category: "CaptureService")

final class CaptureService {
    static let shared = CaptureService()

    private init() {}

    func quickCapture() {
        log.info("Quick capture triggered")
        grabSelectedText { [weak self] text in
            guard let text = text, !text.isEmpty else {
                log.warning("No text captured from selection")
                self?.showToast("No text selected")
                return
            }

            log.info("Captured text: \(text.prefix(50))...")
            if let _ = DatabaseManager.shared.createEntry(text: text) {
                NSPasteboard.general.clearContents()
                self?.showToast("Captured!")
                NotificationCenter.default.post(name: .entriesDidChange, object: nil)
            } else {
                log.error("Failed to save entry to database")
                self?.showToast("Failed to save")
            }
        }
    }

    func showDetailedCapture() {
        log.info("Detailed capture triggered")
        grabSelectedText { [weak self] text in
            guard let text = text, !text.isEmpty else {
                log.warning("No text captured for detailed capture")
                self?.showToast("No text selected")
                return
            }

            // Don't clear clipboard here — DetailedCapturePanel.save() will clear it on successful save
            DetailedCapturePanel.shared.show(with: text)
        }
    }

    private func grabSelectedText(completion: @escaping (String?) -> Void) {
        let pasteboard = NSPasteboard.general
        let changeCountBefore = pasteboard.changeCount

        log.debug("Pasteboard change count before: \(changeCountBefore)")

        let source = CGEventSource(stateID: .combinedSessionState)
        guard let keyDown = CGEvent(keyboardEventSource: source, virtualKey: 0x08, keyDown: true),
              let keyUp = CGEvent(keyboardEventSource: source, virtualKey: 0x08, keyDown: false) else {
            log.error("Failed to create CGEvent for Cmd+C simulation")
            completion(nil)
            return
        }

        keyDown.flags = .maskCommand
        keyUp.flags = .maskCommand
        keyDown.post(tap: .cghidEventTap)
        keyUp.post(tap: .cghidEventTap)

        log.debug("Simulated Cmd+C, waiting for pasteboard update...")

        // Check pasteboard on a background queue to avoid blocking main thread
        DispatchQueue.global(qos: .userInitiated).async {
            var text: String?
            let maxAttempts = 10
            for attempt in 1...maxAttempts {
                Thread.sleep(forTimeInterval: 0.05)
                if pasteboard.changeCount != changeCountBefore {
                    text = pasteboard.string(forType: .string)
                    log.debug("Pasteboard updated on attempt \(attempt)")
                    break
                }
            }

            if text == nil {
                log.debug("Pasteboard did not update after \(maxAttempts) attempts")
            }

            DispatchQueue.main.async {
                completion(text)
            }
        }
    }

    private func showToast(_ message: String) {
        if Thread.isMainThread {
            ToastWindow.show(message: message)
        } else {
            DispatchQueue.main.async {
                ToastWindow.show(message: message)
            }
        }
    }
}

extension Notification.Name {
    static let entriesDidChange = Notification.Name("entriesDidChange")
}
