import SwiftUI

/// Which entity family the Codeplug editor is currently showing.
enum CodeplugSection: String, CaseIterable, Identifiable {
    case channels = "Channels"
    case zones = "Zones"
    case scanLists = "Scan Lists"
    case contacts = "Contacts"
    case groupLists = "Group Lists"
    case radioIds = "Radio IDs"
    case aprs = "APRS"

    var id: String { rawValue }

    var symbol: String {
        switch self {
        case .channels: return "list.bullet"
        case .zones: return "square.grid.2x2"
        case .scanLists: return "antenna.radiowaves.left.and.right"
        case .contacts: return "person.2"
        case .groupLists: return "person.3"
        case .radioIds: return "number"
        case .aprs: return "location.circle"
        }
    }
}

/// Codeplug detail: the pane for one entity family, or the empty state when no
/// file is open.
struct CodeplugView: View {
    let section: CodeplugSection
    @EnvironmentObject private var store: CodeplugStore
    @EnvironmentObject private var device: DeviceStore

    var body: some View {
        Group {
            if store.fileURL == nil {
                noFileState
            } else {
                switch section {
                case .channels: ChannelsPane()
                case .zones: ZonesPane()
                case .scanLists: ScanListsPane()
                case .contacts: ContactsPane()
                case .groupLists: GroupListsPane()
                case .radioIds: RadioIDsPane()
                case .aprs: APRSPane()
                }
            }
        }
        .toolbar { CodeplugFileToolbar() }
        // Populate the Write-to-Radio dropdown without a trip to the Device tab.
        .onAppear { device.refreshPorts() }
    }

    private var noFileState: some View {
        VStack(spacing: Spacing.stack) {
            Image(systemName: "doc.badge.plus")
                .font(.system(size: 40))
                .foregroundStyle(.secondary)
            Text("No codeplug open")
                .font(.title3.weight(.medium))
            Text("Open a .bin file to view and edit channels, zones, contacts, "
                + "group lists, and radio IDs. You can create one from the Device "
                + "tab with Read from Radio.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 380)
            Button("Open Codeplug…") { store.openWithPanel() }
                .keyboardShortcut("o")
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Save/Write-to-radio, shared by every codeplug pane. Open and Close live in
/// `FileToolbar` at the window level, since with no file open these panes — and
/// this toolbar — aren't reachable.
struct CodeplugFileToolbar: ToolbarContent {
    @EnvironmentObject private var store: CodeplugStore
    @EnvironmentObject private var device: DeviceStore
    @State private var confirmWrite = false

    var body: some ToolbarContent {
        ToolbarItem {
            // A split button: click Save, or pick Save As from the chevron. Save
            // As stays live with nothing staged, since saving an unmodified
            // codeplug under a new name is a reasonable thing to want.
            Menu {
                Button("Save") { store.save() }
                    .disabled(!store.isDirty)
                Button("Save As…") { store.saveAs() }
            } label: {
                Label("Save", systemImage: "square.and.arrow.down")
            } primaryAction: {
                store.save()
            }
            .help("Write staged changes to the file")
            // Without this the toolbar Menu takes its accessibility name from
            // the SF Symbol and announces itself as "download".
            .accessibilityLabel("Save")
            .disabled(store.fileURL == nil)
        }
        ToolbarItem {
            // A dropdown rather than a single dead button: it lists the connected
            // radios so the user picks the target directly, and stays live even
            // with nothing connected so Refresh is always reachable.
            Menu {
                if device.radios.isEmpty {
                    Text("No radios connected")
                } else {
                    ForEach(device.radios) { radio in
                        Button(radio.product ?? radio.name) {
                            device.selectedPort = radio.name
                            confirmWrite = true
                        }
                        .disabled(device.busy)
                    }
                }
                Divider()
                Button("Refresh") { device.refreshPorts() }
            } label: {
                Label("Write to Radio", systemImage: "antenna.radiowaves.left.and.right")
            }
            .help("Write this codeplug, including unsaved changes, to a connected radio")
            .accessibilityLabel("Write to Radio")
            .disabled(store.fileURL == nil)
            .confirmationDialog("Write this codeplug to the radio?",
                                isPresented: $confirmWrite) {
                Button("Write to Radio", role: .destructive) {
                    // The work file, not the document: what gets written is what
                    // the user is looking at, staged changes included.
                    device.restore(from: Recovery.workURL,
                                   displayName: store.fileURL?.lastPathComponent ?? "codeplug")
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("""
                This OVERWRITES the radio's entire configuration with what you see \
                here\(store.isDirty ? ", including changes you haven't saved to the file yet" : ""). \
                Read the radio's current codeplug to a file first if you don't have a backup. \
                Do not disconnect the cable or power off the radio during the write.
                """)
            }
        }
    }
}

/// Open and Close a codeplug, always reachable from the window toolbar no matter
/// which pane is showing. This lives above the panes (attached in `ContentView`)
/// because the app starts with no file open, and the codeplug panes that carry
/// the rest of the file chrome are disabled until there is one — so without this
/// the only way to open a file was the File menu.
struct FileToolbar: ToolbarContent {
    @EnvironmentObject private var store: CodeplugStore
    @State private var confirmOpen = false
    @State private var confirmClose = false

    var body: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            Button {
                // Opening drops the staged work file, so ask before discarding it.
                if store.isDirty { confirmOpen = true } else { store.openWithPanel() }
            } label: {
                Label("Open", systemImage: "folder")
            }
            .help("Open a codeplug .bin")
            .confirmationDialog("Save changes before opening another codeplug?",
                                isPresented: $confirmOpen) {
                Button("Save") { store.save(); if !store.isDirty { store.openWithPanel() } }
                Button("Discard", role: .destructive) { store.openWithPanel() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Your staged changes haven't been written to "
                    + "\(store.fileURL?.lastPathComponent ?? "the file") yet.")
            }
        }
        if store.fileURL != nil {
            ToolbarItem(placement: .navigation) {
                Button {
                    if store.isDirty { confirmClose = true } else { store.close() }
                } label: {
                    Label("Close", systemImage: "xmark.circle")
                }
                .help("Close the open codeplug")
                .confirmationDialog("Save changes before closing?",
                                    isPresented: $confirmClose) {
                    // Only close if the save actually succeeded, so a failed write
                    // can't silently throw the changes away.
                    Button("Save") { store.save(); if !store.isDirty { store.close() } }
                    Button("Discard", role: .destructive) { store.close() }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text("Your staged changes haven't been written to "
                        + "\(store.fileURL?.lastPathComponent ?? "the file") yet.")
                }
            }
        }
    }
}

/// Add/Remove/Edit, shared by every entity pane. The pane owns the actions
/// because each entity family has its own store methods.
struct EntityToolbar: ToolbarContent {
    let noun: String
    let canEdit: Bool
    let onAdd: () -> Void
    let onEdit: () -> Void
    let onRemove: () -> Void

    var body: some ToolbarContent {
        ToolbarItemGroup {
            Button(action: onAdd) {
                Label("Add \(noun)", systemImage: "plus")
            }
            .help("Add a \(noun.lowercased())")

            Button(action: onEdit) {
                Label("Edit", systemImage: "pencil")
            }
            .help("Edit the selected \(noun.lowercased())")
            .disabled(!canEdit)

            Button(role: .destructive, action: onRemove) {
                Label("Remove", systemImage: "minus")
            }
            .help("Remove the selected \(noun.lowercased())")
            .disabled(!canEdit)
        }
    }
}

/// Marks something as carrying staged changes the file doesn't have yet: a
/// table row, or the sidebar section that row lives in.
struct UnsavedDot: View {
    var body: some View {
        Image(systemName: "circle.fill")
            .font(.system(size: 6))
            .foregroundStyle(.orange)
            .help("Unsaved changes")
            .accessibilityLabel("Unsaved changes")
    }
}

/// Shown when a table has no rows at all, or when a filter matches nothing.
struct TableEmptyState: View {
    let noun: String
    let query: String
    /// True when filters beyond the text query are narrowing the list.
    var filtered = false

    var body: some View {
        VStack(spacing: Spacing.inline) {
            if query.isEmpty && !filtered {
                Text("No \(noun) yet")
                    .font(.title3.weight(.medium))
                Text("Use the + button to add one.")
                    .foregroundStyle(.secondary)
            } else if query.isEmpty {
                Text("No \(noun) match the current filters")
                    .font(.title3.weight(.medium))
            } else {
                Text("No \(noun) match “\(query)”")
                    .font(.title3.weight(.medium))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
    }
}

// MARK: - Channels

struct ChannelsPane: View {
    @EnvironmentObject private var store: CodeplugStore
    @State private var selected = Set<DumpChannel.ID>()
    @State private var editing: DumpChannel?
    @State private var moving: DumpChannel?
    @State private var bulkEditing = false
    @State private var query = ""
    @State private var filterMode: String?
    @State private var filterColorCode: Int?
    @State private var filterSlot: Int?
    @State private var sortOrder = [KeyPathComparator(\DumpChannel.index)]

    var body: some View {
        VStack(spacing: 0) {
            filterBar
            Divider()
            table
        }
        .searchable(text: $query, prompt: "Filter channels")
        .toolbar {
            EntityToolbar(noun: "Channel", canEdit: !selected.isEmpty,
                          onAdd: {
                              // Open the new record for editing right away — a
                              // blank "NEW" channel is never what the user wants
                              // to keep.
                              if let s = store.addChannel() {
                                  selected = [s]
                                  editing = store.channels.first { $0.id == s }
                              }
                          },
                          onEdit: {
                              if selected.count == 1 {
                                  editing = singleSelection
                              } else if selected.count > 1 {
                                  bulkEditing = true
                              }
                          },
                          onRemove: { remove() })
            ToolbarItem {
                Button {
                    bulkEditing = true
                } label: {
                    Label("Bulk Edit", systemImage: "square.and.pencil")
                }
                .help("Edit all selected channels at once")
                .disabled(selected.count < 2)
            }
        }
        .sheet(item: $editing) { channel in
            ChannelEditorSheet(channel: channel, contacts: store.contacts,
                               groupLists: store.groupLists, radioIds: store.radioIds,
                               scanLists: store.scanLists) {
                store.update($0)
            }
        }
        .sheet(item: $moving) { channel in
            MoveSheet(title: "Move Channel \(formatSlot(channel.index))",
                      currentIndex: channel.index) { target in
                store.moveChannel(channel.index, to: target)
                selected = store.channels.contains { $0.id == target } ? [target] : [channel.index]
            }
        }
        .sheet(isPresented: $bulkEditing) {
            BulkChannelEditorSheet(
                channels: selectedChannels,
                contacts: store.contacts,
                groupLists: store.groupLists,
                radioIds: store.radioIds
            ) { update in
                store.bulkUpdateChannels(update)
            }
        }
    }

    /// The channel table itself. Split out of `body` so the filter bar can sit
    /// above it in a stack while the search field, toolbar, and sheets stay on
    /// the enclosing view.
    private var table: some View {
        Table(rows, selection: $selected, sortOrder: $sortOrder) {
            TableColumn("") { row in
                if store.isUnsaved(.channels, slot: row.index) { UnsavedDot() }
            }
            .width(14)
            TableColumn("#", value: \.index) { Text(verbatim: formatSlot($0.index)) }
                .width(min: 40, ideal: 48, max: 64)
            TableColumn("Name", value: \.name) { Text($0.name) }
            TableColumn("RX (MHz)", value: \.rxFrequencyHz) {
                Text(verbatim: formatMHz($0.rxFrequencyHz)).monospacedDigit()
            }
            TableColumn("TX (MHz)", value: \.txFrequencyHz) {
                Text(verbatim: formatMHz($0.txFrequencyHz)).monospacedDigit()
            }
            TableColumn("Mode", value: \.mode) { Text($0.mode) }
                .width(min: 50, ideal: 80)
            TableColumn("Color code", value: \.colorCode) {
                Text(verbatim: String($0.colorCode))
            }
            .width(min: 40, ideal: 56, max: 80)
            TableColumn("Slot", value: \.timeSlot) { Text(verbatim: String($0.timeSlot)) }
                .width(min: 34, ideal: 44, max: 64)
        }
        .contextMenu(forSelectionType: DumpChannel.ID.self) { ids in
            rowMenu(ids)
        } primaryAction: { ids in
            if ids.count == 1 { editing = record(ids.first) }
        }
        .overlay {
            if rows.isEmpty {
                TableEmptyState(noun: "channels", query: query, filtered: hasActiveFilters)
            }
        }
    }

    /// Mode / color-code / slot filters, plus a Clear button once any is set.
    private var filterBar: some View {
        HStack(spacing: Spacing.stack) {
            Picker("Mode", selection: $filterMode) {
                Text("All modes").tag(String?.none)
                ForEach(channelModeOptions, id: \.tag) { Text($0.label).tag(Optional($0.tag)) }
            }
            .fixedSize()

            Picker("Color code", selection: $filterColorCode) {
                Text("All color codes").tag(Int?.none)
                ForEach(0...15, id: \.self) { Text(verbatim: "CC \($0)").tag(Optional($0)) }
            }
            .fixedSize()

            Picker("Slot", selection: $filterSlot) {
                Text("All slots").tag(Int?.none)
                Text("Slot 1").tag(Optional(1))
                Text("Slot 2").tag(Optional(2))
            }
            .fixedSize()

            if hasActiveFilters {
                Button("Clear") {
                    filterMode = nil
                    filterColorCode = nil
                    filterSlot = nil
                }
                .buttonStyle(.borderless)
            }
            Spacer()
        }
        .padding(.horizontal, Spacing.stack)
        .padding(.vertical, Spacing.inline)
    }

    /// True when any of the mode / color-code / slot filters is narrowing the list.
    private var hasActiveFilters: Bool {
        filterMode != nil || filterColorCode != nil || filterSlot != nil
    }

    private var rows: [DumpChannel] {
        store.channels
            .filter {
                channelMatchesFilter($0, query: query, mode: filterMode,
                                     colorCode: filterColorCode, slot: filterSlot)
            }
            .sorted(using: sortOrder)
    }

    /// The channels currently selected, for the bulk editor.
    private var selectedChannels: [DumpChannel] {
        store.channels.filter { selected.contains($0.id) }
    }

    /// Convenience for the single-edit path: the one selected record, or nil.
    private var singleSelection: DumpChannel? {
        selected.count == 1 ? record(selected.first) : nil
    }

    private func record(_ id: DumpChannel.ID?) -> DumpChannel? {
        guard let id else { return nil }
        return store.channels.first { $0.id == id }
    }

    @ViewBuilder
    private func rowMenu(_ ids: Set<DumpChannel.ID>) -> some View {
        if ids.count == 1, let channel = record(ids.first) {
            Button("Edit…") { editing = channel }
            Button("Move to Slot…") { moving = channel }
            Divider()
            Button("Remove", role: .destructive) {
                selected = []
                store.removeChannel(channel.index)
            }
        } else if ids.count > 1 {
            Button("Bulk Edit \(ids.count) Channels…") { bulkEditing = true }
            Divider()
            Button("Remove \(ids.count) Channels", role: .destructive) {
                let indices = ids.sorted()
                selected = []
                for i in indices.reversed() { store.removeChannel(i) }
            }
        }
    }

    private func remove() {
        let indices = selected.sorted()
        selected = []
        for i in indices.reversed() { store.removeChannel(i) }
    }
}

// MARK: - Zones

struct ZonesPane: View {
    @EnvironmentObject private var store: CodeplugStore
    @State private var selected: DumpZone.ID?
    @State private var editing: DumpZone?
    @State private var moving: DumpZone?
    @State private var query = ""
    @State private var sortOrder = [KeyPathComparator(\DumpZone.index)]

    var body: some View {
        Table(rows, selection: $selected, sortOrder: $sortOrder) {
            TableColumn("") { row in
                if store.isUnsaved(.zones, slot: row.index) { UnsavedDot() }
            }
            .width(14)
            TableColumn("#", value: \.index) { Text(verbatim: formatSlot($0.index)) }
                .width(min: 40, ideal: 48, max: 64)
            TableColumn("Name", value: \.name) { Text($0.name) }
            TableColumn("Channels", value: \.channels.count) {
                Text(verbatim: String($0.channels.count))
            }
            .width(min: 60, ideal: 80)
        }
        .contextMenu(forSelectionType: DumpZone.ID.self) { ids in
            rowMenu(ids)
        } primaryAction: { ids in
            editing = record(ids.first)
        }
        .overlay { if rows.isEmpty { TableEmptyState(noun: "zones", query: query) } }
        .searchable(text: $query, prompt: "Filter zones")
        .toolbar {
            EntityToolbar(noun: "Zone", canEdit: selection != nil,
                          onAdd: {
                              if let s = store.addZone() {
                                  selected = s
                                  editing = record(s)
                              }
                          },
                          onEdit: { editing = selection },
                          onRemove: { remove() })
        }
        .sheet(item: $editing) { zone in
            ZoneEditorSheet(zone: zone, channels: store.channels) { store.update($0) }
        }
        .sheet(item: $moving) { zone in
            MoveSheet(title: "Move Zone \(formatSlot(zone.index))",
                      currentIndex: zone.index) { target in
                store.moveZone(zone.index, to: target)
                selected = store.zones.contains { $0.id == target } ? target : zone.index
            }
        }
    }

    private var rows: [DumpZone] {
        store.zones
            .filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
            .sorted(using: sortOrder)
    }

    private var selection: DumpZone? { record(selected) }

    private func record(_ id: DumpZone.ID?) -> DumpZone? {
        guard let id else { return nil }
        return store.zones.first { $0.id == id }
    }

    @ViewBuilder
    private func rowMenu(_ ids: Set<DumpZone.ID>) -> some View {
        if let zone = record(ids.first) {
            Button("Edit…") { editing = zone }
            Button("Move to Slot…") { moving = zone }
            Divider()
            Button("Remove", role: .destructive) {
                selected = nil
                store.removeZone(zone.index)
            }
        }
    }

    private func remove() {
        guard let zone = selection else { return }
        selected = nil
        store.removeZone(zone.index)
    }
}

// MARK: - Contacts / Talk Groups

struct ContactsPane: View {
    @EnvironmentObject private var store: CodeplugStore
    @State private var selected: DumpContact.ID?
    @State private var editing: DumpContact?
    @State private var moving: DumpContact?
    @State private var query = ""
    @State private var sortOrder = [KeyPathComparator(\DumpContact.index)]

    var body: some View {
        Table(rows, selection: $selected, sortOrder: $sortOrder) {
            TableColumn("") { row in
                if store.isUnsaved(.contacts, slot: row.index) { UnsavedDot() }
            }
            .width(14)
            TableColumn("#", value: \.index) { Text(verbatim: formatSlot($0.index)) }
                .width(min: 40, ideal: 48, max: 64)
            TableColumn("Name", value: \.name) { Text($0.name) }
            TableColumn("DMR ID", value: \.number) {
                Text(verbatim: formatDMRID($0.number)).monospacedDigit()
            }
            TableColumn("Type", value: \.callType) { Text($0.callType) }
                .width(min: 60, ideal: 80)
        }
        .contextMenu(forSelectionType: DumpContact.ID.self) { ids in
            rowMenu(ids)
        } primaryAction: { ids in
            editing = record(ids.first)
        }
        .overlay { if rows.isEmpty { TableEmptyState(noun: "contacts", query: query) } }
        .searchable(text: $query, prompt: "Filter contacts")
        .toolbar {
            EntityToolbar(noun: "Contact", canEdit: selection != nil,
                          onAdd: {
                              if let s = store.addContact() {
                                  selected = s
                                  editing = record(s)
                              }
                          },
                          onEdit: { editing = selection },
                          onRemove: { remove() })
        }
        .sheet(item: $editing) { contact in
            ContactEditorSheet(contact: contact) { store.update($0) }
        }
        .sheet(item: $moving) { contact in
            MoveSheet(title: "Move Contact \(formatSlot(contact.index))",
                      currentIndex: contact.index) { target in
                store.moveContact(contact.index, to: target)
                selected = store.contacts.contains { $0.id == target } ? target : contact.index
            }
        }
    }

    private var rows: [DumpContact] {
        store.contacts
            .filter {
                query.isEmpty
                    || $0.name.localizedCaseInsensitiveContains(query)
                    || formatDMRID($0.number).contains(query)
            }
            .sorted(using: sortOrder)
    }

    private var selection: DumpContact? { record(selected) }

    private func record(_ id: DumpContact.ID?) -> DumpContact? {
        guard let id else { return nil }
        return store.contacts.first { $0.id == id }
    }

    @ViewBuilder
    private func rowMenu(_ ids: Set<DumpContact.ID>) -> some View {
        if let contact = record(ids.first) {
            Button("Edit…") { editing = contact }
            Button("Move to Slot…") { moving = contact }
            Divider()
            Button("Remove", role: .destructive) {
                selected = nil
                store.removeContact(contact.index)
            }
        }
    }

    private func remove() {
        guard let contact = selection else { return }
        selected = nil
        store.removeContact(contact.index)
    }
}

// MARK: - Group lists

struct GroupListsPane: View {
    @EnvironmentObject private var store: CodeplugStore
    @State private var selected: DumpGroupList.ID?
    @State private var editing: DumpGroupList?
    @State private var moving: DumpGroupList?
    @State private var query = ""
    @State private var sortOrder = [KeyPathComparator(\DumpGroupList.index)]

    var body: some View {
        Table(rows, selection: $selected, sortOrder: $sortOrder) {
            TableColumn("") { row in
                if store.isUnsaved(.groupLists, slot: row.index) { UnsavedDot() }
            }
            .width(14)
            TableColumn("#", value: \.index) { Text(verbatim: formatSlot($0.index)) }
                .width(min: 40, ideal: 48, max: 64)
            TableColumn("Name", value: \.name) { Text($0.name) }
            TableColumn("Contacts", value: \.members.count) {
                Text(verbatim: String($0.members.count))
            }
            .width(min: 60, ideal: 80)
        }
        .contextMenu(forSelectionType: DumpGroupList.ID.self) { ids in
            rowMenu(ids)
        } primaryAction: { ids in
            editing = record(ids.first)
        }
        .overlay { if rows.isEmpty { TableEmptyState(noun: "group lists", query: query) } }
        .searchable(text: $query, prompt: "Filter group lists")
        .toolbar {
            EntityToolbar(noun: "Group List", canEdit: selection != nil,
                          onAdd: {
                              if let s = store.addGroupList() {
                                  selected = s
                                  editing = record(s)
                              }
                          },
                          onEdit: { editing = selection },
                          onRemove: { remove() })
        }
        .sheet(item: $editing) { groupList in
            GroupListEditorSheet(groupList: groupList, contacts: store.contacts) { store.update($0) }
        }
        .sheet(item: $moving) { groupList in
            MoveSheet(title: "Move Group List \(formatSlot(groupList.index))",
                      currentIndex: groupList.index) { target in
                store.moveGroupList(groupList.index, to: target)
                selected = store.groupLists.contains { $0.id == target } ? target : groupList.index
            }
        }
    }

    private var rows: [DumpGroupList] {
        store.groupLists
            .filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
            .sorted(using: sortOrder)
    }

    private var selection: DumpGroupList? { record(selected) }

    private func record(_ id: DumpGroupList.ID?) -> DumpGroupList? {
        guard let id else { return nil }
        return store.groupLists.first { $0.id == id }
    }

    @ViewBuilder
    private func rowMenu(_ ids: Set<DumpGroupList.ID>) -> some View {
        if let groupList = record(ids.first) {
            Button("Edit…") { editing = groupList }
            Button("Move to Slot…") { moving = groupList }
            Divider()
            Button("Remove", role: .destructive) {
                selected = nil
                store.removeGroupList(groupList.index)
            }
        }
    }

    private func remove() {
        guard let groupList = selection else { return }
        selected = nil
        store.removeGroupList(groupList.index)
    }
}

// MARK: - Scan Lists

struct ScanListsPane: View {
    @EnvironmentObject private var store: CodeplugStore
    @State private var selected: DumpScanList.ID?
    @State private var editing: DumpScanList?
    @State private var moving: DumpScanList?
    @State private var query = ""
    @State private var sortOrder = [KeyPathComparator(\DumpScanList.index)]

    var body: some View {
        Table(rows, selection: $selected, sortOrder: $sortOrder) {
            TableColumn("") { row in
                if store.isUnsaved(.scanLists, slot: row.index) { UnsavedDot() }
            }
            .width(14)
            TableColumn("#", value: \.index) { Text(verbatim: formatSlot($0.index)) }
                .width(min: 40, ideal: 48, max: 64)
            TableColumn("Name", value: \.name) { Text($0.name) }
            TableColumn("Channels", value: \.members.count) {
                Text(verbatim: String($0.members.count))
            }
            .width(min: 60, ideal: 80)
        }
        .contextMenu(forSelectionType: DumpScanList.ID.self) { ids in
            rowMenu(ids)
        } primaryAction: { ids in
            editing = record(ids.first)
        }
        .overlay { if rows.isEmpty { TableEmptyState(noun: "scan lists", query: query) } }
        .searchable(text: $query, prompt: "Filter scan lists")
        .toolbar {
            EntityToolbar(noun: "Scan List", canEdit: selection != nil,
                          onAdd: {
                              if let s = store.addScanList() {
                                  selected = s
                                  editing = record(s)
                              }
                          },
                          onEdit: { editing = selection },
                          onRemove: { remove() })
        }
        .sheet(item: $editing) { scanList in
            ScanListEditorSheet(scanList: scanList, channels: store.channels) { store.update($0) }
        }
        .sheet(item: $moving) { scanList in
            MoveSheet(title: "Move Scan List \(formatSlot(scanList.index))",
                      currentIndex: scanList.index) { target in
                store.moveScanList(scanList.index, to: target)
                selected = store.scanLists.contains { $0.id == target } ? target : scanList.index
            }
        }
    }

    private var rows: [DumpScanList] {
        store.scanLists
            .filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
            .sorted(using: sortOrder)
    }

    private var selection: DumpScanList? { record(selected) }

    private func record(_ id: DumpScanList.ID?) -> DumpScanList? {
        guard let id else { return nil }
        return store.scanLists.first { $0.id == id }
    }

    @ViewBuilder
    private func rowMenu(_ ids: Set<DumpScanList.ID>) -> some View {
        if let scanList = record(ids.first) {
            Button("Edit…") { editing = scanList }
            Button("Move to Slot…") { moving = scanList }
            Divider()
            Button("Remove", role: .destructive) {
                selected = nil
                store.removeScanList(scanList.index)
            }
        }
    }

    private func remove() {
        guard let scanList = selection else { return }
        selected = nil
        store.removeScanList(scanList.index)
    }
}

// MARK: - APRS (read-only)

/// Read-only view of the radio's APRS identity settings. This block lives in the
/// radio-settings region the tool must never write, so there is no edit path —
/// the values are shown for reference and captured in backups only.
struct APRSPane: View {
    @EnvironmentObject private var store: CodeplugStore

    var body: some View {
        Group {
            if let a = store.aprs {
                Form {
                    Section("Read-only") {
                        Text("APRS settings live in a protected region this tool does not write. "
                             + "These values are shown for reference only; edit them in the vendor CPS.")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    Section("Identity") {
                        LabeledContent("My Call Sign", value: a.sourceCall.isEmpty ? "—" : a.sourceCall)
                        LabeledContent("My SSID", value: String(a.sourceSsid))
                        LabeledContent("Destination", value: a.destinationCall.isEmpty ? "—" : a.destinationCall)
                        LabeledContent("Destination SSID", value: String(a.destinationSsid))
                        LabeledContent("Symbol Table", value: symbolString(a.symbolTable))
                        LabeledContent("Map Icon", value: symbolString(a.symbol))
                    }
                    Section("Analog TX") {
                        LabeledContent("TX Frequency",
                                       value: a.fmTxFrequencyHz == 0 ? "—" : "\(formatMHz(a.fmTxFrequencyHz)) MHz")
                        LabeledContent("TX Power (raw)", value: String(a.fmPower))
                    }
                    Section("Timing (raw values)") {
                        LabeledContent("Manual TX Interval", value: String(a.manualTxInterval))
                        LabeledContent("Auto TX Interval", value: String(a.autoTxInterval))
                    }
                }
                .formStyle(.grouped)
            } else {
                VStack(spacing: Spacing.inline) {
                    Image(systemName: "location.slash")
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                    Text("No APRS settings")
                        .font(.headline)
                    Text("This codeplug has no APRS block.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    /// Render a symbol-selector byte as its ASCII character when printable.
    private func symbolString(_ v: Int) -> String {
        if let scalar = Unicode.Scalar(v), v >= 0x20, v < 0x7f {
            return "\(Character(scalar)) (\(v))"
        }
        return String(v)
    }
}

// MARK: - Radio IDs

struct RadioIDsPane: View {
    @EnvironmentObject private var store: CodeplugStore
    @State private var selected: DumpRadioID.ID?
    @State private var editing: DumpRadioID?
    @State private var moving: DumpRadioID?
    @State private var query = ""
    @State private var sortOrder = [KeyPathComparator(\DumpRadioID.index)]

    var body: some View {
        Table(rows, selection: $selected, sortOrder: $sortOrder) {
            TableColumn("") { row in
                if store.isUnsaved(.radioIds, slot: row.index) { UnsavedDot() }
            }
            .width(14)
            TableColumn("#", value: \.index) { Text(verbatim: formatSlot($0.index)) }
                .width(min: 40, ideal: 48, max: 64)
            TableColumn("Name", value: \.name) { Text($0.name) }
            TableColumn("DMR ID", value: \.number) {
                Text(verbatim: formatDMRID($0.number)).monospacedDigit()
            }
        }
        .contextMenu(forSelectionType: DumpRadioID.ID.self) { ids in
            rowMenu(ids)
        } primaryAction: { ids in
            editing = record(ids.first)
        }
        .overlay { if rows.isEmpty { TableEmptyState(noun: "radio IDs", query: query) } }
        .searchable(text: $query, prompt: "Filter radio IDs")
        .toolbar {
            EntityToolbar(noun: "Radio ID", canEdit: selection != nil,
                          onAdd: {
                              if let s = store.addRadioId() {
                                  selected = s
                                  editing = record(s)
                              }
                          },
                          onEdit: { editing = selection },
                          onRemove: { remove() })
        }
        .sheet(item: $editing) { radioId in
            RadioIDEditorSheet(radioId: radioId) { store.update($0) }
        }
        .sheet(item: $moving) { radioId in
            MoveSheet(title: "Move Radio ID \(formatSlot(radioId.index))",
                      currentIndex: radioId.index) { target in
                store.moveRadioId(radioId.index, to: target)
                selected = store.radioIds.contains { $0.id == target } ? target : radioId.index
            }
        }
    }

    private var rows: [DumpRadioID] {
        store.radioIds
            .filter {
                query.isEmpty
                    || $0.name.localizedCaseInsensitiveContains(query)
                    || formatDMRID($0.number).contains(query)
            }
            .sorted(using: sortOrder)
    }

    private var selection: DumpRadioID? { record(selected) }

    private func record(_ id: DumpRadioID.ID?) -> DumpRadioID? {
        guard let id else { return nil }
        return store.radioIds.first { $0.id == id }
    }

    @ViewBuilder
    private func rowMenu(_ ids: Set<DumpRadioID.ID>) -> some View {
        if let radioId = record(ids.first) {
            Button("Edit…") { editing = radioId }
            Button("Move to Slot…") { moving = radioId }
            Divider()
            Button("Remove", role: .destructive) {
                selected = nil
                store.removeRadioId(radioId.index)
            }
        }
    }

    private func remove() {
        guard let radioId = selection else { return }
        selected = nil
        store.removeRadioId(radioId.index)
    }
}
