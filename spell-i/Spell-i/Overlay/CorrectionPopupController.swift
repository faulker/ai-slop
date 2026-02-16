import AppKit

/// Displays a context menu near a misspelled word with correction suggestions,
/// "Add to Dictionary", and "Ignore" actions.
final class CorrectionPopupController: NSObject {

    /// Called when the user selects a correction. Parameters: replacement string, result index.
    var onCorrectionSelected: ((String, Int) -> Void)?

    /// Called when the user taps "Add to Dictionary" with the original word.
    var onAddToDictionary: ((String) -> Void)?

    /// Called when the user taps "Ignore" with the original word.
    var onIgnore: ((String) -> Void)?

    /// The view used to anchor the popup menu (overlay content view).
    weak var anchorView: NSView?

    // MARK: - Public

    /// Shows the correction menu near the given rect (in overlay view coordinates).
    func show(near viewRect: CGRect, originalWord: String, suggestions: [String], resultIndex: Int) {
        let menu = NSMenu()
        menu.autoenablesItems = false

        if suggestions.isEmpty {
            let item = NSMenuItem(title: "No suggestions", action: nil, keyEquivalent: "")
            item.isEnabled = false
            menu.addItem(item)
        } else {
            for suggestion in suggestions {
                let item = NSMenuItem(title: suggestion, action: #selector(suggestionClicked(_:)), keyEquivalent: "")
                item.target = self
                item.representedObject = ["replacement": suggestion, "index": resultIndex] as [String: Any]
                menu.addItem(item)
            }
        }

        menu.addItem(.separator())

        let addItem = NSMenuItem(title: "Add to Dictionary", action: #selector(addToDictionaryClicked(_:)), keyEquivalent: "")
        addItem.target = self
        addItem.representedObject = originalWord
        menu.addItem(addItem)

        let ignoreItem = NSMenuItem(title: "Ignore", action: #selector(ignoreClicked(_:)), keyEquivalent: "")
        ignoreItem.target = self
        ignoreItem.representedObject = originalWord
        menu.addItem(ignoreItem)

        guard let view = anchorView else { return }

        // Position at the bottom-left of the word rect (view is flipped, so maxY = below word)
        let menuPoint = NSPoint(x: viewRect.minX, y: viewRect.maxY + 4)
        menu.popUp(positioning: nil, at: menuPoint, in: view)
    }

    func dismiss() {
        // NSMenu handles its own dismissal
    }

    // MARK: - Actions

    @objc private func suggestionClicked(_ sender: NSMenuItem) {
        guard let info = sender.representedObject as? [String: Any],
              let replacement = info["replacement"] as? String,
              let resultIndex = info["index"] as? Int else { return }
        onCorrectionSelected?(replacement, resultIndex)
    }

    @objc private func addToDictionaryClicked(_ sender: NSMenuItem) {
        guard let word = sender.representedObject as? String else { return }
        onAddToDictionary?(word)
    }

    @objc private func ignoreClicked(_ sender: NSMenuItem) {
        guard let word = sender.representedObject as? String else { return }
        onIgnore?(word)
    }
}
