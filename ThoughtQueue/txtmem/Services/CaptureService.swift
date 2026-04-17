import Cocoa
import os

private let log = Logger(subsystem: "com.txtmem.app", category: "CaptureService")

final class CaptureService {
    static let shared = CaptureService()

    private init() {}

    func quickCapture() {
        log.info("Quick capture triggered")
        grabSelectedText { [weak self] text, savedClipboard in
            guard let text = text, !text.isEmpty else {
                log.warning("No text captured from selection")
                restoreClipboard(savedClipboard)
                self?.showToast("No text selected")
                return
            }

            log.info("Captured text: \(text.prefix(50))...")
            if let _ = DatabaseManager.shared.createEntry(text: text) {
                restoreClipboard(savedClipboard)
                self?.showToast("Captured!")
                NotificationCenter.default.post(name: .entriesDidChange, object: nil)
            } else {
                log.error("Failed to save entry to database")
                restoreClipboard(savedClipboard)
                self?.showToast("Failed to save")
            }
        }
    }

    func showDetailedCapture() {
        log.info("Detailed capture triggered")
        grabSelectedText { [weak self] text, savedClipboard in
            guard let text = text, !text.isEmpty else {
                log.warning("No text captured for detailed capture")
                restoreClipboard(savedClipboard)
                self?.showToast("No text selected")
                return
            }

            restoreClipboard(savedClipboard)
            DetailedCapturePanel.shared.show(with: text)
        }
    }

    private func grabSelectedText(completion: @escaping (String?, [[NSPasteboard.PasteboardType: Data]]) -> Void) {
        let pasteboard = NSPasteboard.general
        let savedClipboard = saveClipboard()
        let changeCountBefore = pasteboard.changeCount

        log.debug("Pasteboard change count before: \(changeCountBefore)")

        let source = CGEventSource(stateID: .combinedSessionState)
        guard let keyDown = CGEvent(keyboardEventSource: source, virtualKey: 0x08, keyDown: true),
              let keyUp = CGEvent(keyboardEventSource: source, virtualKey: 0x08, keyDown: false) else {
            log.error("Failed to create CGEvent for Cmd+C simulation")
            completion(nil, savedClipboard)
            return
        }

        keyDown.flags = .maskCommand
        keyUp.flags = .maskCommand
        keyDown.post(tap: .cghidEventTap)
        keyUp.post(tap: .cghidEventTap)

        log.debug("Simulated Cmd+C, waiting for pasteboard update...")

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
                completion(text, savedClipboard)
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

// MARK: - Clipboard Save/Restore

func saveClipboard() -> [[NSPasteboard.PasteboardType: Data]] {
    let pb = NSPasteboard.general
    guard let items = pb.pasteboardItems else { return [] }
    return items.map { item in
        var dict: [NSPasteboard.PasteboardType: Data] = [:]
        for type in item.types {
            if let data = item.data(forType: type) {
                dict[type] = data
            }
        }
        return dict
    }
}

func restoreClipboard(_ saved: [[NSPasteboard.PasteboardType: Data]]) {
    let pb = NSPasteboard.general
    pb.clearContents()
    guard !saved.isEmpty else { return }
    let items = saved.map { dict -> NSPasteboardItem in
        let item = NSPasteboardItem()
        for (type, data) in dict {
            item.setData(data, forType: type)
        }
        return item
    }
    pb.writeObjects(items)
}

extension Notification.Name {
    static let entriesDidChange = Notification.Name("entriesDidChange")
}
