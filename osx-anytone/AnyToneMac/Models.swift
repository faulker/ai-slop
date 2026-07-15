import Foundation

/// One serial port as reported by the core's port enumeration
/// (`anytone_ports_json`).
struct PortEntry: Codable, Identifiable, Hashable {
    let name: String
    let vid: UInt16?
    let pid: UInt16?
    let product: String?
    let likelyRadio: Bool

    var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, vid, pid, product
        case likelyRadio = "likely_radio"
    }
}

/// One decoded channel from `anytone_dump_json`. Field names/values match the
/// Rust `Channel` serialization (enum fields arrive capitalized, e.g. "Analog").
struct DumpChannel: Codable, Identifiable, Hashable {
    let index: Int
    var name: String
    var rxFrequencyHz: UInt32
    var txFrequencyHz: UInt32
    var mode: String
    var power: String
    var bandwidth: String
    var colorCode: Int
    var timeSlot: Int
    var contactIndex: UInt32
    var radioIdIndex: Int
    var groupListIndex: Int

    var id: Int { index }

    enum CodingKeys: String, CodingKey {
        case index, name, mode, power, bandwidth
        case rxFrequencyHz = "rx_frequency_hz"
        case txFrequencyHz = "tx_frequency_hz"
        case colorCode = "color_code"
        case timeSlot = "time_slot"
        case contactIndex = "contact_index"
        case radioIdIndex = "radio_id_index"
        case groupListIndex = "group_list_index"
    }
}

/// One decoded zone from `anytone_dump_json`.
struct DumpZone: Codable, Identifiable, Hashable {
    let index: Int
    var name: String
    var channels: [Int]

    var id: Int { index }
}

/// One decoded DMR contact / talk group from `anytone_dump_json`.
struct DumpContact: Codable, Identifiable, Hashable {
    let index: Int
    var name: String
    var number: UInt32
    /// "Private" | "Group" | "All" (talk groups are Group calls).
    var callType: String

    var id: Int { index }

    enum CodingKeys: String, CodingKey {
        case index, name, number
        case callType = "call_type"
    }
}

/// One decoded RX group list from `anytone_dump_json`.
struct DumpGroupList: Codable, Identifiable, Hashable {
    let index: Int
    var name: String
    var members: [Int]

    var id: Int { index }
}

/// One decoded radio ID from `anytone_dump_json`.
struct DumpRadioID: Codable, Identifiable, Hashable {
    let index: Int
    var name: String
    var number: UInt32

    var id: Int { index }
}

/// Top-level payload of `anytone_dump_json`.
struct CodeplugDump: Codable {
    let channels: [DumpChannel]
    let zones: [DumpZone]
    let contacts: [DumpContact]
    let groupLists: [DumpGroupList]
    let radioIds: [DumpRadioID]

    enum CodingKeys: String, CodingKey {
        case channels, zones, contacts
        case groupLists = "group_lists"
        case radioIds = "radio_ids"
    }
}

// MARK: - Edit spec (encoded to `anytone_apply_edits`)

/// One channel update or add. `index` is omitted for adds. Nil fields are
/// omitted by the encoder, so only present fields are applied.
struct ChannelEdit: Encodable {
    var index: Int?
    var name: String?
    var rxFrequencyHz: UInt32?
    var txFrequencyHz: UInt32?
    var mode: String?
    var power: String?
    var bandwidth: String?
    var colorCode: Int?
    var timeSlot: Int?
    var contactIndex: UInt32?
    var radioIdIndex: Int?
    var groupListIndex: Int?

    enum CodingKeys: String, CodingKey {
        case index, name, mode, power, bandwidth
        case rxFrequencyHz = "rx_frequency_hz"
        case txFrequencyHz = "tx_frequency_hz"
        case colorCode = "color_code"
        case timeSlot = "time_slot"
        case contactIndex = "contact_index"
        case radioIdIndex = "radio_id_index"
        case groupListIndex = "group_list_index"
    }
}

/// One zone update or add (`index` omitted for adds).
struct ZoneEdit: Encodable {
    var index: Int?
    var name: String?
    var members: [Int]?
}

/// One contact update or add (`index` omitted for adds).
struct ContactEdit: Encodable {
    var index: Int?
    var name: String?
    var number: UInt32?
    var callType: String?

    enum CodingKeys: String, CodingKey {
        case index, name, number
        case callType = "call_type"
    }
}

/// One group-list update or add (`index` omitted for adds).
struct GroupListEdit: Encodable {
    var index: Int?
    var name: String?
    var members: [Int]?
}

/// One radio-ID update or add (`index` omitted for adds).
struct RadioIDEdit: Encodable {
    var index: Int?
    var name: String?
    var number: UInt32?
}

/// Relocate the record at `from` to the free slot `to` (backs the editable "#"
/// column). Encoded into the `move_*` arrays of `EditSpec`.
struct MoveOp: Encodable {
    let from: Int
    let to: Int
}

/// Full batch passed to `anytone_apply_edits`. Empty arrays are fine; the core
/// applies updates, then removals, then additions.
struct EditSpec: Encodable {
    var channels: [ChannelEdit] = []
    var addChannels: [ChannelEdit] = []
    var removeChannels: [Int] = []
    var moveChannels: [MoveOp] = []
    var zones: [ZoneEdit] = []
    var addZones: [ZoneEdit] = []
    var removeZones: [Int] = []
    var moveZones: [MoveOp] = []
    var contacts: [ContactEdit] = []
    var addContacts: [ContactEdit] = []
    var removeContacts: [Int] = []
    var moveContacts: [MoveOp] = []
    var groupLists: [GroupListEdit] = []
    var addGroupLists: [GroupListEdit] = []
    var removeGroupLists: [Int] = []
    var moveGroupLists: [MoveOp] = []
    var radioIds: [RadioIDEdit] = []
    var addRadioIds: [RadioIDEdit] = []
    var removeRadioIds: [Int] = []
    var moveRadioIds: [MoveOp] = []

    /// True when there is nothing to apply.
    var isEmpty: Bool {
        channels.isEmpty && addChannels.isEmpty && removeChannels.isEmpty && moveChannels.isEmpty
            && zones.isEmpty && addZones.isEmpty && removeZones.isEmpty && moveZones.isEmpty
            && contacts.isEmpty && addContacts.isEmpty && removeContacts.isEmpty && moveContacts.isEmpty
            && groupLists.isEmpty && addGroupLists.isEmpty && removeGroupLists.isEmpty
            && moveGroupLists.isEmpty
            && radioIds.isEmpty && addRadioIds.isEmpty && removeRadioIds.isEmpty && moveRadioIds.isEmpty
    }

    enum CodingKeys: String, CodingKey {
        case channels, zones, contacts
        case addChannels = "add_channels"
        case removeChannels = "remove_channels"
        case moveChannels = "move_channels"
        case addZones = "add_zones"
        case removeZones = "remove_zones"
        case moveZones = "move_zones"
        case addContacts = "add_contacts"
        case removeContacts = "remove_contacts"
        case moveContacts = "move_contacts"
        case groupLists = "group_lists"
        case addGroupLists = "add_group_lists"
        case removeGroupLists = "remove_group_lists"
        case moveGroupLists = "move_group_lists"
        case radioIds = "radio_ids"
        case addRadioIds = "add_radio_ids"
        case removeRadioIds = "remove_radio_ids"
        case moveRadioIds = "move_radio_ids"
    }
}

// MARK: - Display / value helpers

/// Picker options for channel mode. `tag` is the capitalized value as it appears
/// in a dump (so it binds directly to `DumpChannel.mode`); `label` is friendly.
let channelModeOptions: [(tag: String, label: String)] = [
    ("Analog", "Analog"), ("Digital", "Digital"),
    ("MixedAnalog", "Mixed (A)"), ("MixedDigital", "Mixed (D)"),
]

/// Picker options for transmit power (dump-valued tags).
let channelPowerOptions: [(tag: String, label: String)] = [
    ("Low", "Low"), ("Mid", "Mid"), ("High", "High"), ("Turbo", "Turbo"),
]

/// Picker options for channel bandwidth (dump-valued tags).
let channelBandwidthOptions: [(tag: String, label: String)] = [
    ("Narrow", "Narrow"), ("Wide", "Wide"),
]

/// Picker options for contact call type (dump-valued tags).
let callTypeOptions: [(tag: String, label: String)] = [
    ("Group", "Group (talk group)"), ("Private", "Private"), ("All", "All"),
]

/// Map a capitalized enum string from a dump ("Analog", "Turbo", "Group") to the
/// lowercase snake_case value the edit API expects ("analog", "turbo", "group").
func editValue(_ dumped: String) -> String {
    switch dumped {
    case "MixedAnalog": return "mixed_analog"
    case "MixedDigital": return "mixed_digital"
    default: return dumped.lowercased()
    }
}

/// Shared MHz display/edit format: fixed 5 decimals (the codeplug's 10 Hz BCD
/// resolution), no grouping, dot decimal separator. Pinned to POSIX because
/// these are hardware values rather than prose numbers, and because the table
/// cell and the editor field must round-trip identically.
let mhzFormat = FloatingPointFormatStyle<Double>.number
    .precision(.fractionLength(5))
    .grouping(.never)
    .locale(Locale(identifier: "en_US_POSIX"))

/// Format a frequency in Hz as a MHz string.
func formatMHz(_ hz: UInt32) -> String {
    (Double(hz) / 1_000_000).formatted(mhzFormat)
}

/// Format a DMR ID as plain digits. DMR IDs are identifiers, never grouped:
/// `Text("\(id)")` would route a UInt32 through LocalizedStringKey and render
/// 3113043 as "3,113,043".
func formatDMRID(_ id: UInt32) -> String {
    String(id)
}

/// Format a 0-based slot as its 1-based display number, ungrouped (slot counts
/// reach the thousands, so interpolation would introduce a separator).
func formatSlot(_ index: Int) -> String {
    String(index + 1)
}

// MARK: - Layout

/// The app's spacing scale. Every gap between views comes from here so the
/// rhythm stays consistent instead of drifting into ad-hoc 4/6/8/10/12 values.
enum Spacing {
    /// Between tightly related items (a label and the control it names).
    static let tight: CGFloat = 4
    /// Between controls within a row or group.
    static let inline: CGFloat = 8
    /// Between distinct rows or stacked groups.
    static let stack: CGFloat = 12
    /// Between major sections, and around window content.
    static let section: CGFloat = 20
}
