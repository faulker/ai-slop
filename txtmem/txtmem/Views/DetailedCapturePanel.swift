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

        // Text scroll view
        let scrollView = NSScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .bezelBorder

        let tv = NSTextView()
        tv.string = text
        tv.font = .systemFont(ofSize: 13)
        tv.isEditable = true
        tv.isRichText = false
        tv.isVerticallyResizable = true
        tv.isHorizontallyResizable = false
        tv.textContainerInset = NSSize(width: 4, height: 4)
        tv.textContainer?.widthTracksTextView = true
        tv.autoresizingMask = [.width, .height]
        scrollView.documentView = tv
        self.textView = tv

        // Category popup
        let popup = NSPopUpButton()
        popup.translatesAutoresizingMaskIntoConstraints = false
        popup.addItem(withTitle: "Uncategorized")
        popup.menu?.items.first?.tag = 0
        for cat in DatabaseManager.shared.fetchCategories() {
            popup.addItem(withTitle: cat.name)
            popup.menu?.items.last?.tag = Int(cat.id)
        }
        self.categoryPopup = popup

        // Buttons
        let saveBtn = NSButton(title: "Save", target: self, action: #selector(save))
        saveBtn.translatesAutoresizingMaskIntoConstraints = false
        saveBtn.bezelStyle = .rounded
        saveBtn.keyEquivalent = "\r"

        let cancelBtn = NSButton(title: "Cancel", target: self, action: #selector(cancel))
        cancelBtn.translatesAutoresizingMaskIntoConstraints = false
        cancelBtn.bezelStyle = .rounded
        cancelBtn.keyEquivalent = "\u{1b}"

        contentView.addSubview(scrollView)
        contentView.addSubview(popup)
        contentView.addSubview(saveBtn)
        contentView.addSubview(cancelBtn)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 16),
            scrollView.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            scrollView.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            scrollView.bottomAnchor.constraint(equalTo: popup.topAnchor, constant: -12),

            popup.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            popup.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -16),
            popup.widthAnchor.constraint(greaterThanOrEqualToConstant: 150),

            saveBtn.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            saveBtn.centerYAnchor.constraint(equalTo: popup.centerYAnchor),
            saveBtn.widthAnchor.constraint(equalToConstant: 70),

            cancelBtn.trailingAnchor.constraint(equalTo: saveBtn.leadingAnchor, constant: -8),
            cancelBtn.centerYAnchor.constraint(equalTo: popup.centerYAnchor),
            cancelBtn.widthAnchor.constraint(equalToConstant: 70),
        ])

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
