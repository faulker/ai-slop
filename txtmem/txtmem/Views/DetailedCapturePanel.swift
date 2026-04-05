import Cocoa

final class DetailedCapturePanel {
    static let shared = DetailedCapturePanel()

    private var panel: NSPanel?
    private var textView: NSTextView?
    private var categoryPopup: NSPopUpButton?

    private init() {}

    func show(with text: String) {
        panel?.close()

        let panelWidth: CGFloat = 400
        let panelHeight: CGFloat = 260

        let mouseLocation = NSEvent.mouseLocation
        let origin = NSPoint(x: mouseLocation.x - panelWidth / 2, y: mouseLocation.y - panelHeight - 20)

        let newPanel = NSPanel(
            contentRect: NSRect(origin: origin, size: NSSize(width: panelWidth, height: panelHeight)),
            styleMask: [.titled, .closable, .nonactivatingPanel, .hudWindow],
            backing: .buffered,
            defer: false
        )
        newPanel.level = .floating
        newPanel.title = "Capture to ThoughtQueue"
        newPanel.isFloatingPanel = true
        newPanel.isReleasedWhenClosed = false

        let contentView = NSView(frame: NSRect(x: 0, y: 0, width: panelWidth, height: panelHeight))

        let scrollView = NSScrollView(frame: NSRect(x: 16, y: 60, width: panelWidth - 32, height: panelHeight - 100))
        let tv = NSTextView(frame: scrollView.bounds)
        tv.string = text
        tv.font = .systemFont(ofSize: 13)
        tv.isEditable = true
        tv.isRichText = false
        tv.autoresizingMask = [.width, .height]
        scrollView.documentView = tv
        scrollView.hasVerticalScroller = true
        contentView.addSubview(scrollView)
        self.textView = tv

        let popup = NSPopUpButton(frame: NSRect(x: 16, y: 20, width: 180, height: 28))
        popup.addItem(withTitle: "Uncategorized")
        popup.menu?.items.first?.tag = 0
        for cat in DatabaseManager.shared.fetchCategories() {
            popup.addItem(withTitle: cat.name)
            popup.menu?.items.last?.tag = Int(cat.id)
        }
        contentView.addSubview(popup)
        self.categoryPopup = popup

        let saveBtn = NSButton(title: "Save", target: self, action: #selector(save))
        saveBtn.bezelStyle = .rounded
        saveBtn.frame = NSRect(x: panelWidth - 90, y: 20, width: 70, height: 28)
        saveBtn.keyEquivalent = "\r"
        contentView.addSubview(saveBtn)

        let cancelBtn = NSButton(title: "Cancel", target: self, action: #selector(cancel))
        cancelBtn.bezelStyle = .rounded
        cancelBtn.frame = NSRect(x: panelWidth - 170, y: 20, width: 70, height: 28)
        cancelBtn.keyEquivalent = "\u{1b}"
        contentView.addSubview(cancelBtn)

        newPanel.contentView = contentView
        newPanel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        panel = newPanel
    }

    @objc private func save() {
        guard let tv = textView, let popup = categoryPopup else {
            NSLog("[ThoughtQueue] DetailedCapture save failed — missing references")
            return
        }

        let text = tv.string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }

        let selectedTag = popup.selectedItem?.tag ?? 0
        let categoryId: Int64? = selectedTag == 0 ? nil : Int64(selectedTag)

        if let _ = DatabaseManager.shared.createEntry(text: text, categoryId: categoryId) {
            NSPasteboard.general.clearContents()
            NotificationCenter.default.post(name: .entriesDidChange, object: nil)
            ToastWindow.show(message: "Captured!")
        }

        panel?.close()
        panel = nil
        textView = nil
        categoryPopup = nil
    }

    @objc private func cancel() {
        panel?.close()
        panel = nil
        textView = nil
        categoryPopup = nil
    }
}
