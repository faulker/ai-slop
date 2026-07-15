import SwiftUI

/// Shared chrome for every record editor: a title, the fields, and Cancel/Done.
///
/// Cancel carries `.cancelAction` so Escape closes the sheet, and Done carries
/// `.defaultAction` so Return commits — the two things the old inline editors
/// had no way to do.
struct EditorSheet<Content: View>: View {
    let title: String
    let onCancel: () -> Void
    let onDone: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(.headline)
                .padding(.horizontal, Spacing.section)
                .padding(.top, Spacing.section)
                .padding(.bottom, Spacing.stack)

            content
                .padding(.horizontal, Spacing.section)

            Divider()
                .padding(.top, Spacing.stack)

            HStack(spacing: Spacing.stack) {
                Spacer()
                Button("Cancel", role: .cancel, action: onCancel)
                    .keyboardShortcut(.cancelAction)
                Button("Done", action: onDone)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
            }
            .padding(.horizontal, Spacing.section)
            .padding(.vertical, Spacing.stack)
        }
        .frame(width: 520)
    }
}

// MARK: - Channel

/// Editor for one channel: name, RX/TX, mode/power/bandwidth, color code, time
/// slot, and the DMR contact / group-list / radio-ID references.
struct ChannelEditorSheet: View {
    /// Referenced entities, so the DMR fields are name pickers instead of raw
    /// index boxes.
    let contacts: [DumpContact]
    let groupLists: [DumpGroupList]
    let radioIds: [DumpRadioID]
    let onCommit: (DumpChannel) -> Void

    @State private var draft: DumpChannel
    @Environment(\.dismiss) private var dismiss

    /// `0xff` in the group-list index means "no RX group list".
    private static let groupListNone = 255

    init(channel: DumpChannel, contacts: [DumpContact], groupLists: [DumpGroupList],
         radioIds: [DumpRadioID], onCommit: @escaping (DumpChannel) -> Void) {
        self.contacts = contacts
        self.groupLists = groupLists
        self.radioIds = radioIds
        self.onCommit = onCommit
        _draft = State(initialValue: channel)
    }

    var body: some View {
        EditorSheet(title: "Channel \(formatSlot(draft.index))",
                    onCancel: { dismiss() },
                    onDone: { onCommit(draft); dismiss() }) {
            Form {
                TextField("Name", text: $draft.name)
                TextField("Receive (MHz)", value: mhzBinding($draft.rxFrequencyHz), format: mhzFormat)
                TextField("Transmit (MHz)", value: mhzBinding($draft.txFrequencyHz), format: mhzFormat)

                Picker("Mode", selection: $draft.mode) {
                    ForEach(channelModeOptions, id: \.tag) { Text($0.label).tag($0.tag) }
                }
                Picker("Power", selection: $draft.power) {
                    ForEach(channelPowerOptions, id: \.tag) { Text($0.label).tag($0.tag) }
                }
                Picker("Bandwidth", selection: $draft.bandwidth) {
                    ForEach(channelBandwidthOptions, id: \.tag) { Text($0.label).tag($0.tag) }
                }

                Stepper("Color code: \(String(draft.colorCode))",
                        value: $draft.colorCode, in: 0...15)
                Picker("Time slot", selection: $draft.timeSlot) {
                    Text("1").tag(1)
                    Text("2").tag(2)
                }
                .pickerStyle(.segmented)

                Picker("Contact", selection: $draft.contactIndex) {
                    // A channel can reference a contact that was removed; without
                    // a matching tag the picker renders blank and can write back a
                    // wrong value on interaction.
                    if !contacts.contains(where: { UInt32($0.index) == draft.contactIndex }) {
                        Text("#\(formatSlot(Int(draft.contactIndex))) (missing)")
                            .tag(draft.contactIndex)
                    }
                    ForEach(contacts) { c in
                        Text(verbatim: "\(c.name) (\(formatDMRID(c.number)))").tag(UInt32(c.index))
                    }
                }
                .disabled(contacts.isEmpty)

                Picker("RX group list", selection: $draft.groupListIndex) {
                    Text("None").tag(Self.groupListNone)
                    if draft.groupListIndex != Self.groupListNone
                        && !groupLists.contains(where: { $0.index == draft.groupListIndex }) {
                        Text("#\(formatSlot(draft.groupListIndex)) (missing)").tag(draft.groupListIndex)
                    }
                    ForEach(groupLists) { g in Text(g.name).tag(g.index) }
                }

                Picker("Radio ID", selection: $draft.radioIdIndex) {
                    if !radioIds.contains(where: { $0.index == draft.radioIdIndex }) {
                        Text("#\(formatSlot(draft.radioIdIndex)) (missing)").tag(draft.radioIdIndex)
                    }
                    ForEach(radioIds) { r in
                        Text(verbatim: "\(r.name) (\(formatDMRID(r.number)))").tag(r.index)
                    }
                }
                .disabled(radioIds.isEmpty)
            }
            .formStyle(.grouped)
        }
    }

    /// Bridge a Hz field to an editable MHz value, rounded to the codeplug's
    /// 10 Hz BCD resolution.
    private func mhzBinding(_ hz: Binding<UInt32>) -> Binding<Double> {
        Binding(
            get: { Double(hz.wrappedValue) / 1_000_000 },
            set: { mhz in
                let clamped = min(max(mhz, 0), 999.999_99)
                hz.wrappedValue = UInt32((clamped * 100_000).rounded()) * 10
            }
        )
    }
}

// MARK: - Zone

/// Editor for one zone: its name and its channel membership.
struct ZoneEditorSheet: View {
    let channels: [DumpChannel]
    let onCommit: (DumpZone) -> Void

    @State private var draft: DumpZone
    @Environment(\.dismiss) private var dismiss

    init(zone: DumpZone, channels: [DumpChannel], onCommit: @escaping (DumpZone) -> Void) {
        self.channels = channels
        self.onCommit = onCommit
        _draft = State(initialValue: zone)
    }

    var body: some View {
        EditorSheet(title: "Zone \(formatSlot(draft.index))",
                    onCancel: { dismiss() },
                    onDone: { onCommit(draft); dismiss() }) {
            Form {
                TextField("Name", text: $draft.name)
                MemberPickerField(label: "Channels", addTitle: "Add channel",
                                  candidates: candidates, members: $draft.channels)
            }
            .formStyle(.grouped)
        }
    }

    /// Channels as pickable members, labeled by slot, name, and RX frequency.
    private var candidates: [MemberCandidate] {
        channels.map {
            MemberCandidate(index: $0.index,
                            label: "\(formatSlot($0.index)) — \($0.name) (\(formatMHz($0.rxFrequencyHz)))")
        }
    }
}

// MARK: - Contact

/// Editor for one DMR contact / talk group.
struct ContactEditorSheet: View {
    let onCommit: (DumpContact) -> Void

    @State private var draft: DumpContact
    @Environment(\.dismiss) private var dismiss

    init(contact: DumpContact, onCommit: @escaping (DumpContact) -> Void) {
        self.onCommit = onCommit
        _draft = State(initialValue: contact)
    }

    var body: some View {
        EditorSheet(title: "Contact \(formatSlot(draft.index))",
                    onCancel: { dismiss() },
                    onDone: { onCommit(draft); dismiss() }) {
            Form {
                TextField("Name", text: $draft.name)
                TextField("DMR ID", value: dmrIDBinding($draft.number),
                          format: .number.grouping(.never))
                Picker("Call type", selection: $draft.callType) {
                    ForEach(callTypeOptions, id: \.tag) { Text($0.label).tag($0.tag) }
                }
            }
            .formStyle(.grouped)
        }
    }

    private func dmrIDBinding(_ v: Binding<UInt32>) -> Binding<Int> {
        Binding(get: { Int(v.wrappedValue) }, set: { v.wrappedValue = UInt32(max(0, $0)) })
    }
}

// MARK: - Group list

/// Editor for one RX group list: its name and its contact membership.
struct GroupListEditorSheet: View {
    let contacts: [DumpContact]
    let onCommit: (DumpGroupList) -> Void

    @State private var draft: DumpGroupList
    @Environment(\.dismiss) private var dismiss

    init(groupList: DumpGroupList, contacts: [DumpContact],
         onCommit: @escaping (DumpGroupList) -> Void) {
        self.contacts = contacts
        self.onCommit = onCommit
        _draft = State(initialValue: groupList)
    }

    var body: some View {
        EditorSheet(title: "Group List \(formatSlot(draft.index))",
                    onCancel: { dismiss() },
                    onDone: { onCommit(draft); dismiss() }) {
            Form {
                TextField("Name", text: $draft.name)
                MemberPickerField(label: "Contacts", addTitle: "Add contact",
                                  candidates: candidates, members: $draft.members)
            }
            .formStyle(.grouped)
        }
    }

    /// Contacts as pickable members, labeled by name and DMR ID.
    private var candidates: [MemberCandidate] {
        contacts.map { MemberCandidate(index: $0.index, label: "\($0.name) (\(formatDMRID($0.number)))") }
    }
}

// MARK: - Radio ID

/// Editor for one radio ID.
struct RadioIDEditorSheet: View {
    let onCommit: (DumpRadioID) -> Void

    @State private var draft: DumpRadioID
    @Environment(\.dismiss) private var dismiss

    init(radioId: DumpRadioID, onCommit: @escaping (DumpRadioID) -> Void) {
        self.onCommit = onCommit
        _draft = State(initialValue: radioId)
    }

    var body: some View {
        EditorSheet(title: "Radio ID \(formatSlot(draft.index))",
                    onCancel: { dismiss() },
                    onDone: { onCommit(draft); dismiss() }) {
            Form {
                TextField("Name", text: $draft.name)
                TextField("DMR ID", value: dmrIDBinding($draft.number),
                          format: .number.grouping(.never))
            }
            .formStyle(.grouped)
        }
    }

    private func dmrIDBinding(_ v: Binding<UInt32>) -> Binding<Int> {
        Binding(get: { Int(v.wrappedValue) }, set: { v.wrappedValue = UInt32(max(0, $0)) })
    }
}

// MARK: - Move

/// Relocate a record to a different slot. The core rejects an occupied or
/// out-of-range target, which surfaces as an error.
struct MoveSheet: View {
    let title: String
    let currentIndex: Int
    let onCommit: (Int) -> Void

    @State private var slot: Int
    @Environment(\.dismiss) private var dismiss

    init(title: String, currentIndex: Int, onCommit: @escaping (Int) -> Void) {
        self.title = title
        self.currentIndex = currentIndex
        self.onCommit = onCommit
        _slot = State(initialValue: currentIndex + 1)
    }

    var body: some View {
        EditorSheet(title: title,
                    onCancel: { dismiss() },
                    onDone: {
                        let target = slot - 1
                        if target != currentIndex { onCommit(target) }
                        dismiss()
                    }) {
            Form {
                TextField("Slot number", value: $slot, format: .number.grouping(.never))
                Text("The slot must be free and within the radio's range.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .formStyle(.grouped)
        }
    }
}

// MARK: - Membership

/// One selectable member: its 0-based slot index and a human label. Identified
/// by `index` for `ForEach`.
struct MemberCandidate: Identifiable {
    let index: Int
    let label: String
    var id: Int { index }
}

/// Edits a membership list (zone channels, group-list contacts) by name instead
/// of raw indices. Shows current members as labeled rows with Remove, and an
/// Add menu of the candidates not yet included. The bound `members` stays a list
/// of 0-based indices; the index bookkeeping is hidden from the user.
struct MemberPickerField: View {
    let label: String
    let addTitle: String
    let candidates: [MemberCandidate]
    @Binding var members: [Int]

    var body: some View {
        Section(label) {
            if members.isEmpty {
                Text("No members yet — use “\(addTitle)”.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(Array(members.enumerated()), id: \.offset) { pos, m in
                    HStack {
                        Text(labelFor(m))
                        Spacer()
                        Button {
                            members.remove(at: pos)
                        } label: {
                            Image(systemName: "minus.circle")
                        }
                        .buttonStyle(.borderless)
                        .help("Remove from \(label.lowercased())")
                    }
                }
            }
            Menu(addTitle) {
                ForEach(available) { item in
                    Button(item.label) { members.append(item.index) }
                }
            }
            .disabled(available.isEmpty)
        }
    }

    /// Candidates not already in the membership list.
    private var available: [MemberCandidate] {
        candidates.filter { !members.contains($0.index) }
    }

    /// Display label for a member index (or a fallback if it no longer exists).
    private func labelFor(_ index: Int) -> String {
        candidates.first { $0.index == index }?.label ?? "#\(formatSlot(index)) (missing)"
    }
}
