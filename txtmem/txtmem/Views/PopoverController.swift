import Cocoa

final class PopoverController: NSObject {
    private let popover = NSPopover()
    private let viewController = PopoverViewController()

    override init() {
        super.init()
        popover.contentViewController = viewController
        popover.behavior = .transient
        popover.contentSize = NSSize(width: 360, height: 400)
    }

    func toggle(relativeTo rect: NSRect, of view: NSView) {
        if popover.isShown {
            popover.close()
        } else {
            viewController.reload()
            popover.show(relativeTo: rect, of: view, preferredEdge: .minY)
            popover.contentViewController?.view.window?.makeKey()
        }
    }
}

final class PopoverViewController: NSViewController {
    private var stackView: NSStackView!
    private var categories: [Category] = []
    private var uncategorizedEntries: [Entry] = []
    private var expandedCategories: Set<Int64> = [-1]  // Uncategorized expanded by default
    private var entriesByCategory: [Int64: [Entry]] = [:]
    private var reloadWorkItem: DispatchWorkItem?

    override func loadView() {
        let container = NSView(frame: NSRect(x: 0, y: 0, width: 360, height: 400))

        stackView = NSStackView()
        stackView.orientation = .vertical
        stackView.alignment = .width
        stackView.spacing = 0

        let scrollView = NSScrollView(frame: container.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder

        let flipView = FlippedView(frame: NSRect(x: 0, y: 0, width: 344, height: 0))
        flipView.autoresizingMask = .width
        flipView.addSubview(stackView)
        stackView.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            stackView.topAnchor.constraint(equalTo: flipView.topAnchor, constant: 8),
            stackView.leadingAnchor.constraint(equalTo: flipView.leadingAnchor, constant: 8),
            stackView.trailingAnchor.constraint(equalTo: flipView.trailingAnchor, constant: -8),
            stackView.bottomAnchor.constraint(equalTo: flipView.bottomAnchor, constant: -8),
        ])

        scrollView.documentView = flipView
        container.addSubview(scrollView)

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
        _ = view  // ensure loadView() has been called
        let db = DatabaseManager.shared
        categories = db.fetchCategories()
        uncategorizedEntries = db.fetchUncategorizedEntries()
        entriesByCategory = [:]
        for cat in categories {
            entriesByCategory[cat.id] = db.fetchEntries(categoryId: cat.id)
        }
        rebuildUI()
    }

    private func rebuildUI() {
        stackView.arrangedSubviews.forEach { $0.removeFromSuperview() }

        // Uncategorized section — always present
        addSectionHeader(title: "Uncategorized", count: uncategorizedEntries.count, categoryId: nil)
        if expandedCategories.contains(-1) {
            for entry in uncategorizedEntries {
                addEntryRow(entry)
            }
        }

        // User categories
        for cat in categories {
            let entries = entriesByCategory[cat.id] ?? []
            addSectionHeader(title: cat.name, count: entries.count, categoryId: cat.id)
            if expandedCategories.contains(cat.id) {
                for entry in entries {
                    addEntryRow(entry)
                }
            }
        }

        // Bottom separator
        stackView.addArrangedSubview(makeSeparator())

        layoutDocumentView()
    }

    private func layoutDocumentView() {
        stackView.layoutSubtreeIfNeeded()
        let contentHeight = stackView.fittingSize.height + 16
        if let docView = stackView.superview {
            docView.frame.size.height = max(contentHeight, view.bounds.height)
        }
    }

    private func makeSeparator() -> NSBox {
        let sep = NSBox()
        sep.boxType = .separator
        sep.translatesAutoresizingMaskIntoConstraints = false
        return sep
    }

    private func addSectionHeader(title: String, count: Int, categoryId: Int64?) {
        let effectiveId = categoryId ?? -1
        let isExpanded = expandedCategories.contains(effectiveId)

        stackView.addArrangedSubview(makeSeparator())

        let header = CategoryHeaderView(
            title: title, count: count, isExpanded: isExpanded, categoryId: effectiveId
        ) { [weak self] id in
            self?.toggleCategory(id)
        }
        stackView.addArrangedSubview(header)
    }

    private func toggleCategory(_ effectiveId: Int64) {
        if expandedCategories.contains(effectiveId) {
            expandedCategories.remove(effectiveId)
        } else {
            expandedCategories.insert(effectiveId)
        }
        rebuildUI()
    }

    private func addEntryRow(_ entry: Entry) {
        let row = EntryRowView(entry: entry, compact: true)
        stackView.addArrangedSubview(row)
    }
}

final class CategoryHeaderView: NSView {
    private let categoryId: Int64
    private let onTap: (Int64) -> Void

    init(title: String, count: Int, isExpanded: Bool, categoryId: Int64, onTap: @escaping (Int64) -> Void) {
        self.categoryId = categoryId
        self.onTap = onTap
        super.init(frame: .zero)

        let chevron = isExpanded ? "\u{25BE}" : "\u{25B8}"  // down / right
        let label = NSTextField(labelWithString: "\(chevron)   \(title)  (\(count))")
        label.font = .systemFont(ofSize: 13, weight: .semibold)
        label.textColor = .controlTextColor
        label.translatesAutoresizingMaskIntoConstraints = false

        addSubview(label)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 32),
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    override func mouseDown(with event: NSEvent) {
        onTap(categoryId)
    }
}

final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}
