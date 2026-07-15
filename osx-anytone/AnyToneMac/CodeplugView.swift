import SwiftUI

/// Which entity family the Codeplug editor is currently showing.
enum CodeplugSection: String, CaseIterable, Identifiable {
    case channels = "Channels"
    case zones = "Zones"
    case contacts = "Contacts"
    case groupLists = "Group Lists"
    case radioIds = "Radio IDs"

    var id: String { rawValue }

    var symbol: String {
        switch self {
        case .channels: return "list.bullet"
        case .zones: return "square.grid.2x2"
        case .contacts: return "person.2"
        case .groupLists: return "person.3"
        case .radioIds: return "number"
        }
    }
}

/// Codeplug detail: the pane for one entity family, or the empty state when no
/// file is open.
struct CodeplugView: View {
    let section: CodeplugSection
    @EnvironmentObject private var store: CodeplugStore

    var body: some View {
        Group {
            if store.fileURL == nil {
                noFileState
            } else {
                switch section {
                case .channels: ChannelsPane()
                case .zones: ZonesPane()
                case .contacts: ContactsPane()
                case .groupLists: GroupListsPane()
                case .radioIds: RadioIDsPane()
                }
            }
        }
        .toolbar { CodeplugFileToolbar() }
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
                + "tab with Backup.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 380)
            Button("Open Codeplug…") { store.openWithPanel() }
                .keyboardShortcut("o")
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Open/Save, shared by every codeplug pane.
struct CodeplugFileToolbar: ToolbarContent {
    @EnvironmentObject private var store: CodeplugStore
    @State private var confirmOpen = false

    var body: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            Button {
                // Opening would drop the staged work file, so ask first.
                if store.isDirty { confirmOpen = true } else { store.openWithPanel() }
            } label: {
                Label("Open", systemImage: "folder")
            }
            .help("Open a codeplug .bin")
            .confirmationDialog("Save changes before opening another codeplug?",
                                isPresented: $confirmOpen) {
                Button("Save") { store.save(); store.openWithPanel() }
                Button("Discard", role: .destructive) { store.openWithPanel() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Your staged changes haven't been written to "
                    + "\(store.fileURL?.lastPathComponent ?? "the file") yet.")
            }
        }
        ToolbarItem {
            Button {
                store.save()
            } label: {
                Label("Save", systemImage: "square.and.arrow.down")
            }
            .help("Write staged changes to the file")
            .disabled(!store.isDirty)
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

/// Shown when a table has no rows at all, or when a filter matches nothing.
struct TableEmptyState: View {
    let noun: String
    let query: String

    var body: some View {
        VStack(spacing: Spacing.inline) {
            if query.isEmpty {
                Text("No \(noun) yet")
                    .font(.title3.weight(.medium))
                Text("Use the + button to add one.")
                    .foregroundStyle(.secondary)
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
    @State private var selected: DumpChannel.ID?
    @State private var editing: DumpChannel?
    @State private var moving: DumpChannel?
    @State private var query = ""
    @State private var sortOrder = [KeyPathComparator(\DumpChannel.index)]

    var body: some View {
        Table(rows, selection: $selected, sortOrder: $sortOrder) {
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
            editing = record(ids.first)
        }
        .overlay { if rows.isEmpty { TableEmptyState(noun: "channels", query: query) } }
        .searchable(text: $query, prompt: "Filter channels")
        .toolbar {
            EntityToolbar(noun: "Channel", canEdit: selection != nil,
                          onAdd: { selected = store.addChannel() },
                          onEdit: { editing = selection },
                          onRemove: { remove() })
        }
        .sheet(item: $editing) { channel in
            ChannelEditorSheet(channel: channel, contacts: store.contacts,
                               groupLists: store.groupLists, radioIds: store.radioIds) {
                store.update($0)
            }
        }
        .sheet(item: $moving) { channel in
            MoveSheet(title: "Move Channel \(formatSlot(channel.index))",
                      currentIndex: channel.index) { target in
                store.moveChannel(channel.index, to: target)
                selected = store.channels.contains { $0.id == target } ? target : channel.index
            }
        }
    }

    private var rows: [DumpChannel] {
        store.channels
            .filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
            .sorted(using: sortOrder)
    }

    /// The selected record, looked up by id. Nil once the selection points at a
    /// slot that no longer holds anything.
    private var selection: DumpChannel? { record(selected) }

    private func record(_ id: DumpChannel.ID?) -> DumpChannel? {
        guard let id else { return nil }
        return store.channels.first { $0.id == id }
    }

    @ViewBuilder
    private func rowMenu(_ ids: Set<DumpChannel.ID>) -> some View {
        if let channel = record(ids.first) {
            Button("Edit…") { editing = channel }
            Button("Move to Slot…") { moving = channel }
            Divider()
            Button("Remove", role: .destructive) {
                selected = nil
                store.removeChannel(channel.index)
            }
        }
    }

    private func remove() {
        guard let channel = selection else { return }
        selected = nil
        store.removeChannel(channel.index)
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
                          onAdd: { selected = store.addZone() },
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
                          onAdd: { selected = store.addContact() },
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
                          onAdd: { selected = store.addGroupList() },
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
                          onAdd: { selected = store.addRadioId() },
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
