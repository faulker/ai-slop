import Cocoa

private let entryDragType = NSPasteboard.PasteboardType("com.txtmem.entry")

// MARK: - Main Window Controller

final class MainWindowController: NSWindowController {
    static let shared = MainWindowController()

    private let splitView = NSSplitViewController()
    private let sidebarVC = CategorySidebarViewController()
    private let entriesVC = EntriesListViewController()

    convenience init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 800, height: 550),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "ThoughtQueue"
        window.center()
        window.setFrameAutosaveName("ThoughtQueueMainWindow")

        self.init(window: window)

        sidebarVC.onCategorySelected = { [weak self] categoryId in
            if categoryId == CategorySidebarViewController.allSentinel {
                self?.entriesVC.showAllEntries()
            } else {
                self?.entriesVC.showEntries(forCategoryId: categoryId)
            }
        }

        let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarVC)
        sidebarItem.minimumThickness = 180
        sidebarItem.maximumThickness = 280
        sidebarItem.canCollapse = false

        let contentItem = NSSplitViewItem(viewController: entriesVC)
        contentItem.minimumThickness = 400

        splitView.addSplitViewItem(sidebarItem)
        splitView.addSplitViewItem(contentItem)

        window.contentViewController = splitView
    }

    override func showWindow(_ sender: Any?) {
        super.showWindow(sender)
        sidebarVC.reload()
        sidebarVC.selectAll()
        entriesVC.showAllEntries()
    }
}

// MARK: - Custom Row View (full-width rounded highlight)

final class CategoryRowView: NSTableRowView {
    override var isEmphasized: Bool {
        get { true }
        set { }
    }

    override func drawSelection(in dirtyRect: NSRect) {
        let selectionRect = bounds.insetBy(dx: 4, dy: 2)
        let path = NSBezierPath(roundedRect: selectionRect, xRadius: 6, yRadius: 6)
        NSColor.controlAccentColor.withAlphaComponent(0.2).setFill()
        path.fill()
    }
}

// MARK: - Category Sidebar

final class CategorySidebarViewController: NSViewController {
    var onCategorySelected: ((Int64?) -> Void)?
    private let tableView = NSTableView()
    private var categories: [Category] = []
    private var selectedCategoryId: Int64? = nil
    private var reloadWorkItem: DispatchWorkItem?

    override func loadView() {
        let container = NSView(frame: NSRect(x: 0, y: 0, width: 200, height: 550))

        let scrollView = NSScrollView()
        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.drawsBackground = false

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("category"))
        column.title = "Categories"
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.delegate = self
        tableView.dataSource = self
        tableView.rowHeight = 40
        tableView.selectionHighlightStyle = .regular
        tableView.backgroundColor = .clear
        tableView.intercellSpacing = NSSize(width: 0, height: 2)

        tableView.registerForDraggedTypes([entryDragType])

        let contextMenu = NSMenu()
        contextMenu.delegate = self
        tableView.menu = contextMenu

        container.addSubview(scrollView)

        let addButton = NSButton(title: "+ Add Category", target: self, action: #selector(addCategory))
        addButton.bezelStyle = .inline
        addButton.font = .systemFont(ofSize: 13)
        addButton.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(addButton)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.topAnchor, constant: 8),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: addButton.topAnchor, constant: -8),
            addButton.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 12),
            addButton.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -12),
            addButton.heightAnchor.constraint(equalToConstant: 32),
        ])

        self.view = container

        NotificationCenter.default.addObserver(self, selector: #selector(onEntriesChanged), name: .entriesDidChange, object: nil)
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    @objc private func onEntriesChanged() {
        let scheduleReload = { [weak self] in
            self?.reloadWorkItem?.cancel()
            let item = DispatchWorkItem { [weak self] in self?.reload() }
            self?.reloadWorkItem = item
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1, execute: item)
        }
        if Thread.isMainThread { scheduleReload() }
        else { DispatchQueue.main.async { scheduleReload() } }
    }

    func reload() {
        categories = DatabaseManager.shared.fetchCategories()
        tableView.reloadData()
    }

    func selectAll() {
        tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
    }

    @objc private func addCategory() {
        let alert = NSAlert()
        alert.messageText = "New Category"
        alert.informativeText = "Enter a name for the new category:"
        alert.addButton(withTitle: "Create")
        alert.addButton(withTitle: "Cancel")

        let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 250, height: 24))
        alert.accessoryView = input

        if alert.runModal() == .alertFirstButtonReturn {
            let name = input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if !name.isEmpty {
                _ = DatabaseManager.shared.createCategory(name: name)
                reload()
            }
        }
    }

    // Row mapping: 0 = All, 1 = Uncategorized, 2+ = user categories
    static let allSentinel: Int64 = -999
    static let uncategorizedSentinel: Int64 = -1

    func categoryId(forRow row: Int) -> Int64? {
        if row == 0 { return Self.allSentinel }
        if row == 1 { return nil } // uncategorized
        let catIndex = row - 2
        if catIndex >= 0 && catIndex < categories.count {
            return categories[catIndex].id
        }
        return nil
    }

    // MARK: - Context menu for categories

    @objc private func renameCategory(_ sender: NSMenuItem) {
        guard let catId = sender.representedObject as? Int64 else { return }
        guard let cat = categories.first(where: { $0.id == catId }) else { return }

        let alert = NSAlert()
        alert.messageText = "Rename Category"
        alert.informativeText = "Enter a new name:"
        alert.addButton(withTitle: "Rename")
        alert.addButton(withTitle: "Cancel")

        let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 250, height: 24))
        input.stringValue = cat.name
        alert.accessoryView = input

        if alert.runModal() == .alertFirstButtonReturn {
            let name = input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if !name.isEmpty {
                _ = DatabaseManager.shared.renameCategory(id: catId, name: name)
                reload()
                NotificationCenter.default.post(name: .entriesDidChange, object: nil)
            }
        }
    }

    @objc private func deleteCategory(_ sender: NSMenuItem) {
        guard let catId = sender.representedObject as? Int64 else { return }
        guard let cat = categories.first(where: { $0.id == catId }) else { return }

        let count = DatabaseManager.shared.fetchEntriesCount(categoryId: catId)

        let alert = NSAlert()
        alert.messageText = "Delete \"\(cat.name)\"?"
        if count > 0 {
            alert.informativeText = "This category has \(count) entry\(count == 1 ? "" : "ies"). What should happen to them?"
            alert.addButton(withTitle: "Move to Uncategorized")
            alert.addButton(withTitle: "Delete All Entries")
            alert.addButton(withTitle: "Cancel")
        } else {
            alert.informativeText = "This category is empty."
            alert.addButton(withTitle: "Delete")
            alert.addButton(withTitle: "Cancel")
        }

        let response = alert.runModal()
        if count > 0 {
            if response == .alertFirstButtonReturn {
                DatabaseManager.shared.deleteCategory(id: catId, moveToUncategorized: true)
            } else if response == .alertSecondButtonReturn {
                DatabaseManager.shared.deleteCategory(id: catId, moveToUncategorized: false)
            } else {
                return
            }
        } else {
            if response == .alertFirstButtonReturn {
                DatabaseManager.shared.deleteCategory(id: catId, moveToUncategorized: true)
            } else {
                return
            }
        }

        reload()
        NotificationCenter.default.post(name: .entriesDidChange, object: nil)
    }
}

extension CategorySidebarViewController: NSMenuDelegate {
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let clickedRow = tableView.clickedRow
        let catIndex = clickedRow - 2
        guard catIndex >= 0, catIndex < categories.count else { return }
        let cat = categories[catIndex]

        let renameItem = NSMenuItem(title: "Rename", action: #selector(renameCategory(_:)), keyEquivalent: "")
        renameItem.target = self
        renameItem.representedObject = cat.id
        menu.addItem(renameItem)

        let deleteItem = NSMenuItem(title: "Delete", action: #selector(deleteCategory(_:)), keyEquivalent: "")
        deleteItem.target = self
        deleteItem.representedObject = cat.id
        menu.addItem(deleteItem)
    }
}

extension CategorySidebarViewController: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        return categories.count + 2
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        return CategoryRowView()
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let container = NSStackView()
        container.orientation = .horizontal
        container.distribution = .fill
        container.spacing = 4
        container.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12)

        let nameLabel = NSTextField(labelWithString: "")
        nameLabel.font = .systemFont(ofSize: 14, weight: .semibold)
        nameLabel.lineBreakMode = .byTruncatingTail
        nameLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let countLabel = NSTextField(labelWithString: "")
        countLabel.font = .systemFont(ofSize: 12)
        countLabel.textColor = .secondaryLabelColor
        countLabel.alignment = .right
        countLabel.setContentHuggingPriority(.required, for: .horizontal)

        if row == 0 {
            let count = DatabaseManager.shared.fetchTotalEntriesCount()
            nameLabel.stringValue = "All"
            countLabel.stringValue = "\(count)"
        } else if row == 1 {
            let count = DatabaseManager.shared.fetchEntriesCount(categoryId: nil)
            nameLabel.stringValue = "Uncategorized"
            countLabel.stringValue = "\(count)"
        } else {
            let cat = categories[row - 2]
            let count = DatabaseManager.shared.fetchEntriesCount(categoryId: cat.id)
            nameLabel.stringValue = cat.name
            countLabel.stringValue = "\(count)"
        }

        container.addArrangedSubview(nameLabel)
        container.addArrangedSubview(countLabel)
        return container
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        let row = tableView.selectedRow
        guard row >= 0 else { return }
        if row == 0 {
            selectedCategoryId = Self.allSentinel
            onCategorySelected?(Self.allSentinel)
        } else if row == 1 {
            selectedCategoryId = nil
            onCategorySelected?(nil)
        } else {
            let catIndex = row - 2
            if catIndex < categories.count {
                let cat = categories[catIndex]
                selectedCategoryId = cat.id
                onCategorySelected?(cat.id)
            }
        }
    }

    // MARK: - Drag-and-drop (accept entries onto categories)

    func tableView(_ tableView: NSTableView, validateDrop info: NSDraggingInfo, proposedRow row: Int, proposedDropOperation dropOperation: NSTableView.DropOperation) -> NSDragOperation {
        if dropOperation == .on && row >= 1 {
            return .move
        }
        return []
    }

    func tableView(_ tableView: NSTableView, acceptDrop info: NSDraggingInfo, row: Int, dropOperation: NSTableView.DropOperation) -> Bool {
        guard let data = info.draggingPasteboard.data(forType: entryDragType),
              let entryId = String(data: data, encoding: .utf8).flatMap({ Int64($0) }) else {
            return false
        }

        let targetCategoryId = categoryId(forRow: row)
        _ = DatabaseManager.shared.moveEntry(id: entryId, toCategoryId: targetCategoryId)
        NotificationCenter.default.post(name: .entriesDidChange, object: nil)
        return true
    }
}

// MARK: - Entry Card View

final class EntryCardView: NSView {
    let entryId: Int64
    private let entry: Entry
    private weak var actionTarget: EntriesListViewController?

    init(entry: Entry, target: EntriesListViewController) {
        self.entry = entry
        self.entryId = entry.id
        self.actionTarget = target
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.borderColor = NSColor.separatorColor.cgColor
        alphaValue = entry.isSent ? 0.6 : 1.0
        setupUI()
    }

    required init?(coder: NSCoder) { fatalError() }

    override func updateLayer() {
        layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor
        layer?.borderColor = NSColor.separatorColor.cgColor
    }

    private func setupUI() {
        let textView = NSTextView()
        textView.string = entry.text
        textView.font = .systemFont(ofSize: 13)
        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = false
        textView.drawsBackground = false
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.textContainerInset = NSSize(width: 4, height: 4)
        textView.textContainer?.widthTracksTextView = true

        let textScroll = NSScrollView()
        textScroll.documentView = textView
        textScroll.hasVerticalScroller = true
        textScroll.autohidesScrollers = true
        textScroll.drawsBackground = false
        textScroll.borderType = .noBorder
        textScroll.translatesAutoresizingMaskIntoConstraints = false

        addSubview(textScroll)

        let maxTextHeight: CGFloat = 100
        let minTextHeight: CGFloat = 30

        // Bottom bar
        let bottomBar = NSStackView()
        bottomBar.orientation = .horizontal
        bottomBar.distribution = .fill
        bottomBar.spacing = 8
        bottomBar.translatesAutoresizingMaskIntoConstraints = false
        addSubview(bottomBar)

        let sentBtn = EntryButton(title: "", target: self, action: #selector(toggleSent))
        sentBtn.entryId = entry.id
        sentBtn.image = NSImage(systemSymbolName: entry.isSent ? "checkmark.circle.fill" : "circle", accessibilityDescription: entry.isSent ? "Mark as unread" : "Mark as read")
        sentBtn.imagePosition = .imageOnly
        sentBtn.bezelStyle = .inline
        sentBtn.isBordered = false
        sentBtn.toolTip = entry.isSent ? "Mark as unread" : "Mark as read"
        sentBtn.contentTintColor = entry.isSent ? .systemGreen : .secondaryLabelColor
        bottomBar.addArrangedSubview(sentBtn)

        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        bottomBar.addArrangedSubview(spacer)

        let openBtn = makeActionButton(title: "Open", symbolName: "arrow.up.forward.app", action: #selector(openInClaude))
        let moveBtn = makeActionButton(title: "Move", symbolName: "folder", action: #selector(showMoveMenu(_:)))
        let deleteBtn = makeActionButton(title: "Delete", symbolName: "trash", action: #selector(confirmDelete))
        deleteBtn.contentTintColor = .systemRed

        bottomBar.addArrangedSubview(openBtn)
        bottomBar.addArrangedSubview(moveBtn)
        bottomBar.addArrangedSubview(deleteBtn)

        textHeightConstraint = textScroll.heightAnchor.constraint(equalToConstant: maxTextHeight)
        textHeightConstraint?.isActive = true

        NSLayoutConstraint.activate([
            textScroll.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            textScroll.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            textScroll.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),

            bottomBar.topAnchor.constraint(equalTo: textScroll.bottomAnchor, constant: 8),
            bottomBar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            bottomBar.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            bottomBar.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
            bottomBar.heightAnchor.constraint(equalToConstant: 28),
        ])

        self.textView = textView
        self.textScrollView = textScroll
        self.maxTextHeight = maxTextHeight
        self.minTextHeight = minTextHeight
    }

    private var textView: NSTextView?
    private var textScrollView: NSScrollView?
    private var textHeightConstraint: NSLayoutConstraint?
    private var maxTextHeight: CGFloat = 100
    private var minTextHeight: CGFloat = 30

    override func layout() {
        super.layout()
        updateTextHeight()
    }

    private func updateTextHeight() {
        guard let textView = textView, let textScrollView = textScrollView else { return }
        let containerWidth = textScrollView.frame.width - 8
        guard containerWidth > 0 else { return }

        textView.textContainer?.containerSize = NSSize(width: containerWidth, height: .greatestFiniteMagnitude)
        textView.layoutManager?.ensureLayout(for: textView.textContainer!)

        let naturalHeight = textView.layoutManager?.usedRect(for: textView.textContainer!).height ?? 50
        let idealHeight = naturalHeight + 12
        let clampedHeight = min(max(idealHeight, minTextHeight), maxTextHeight)
        textHeightConstraint?.constant = clampedHeight
    }

    private func makeActionButton(title: String, symbolName: String, action: Selector) -> NSButton {
        let btn = EntryButton(title: title, target: self, action: action)
        btn.entryId = entry.id
        btn.image = NSImage(systemSymbolName: symbolName, accessibilityDescription: title)
        btn.imagePosition = .imageLeading
        btn.bezelStyle = .rounded
        btn.controlSize = .regular
        btn.font = .systemFont(ofSize: 12)
        return btn
    }

    // MARK: - Actions

    @objc private func toggleSent() {
        _ = DatabaseManager.shared.toggleEntrySent(id: entry.id, isSent: !entry.isSent)
        NotificationCenter.default.post(name: .entriesDidChange, object: nil)
    }

    @objc private func openInClaude() {
        ClaudeIntegration.shared.sendToClaude(entry: entry)
    }

    @objc private func showMoveMenu(_ sender: NSButton) {
        let menu = NSMenu()

        let uncatItem = NSMenuItem(title: "Uncategorized", action: #selector(performMove(_:)), keyEquivalent: "")
        uncatItem.target = self
        uncatItem.representedObject = MoveInfo(entryId: entry.id, categoryId: nil)
        menu.addItem(uncatItem)
        menu.addItem(NSMenuItem.separator())

        for cat in DatabaseManager.shared.fetchCategories() {
            let item = NSMenuItem(title: cat.name, action: #selector(performMove(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = MoveInfo(entryId: entry.id, categoryId: cat.id)
            menu.addItem(item)
        }

        menu.popUp(positioning: nil, at: sender.bounds.origin, in: sender)
    }

    @objc private func performMove(_ sender: NSMenuItem) {
        guard let info = sender.representedObject as? MoveInfo else { return }
        _ = DatabaseManager.shared.moveEntry(id: info.entryId, toCategoryId: info.categoryId)
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

    // MARK: - Drag source

    override func mouseDown(with event: NSEvent) {
        super.mouseDown(with: event)
    }

    override func mouseDragged(with event: NSEvent) {
        let item = NSDraggingItem(pasteboardWriter: NSPasteboardItem())
        (item.item as? NSPasteboardItem)?.setString(String(entryId), forType: entryDragType)

        let dragImage = NSImage(size: bounds.size, flipped: false) { [weak self] rect in
            guard let self = self else { return false }
            NSColor.controlAccentColor.withAlphaComponent(0.15).setFill()
            NSBezierPath(roundedRect: self.bounds, xRadius: 8, yRadius: 8).fill()
            return true
        }
        item.setDraggingFrame(bounds, contents: dragImage)

        beginDraggingSession(with: [item], event: event, source: self)
    }
}

extension EntryCardView: NSDraggingSource {
    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        return context == .withinApplication ? .move : []
    }
}

// MARK: - Entries List (card-based)

final class EntriesListViewController: NSViewController {
    private var scrollView: NSScrollView!
    private var stackView: NSStackView!
    private var entries: [Entry] = []
    private var currentCategoryId: Int64? = nil
    private var showAll = true
    private var reloadWorkItem: DispatchWorkItem?

    override func loadView() {
        let container = NSView(frame: NSRect(x: 0, y: 0, width: 500, height: 550))

        stackView = NSStackView()
        stackView.orientation = .vertical
        stackView.alignment = .leading
        stackView.spacing = 12
        stackView.translatesAutoresizingMaskIntoConstraints = false

        scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        let docView = FlippedClipView()
        docView.translatesAutoresizingMaskIntoConstraints = false
        docView.addSubview(stackView)
        scrollView.documentView = docView

        NSLayoutConstraint.activate([
            stackView.topAnchor.constraint(equalTo: docView.topAnchor, constant: 8),
            stackView.leadingAnchor.constraint(equalTo: docView.leadingAnchor, constant: 8),
            stackView.trailingAnchor.constraint(equalTo: docView.trailingAnchor, constant: -8),
            stackView.bottomAnchor.constraint(lessThanOrEqualTo: docView.bottomAnchor, constant: -8),
            docView.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
        ])

        container.addSubview(scrollView)

        let clearButton = NSButton(title: "Clear Completed", target: self, action: #selector(clearCompleted))
        clearButton.bezelStyle = .rounded
        clearButton.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(clearButton)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: clearButton.topAnchor, constant: -4),
            clearButton.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            clearButton.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -12),
        ])

        self.view = container

        NotificationCenter.default.addObserver(self, selector: #selector(onEntriesChanged), name: .entriesDidChange, object: nil)
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    @objc private func onEntriesChanged() {
        let scheduleReload = { [weak self] in
            self?.reloadWorkItem?.cancel()
            let item = DispatchWorkItem { [weak self] in self?.reload() }
            self?.reloadWorkItem = item
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1, execute: item)
        }
        if Thread.isMainThread { scheduleReload() }
        else { DispatchQueue.main.async { scheduleReload() } }
    }

    func showAllEntries() {
        showAll = true
        currentCategoryId = nil
        reload()
    }

    func showEntries(forCategoryId categoryId: Int64?) {
        currentCategoryId = categoryId
        showAll = false
        reload()
    }

    func reload() {
        _ = view // ensure loadView called

        if showAll {
            entries = DatabaseManager.shared.fetchEntries()
        } else if let catId = currentCategoryId {
            entries = DatabaseManager.shared.fetchEntries(categoryId: catId)
        } else {
            entries = DatabaseManager.shared.fetchUncategorizedEntries()
        }

        rebuildCards()
    }

    private func rebuildCards() {
        stackView.arrangedSubviews.forEach { $0.removeFromSuperview() }

        if entries.isEmpty {
            let emptyLabel = NSTextField(labelWithString: "No entries in this category.")
            emptyLabel.font = .systemFont(ofSize: 13)
            emptyLabel.textColor = .secondaryLabelColor
            emptyLabel.alignment = .center
            emptyLabel.translatesAutoresizingMaskIntoConstraints = false
            stackView.addArrangedSubview(emptyLabel)
            emptyLabel.widthAnchor.constraint(equalTo: stackView.widthAnchor).isActive = true
        } else {
            for entry in entries {
                let card = EntryCardView(entry: entry, target: self)
                card.translatesAutoresizingMaskIntoConstraints = false
                stackView.addArrangedSubview(card)
                card.widthAnchor.constraint(equalTo: stackView.widthAnchor).isActive = true
            }
        }

        updateDocumentViewHeight()
    }

    private func updateDocumentViewHeight() {
        guard let docView = stackView.superview else { return }
        stackView.layoutSubtreeIfNeeded()
        let contentHeight = stackView.fittingSize.height + 16
        let visibleHeight = scrollView.contentView.bounds.height
        docView.frame.size.height = max(contentHeight, visibleHeight)
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        updateDocumentViewHeight()
    }

    @objc private func clearCompleted() {
        let count = DatabaseManager.shared.clearCompletedEntries()
        if count > 0 {
            ToastWindow.show(message: "Cleared \(count) completed")
            NotificationCenter.default.post(name: .entriesDidChange, object: nil)
        }
    }
}

// MARK: - Flipped clip view (content starts at top)

final class FlippedClipView: NSView {
    override var isFlipped: Bool { true }
}

// MARK: - Helper Types

final class EntryButton: NSButton {
    var entryId: Int64 = 0
}

final class MoveInfo: NSObject {
    let entryId: Int64
    let categoryId: Int64?
    init(entryId: Int64, categoryId: Int64?) {
        self.entryId = entryId
        self.categoryId = categoryId
    }
}
