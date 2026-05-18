import Cocoa

/// Floating panel used for both capturing new entries and editing existing ones.
/// Hosts an editable text view, a category picker (with inline "New Category…" creation),
/// and Save/Cancel buttons.
final class DetailedCapturePanel {
    static let shared = DetailedCapturePanel()

    /// Sentinel tag on the category popup's "+ New Category…" item.
    private static let newCategoryTag = -1

    private var panel: NSPanel?
    private var textView: NSTextView?
    private var categoryPopup: NSPopUpButton?

    /// When non-nil, the panel is editing an existing entry rather than creating one.
    private var editingEntryId: Int64?
    /// Last valid selected category tag, used to revert if the user cancels the "New Category" prompt.
    private var lastSelectedCategoryTag: Int = 0

    private init() {}

    /// Show the panel in capture mode, pre-filled with the given text (empty string for a blank note).
    func show(with text: String) {
        present(text: text, editingEntryId: nil, selectedCategoryId: nil, title: "Capture to ThoughtQueue")
    }

    /// Show the panel in edit mode for an existing entry.
    func showEditing(entry: Entry) {
        present(text: entry.text, editingEntryId: entry.id, selectedCategoryId: entry.categoryId, title: "Edit Entry")
    }

    /// Build and display the panel. `selectedCategoryId` is the tag to pre-select in the popup.
    private func present(text: String, editingEntryId: Int64?, selectedCategoryId: Int64?, title: String) {
        panel?.close()
        self.editingEntryId = editingEntryId

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
        newPanel.title = title
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
        popup.target = self
        popup.action = #selector(categoryPopupChanged(_:))
        self.categoryPopup = popup
        populateCategoryPopup(selecting: selectedCategoryId)

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
        newPanel.makeFirstResponder(tv)
        NSApp.activate(ignoringOtherApps: true)
        panel = newPanel
    }

    /// Rebuild the category popup from the database, selecting the item whose categoryId tag matches.
    /// Pass nil to select "Uncategorized".
    private func populateCategoryPopup(selecting selectedCategoryId: Int64?) {
        guard let popup = categoryPopup else { return }
        popup.removeAllItems()

        popup.addItem(withTitle: "Uncategorized")
        popup.menu?.items.last?.tag = 0

        for cat in DatabaseManager.shared.fetchCategories() {
            popup.addItem(withTitle: cat.name)
            popup.menu?.items.last?.tag = Int(cat.id)
        }

        popup.menu?.addItem(NSMenuItem.separator())
        let newItem = NSMenuItem(title: "+ New Category\u{2026}", action: nil, keyEquivalent: "")
        newItem.tag = Self.newCategoryTag
        popup.menu?.addItem(newItem)

        let targetTag = selectedCategoryId.map { Int($0) } ?? 0
        popup.selectItem(withTag: targetTag)
        lastSelectedCategoryTag = popup.selectedItem?.tag ?? 0
    }

    /// Handle popup selection. If the user picked the "New Category" sentinel, prompt for a name
    /// and rebuild the popup with the new category selected; otherwise just remember the choice.
    @objc private func categoryPopupChanged(_ sender: NSPopUpButton) {
        let tag = sender.selectedItem?.tag ?? 0
        guard tag == Self.newCategoryTag else {
            lastSelectedCategoryTag = tag
            return
        }
        promptForNewCategory()
    }

    /// Show a modal NSAlert to collect a category name, create it, and refresh the popup.
    /// If the user cancels or provides an empty name, restore the previously selected category.
    private func promptForNewCategory() {
        let alert = NSAlert()
        alert.messageText = "New Category"
        alert.informativeText = "Enter a name for the new category:"
        alert.addButton(withTitle: "Create")
        alert.addButton(withTitle: "Cancel")

        let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 250, height: 24))
        alert.accessoryView = input

        if let panel = panel {
            alert.beginSheetModal(for: panel) { [weak self] response in
                self?.handleNewCategoryResponse(response, name: input.stringValue)
            }
        } else {
            let response = alert.runModal()
            handleNewCategoryResponse(response, name: input.stringValue)
        }
    }

    private func handleNewCategoryResponse(_ response: NSApplication.ModalResponse, name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard response == .alertFirstButtonReturn, !trimmed.isEmpty else {
            categoryPopup?.selectItem(withTag: lastSelectedCategoryTag)
            return
        }

        if let newCat = DatabaseManager.shared.createCategory(name: trimmed) {
            NotificationCenter.default.post(name: .entriesDidChange, object: nil)
            populateCategoryPopup(selecting: newCat.id)
        } else {
            // Creation failed (likely duplicate name) — fall back to whatever exists with that name.
            let existing = DatabaseManager.shared.fetchCategories().first(where: { $0.name == trimmed })
            populateCategoryPopup(selecting: existing?.id)
        }
    }

    /// Save action. Persists either a new entry or updates the entry currently being edited.
    @objc private func save() {
        guard let tv = textView, let popup = categoryPopup else {
            NSLog("[ThoughtQueue] DetailedCapture save failed — missing references")
            return
        }

        let text = tv.string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }

        let selectedTag = popup.selectedItem?.tag ?? 0
        let categoryId: Int64? = selectedTag == 0 ? nil : Int64(selectedTag)

        if let editingId = editingEntryId {
            let textOK = DatabaseManager.shared.updateEntry(id: editingId, text: text)
            let moveOK = DatabaseManager.shared.moveEntry(id: editingId, toCategoryId: categoryId)
            if textOK || moveOK {
                NotificationCenter.default.post(name: .entriesDidChange, object: nil)
                ToastWindow.show(message: "Updated!")
            }
        } else {
            if let _ = DatabaseManager.shared.createEntry(text: text, categoryId: categoryId) {
                NotificationCenter.default.post(name: .entriesDidChange, object: nil)
                ToastWindow.show(message: "Captured!")
            }
        }

        closeAndReset()
    }

    @objc private func cancel() {
        closeAndReset()
    }

    private func closeAndReset() {
        panel?.close()
        panel = nil
        textView = nil
        categoryPopup = nil
        editingEntryId = nil
    }
}
