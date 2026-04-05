import Cocoa

final class EntryRowView: NSView {
    private let entry: Entry

    init(entry: Entry, compact: Bool = false) {
        self.entry = entry
        super.init(frame: .zero)
        setupUI(compact: compact)
    }

    required init?(coder: NSCoder) { fatalError() }

    private func setupUI(compact: Bool) {
        let sentButton = NSButton(title: "", target: self, action: #selector(toggleSent))
        sentButton.image = NSImage(systemSymbolName: entry.isSent ? "checkmark.circle.fill" : "circle", accessibilityDescription: entry.isSent ? "Mark as unread" : "Mark as read")
        sentButton.imagePosition = .imageOnly
        sentButton.bezelStyle = .inline
        sentButton.isBordered = false
        sentButton.toolTip = entry.isSent ? "Mark as unread" : "Mark as read"
        sentButton.contentTintColor = entry.isSent ? .systemGreen : .secondaryLabelColor
        let sentIndicator = sentButton

        let truncated = entry.text.prefix(80)
        let preview = String(truncated) + (entry.text.count > 80 ? "\u{2026}" : "")
        let textLabel = NSTextField(labelWithString: preview)
        textLabel.font = .systemFont(ofSize: 12)
        textLabel.lineBreakMode = .byTruncatingTail
        textLabel.maximumNumberOfLines = compact ? 1 : 2
        textLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let openBtn = makeButton(title: "Open", action: #selector(openInClaude))
        let moveBtn = makeButton(title: "Move", action: #selector(showMoveMenu(_:)))
        let deleteBtn = makeButton(title: "Del", action: #selector(confirmDelete))

        let buttonStack = NSStackView(views: [openBtn, moveBtn, deleteBtn])
        buttonStack.spacing = 4

        let mainStack = NSStackView(views: [sentIndicator, textLabel, buttonStack])
        mainStack.spacing = 6
        mainStack.alignment = .centerY
        mainStack.translatesAutoresizingMaskIntoConstraints = false

        addSubview(mainStack)
        NSLayoutConstraint.activate([
            mainStack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            mainStack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            mainStack.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            mainStack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 30),
            sentIndicator.widthAnchor.constraint(equalToConstant: 16),
        ])
    }

    private func makeButton(title: String, action: Selector) -> NSButton {
        let btn = NSButton(title: title, target: self, action: action)
        btn.bezelStyle = .inline
        btn.font = .systemFont(ofSize: 11)
        btn.controlSize = .small
        return btn
    }

    @objc private func toggleSent() {
        _ = DatabaseManager.shared.toggleEntrySent(id: entry.id, isSent: !entry.isSent)
        NotificationCenter.default.post(name: .entriesDidChange, object: nil)
    }

    @objc private func openInClaude() {
        ClaudeIntegration.shared.sendToClaude(entry: entry)
    }

    @objc private func showMoveMenu(_ sender: NSButton) {
        let menu = NSMenu()

        let uncatItem = NSMenuItem(title: "Uncategorized", action: #selector(moveToCategory(_:)), keyEquivalent: "")
        uncatItem.target = self
        uncatItem.representedObject = nil as Int64? as AnyObject
        menu.addItem(uncatItem)
        menu.addItem(NSMenuItem.separator())

        for cat in DatabaseManager.shared.fetchCategories() {
            let item = NSMenuItem(title: cat.name, action: #selector(moveToCategory(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = cat.id as AnyObject
            menu.addItem(item)
        }

        menu.popUp(positioning: nil, at: sender.bounds.origin, in: sender)
    }

    @objc private func moveToCategory(_ sender: NSMenuItem) {
        let categoryId = sender.representedObject as? Int64
        _ = DatabaseManager.shared.moveEntry(id: entry.id, toCategoryId: categoryId)
        NotificationCenter.default.post(name: .entriesDidChange, object: nil)
    }

    @objc private func confirmDelete() {
        let alert = NSAlert()
        alert.messageText = "Delete Entry?"
        let preview = String(entry.text.prefix(60)) + (entry.text.count > 60 ? "\u{2026}" : "")
        alert.informativeText = "This will permanently delete: \"\(preview)\""
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Delete")
        alert.addButton(withTitle: "Cancel")

        if alert.runModal() == .alertFirstButtonReturn {
            _ = DatabaseManager.shared.deleteEntry(id: entry.id)
            NotificationCenter.default.post(name: .entriesDidChange, object: nil)
        }
    }
}
