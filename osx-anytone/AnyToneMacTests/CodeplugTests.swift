import XCTest
@testable import AnyToneMac

/// Number formatting. These pin two bugs that reached the UI: DMR IDs rendering
/// as "3,113,043" because a UInt32 interpolated into `Text("\(n)")` routes
/// through LocalizedStringKey, and the same frequency rendering as "145.50000"
/// in a table but "145.5" in its editor.
final class FormattingTests: XCTestCase {

    func testDMRIDHasNoGroupingSeparator() {
        XCTAssertEqual(formatDMRID(3_113_043), "3113043")
        XCTAssertEqual(formatDMRID(1), "1")
        XCTAssertEqual(formatDMRID(16_777_215), "16777215")
    }

    func testMHzUsesFiveDecimalsAndNoGrouping() {
        XCTAssertEqual(formatMHz(145_500_000), "145.50000")
        XCTAssertEqual(formatMHz(446_000_000), "446.00000")
    }

    /// A 1.2 GHz channel is where a grouping separator would appear.
    func testMHzDoesNotGroupAboveOneThousand() {
        XCTAssertEqual(formatMHz(1_296_000_000), "1296.00000")
    }

    /// The table and the editor share one format, so what is displayed parses
    /// back to the same value.
    func testMHzRoundTripsThroughItsFormat() throws {
        let text = formatMHz(145_512_500)
        let parsed = try mhzFormat.parseStrategy.parse(text)
        XCTAssertEqual(parsed, 145.5125, accuracy: 0.000_001)
    }

    /// Slots are displayed 1-based and reach the thousands, so they must not
    /// pick up a separator either.
    func testSlotIsOneBasedAndUngrouped() {
        XCTAssertEqual(formatSlot(0), "1")
        XCTAssertEqual(formatSlot(1233), "1234")
    }

    func testEditValueMapsDumpEnumsToEditAPIValues() {
        XCTAssertEqual(editValue("MixedAnalog"), "mixed_analog")
        XCTAssertEqual(editValue("MixedDigital"), "mixed_digital")
        XCTAssertEqual(editValue("Turbo"), "turbo")
        XCTAssertEqual(editValue("Analog"), "analog")
        XCTAssertEqual(editValue("Group"), "group")
    }
}

/// Channel mode semantics and the Channels-pane filter matching.
final class ChannelFilterTests: XCTestCase {

    private func channel(mode: String = "Digital", colorCode: Int = 1,
                         timeSlot: Int = 1, name: String = "Repeater") -> DumpChannel {
        DumpChannel(index: 0, name: name, rxFrequencyHz: 146_520_000,
                    txFrequencyHz: 146_520_000, mode: mode, power: "High",
                    bandwidth: "Wide", colorCode: colorCode, timeSlot: timeSlot,
                    contactIndex: 0, radioIdIndex: 0, groupListIndex: 255)
    }

    /// Only pure "Analog" lacks a digital component; the two mixed modes keep it.
    func testModeHasDigital() {
        XCTAssertFalse(modeHasDigital("Analog"))
        XCTAssertTrue(modeHasDigital("Digital"))
        XCTAssertTrue(modeHasDigital("MixedAnalog"))
        XCTAssertTrue(modeHasDigital("MixedDigital"))
    }

    func testNilFiltersMatchEverything() {
        let ch = channel(mode: "Analog", colorCode: 7, timeSlot: 2)
        XCTAssertTrue(channelMatchesFilter(ch, query: "", mode: nil, colorCode: nil, slot: nil))
    }

    func testEachFilterComponentNarrows() {
        let ch = channel(mode: "Digital", colorCode: 3, timeSlot: 2)
        XCTAssertTrue(channelMatchesFilter(ch, query: "", mode: "Digital", colorCode: nil, slot: nil))
        XCTAssertFalse(channelMatchesFilter(ch, query: "", mode: "Analog", colorCode: nil, slot: nil))
        XCTAssertTrue(channelMatchesFilter(ch, query: "", mode: nil, colorCode: 3, slot: nil))
        XCTAssertFalse(channelMatchesFilter(ch, query: "", mode: nil, colorCode: 4, slot: nil))
        XCTAssertTrue(channelMatchesFilter(ch, query: "", mode: nil, colorCode: nil, slot: 2))
        XCTAssertFalse(channelMatchesFilter(ch, query: "", mode: nil, colorCode: nil, slot: 1))
    }

    /// The text query and the dropdown filters combine with AND.
    func testQueryAndFiltersCombine() {
        let ch = channel(mode: "Digital", colorCode: 1, timeSlot: 1, name: "Local Repeater")
        XCTAssertTrue(channelMatchesFilter(ch, query: "repeater", mode: "Digital",
                                           colorCode: 1, slot: 1))
        XCTAssertFalse(channelMatchesFilter(ch, query: "simplex", mode: "Digital",
                                            colorCode: 1, slot: 1))
    }
}

/// `EditSpec` encoding. This is the highest-consequence pure logic in the Swift
/// layer: the core matches on snake_case keys, so one wrong key silently drops
/// an edit to a radio's configuration with no error anywhere.
final class EditSpecTests: XCTestCase {

    private func encode(_ spec: EditSpec) throws -> [String: Any] {
        let data = try JSONEncoder().encode(spec)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// Absent fields must be omitted, not sent as null: the core leaves out
    /// fields alone, so an unintended key would overwrite a value the user
    /// never touched.
    func testChannelEditOmitsUntouchedFields() throws {
        var edit = ChannelEdit(index: 4)
        edit.name = "Simplex"
        let json = try encode(EditSpec(channels: [edit]))

        let channels = try XCTUnwrap(json["channels"] as? [[String: Any]])
        XCTAssertEqual(channels.count, 1)
        XCTAssertEqual(channels[0]["index"] as? Int, 4)
        XCTAssertEqual(channels[0]["name"] as? String, "Simplex")
        XCTAssertEqual(channels[0].count, 2, "only the touched fields should be encoded")
    }

    func testChannelEditUsesSnakeCaseKeys() throws {
        var edit = ChannelEdit(index: 0)
        edit.rxFrequencyHz = 145_500_000
        edit.txFrequencyHz = 145_500_000
        edit.colorCode = 1
        edit.timeSlot = 2
        edit.contactIndex = 3
        edit.radioIdIndex = 4
        edit.groupListIndex = 5
        let channel = try XCTUnwrap(try encode(EditSpec(channels: [edit]))["channels"]
            as? [[String: Any]])[0]

        XCTAssertEqual(channel["rx_frequency_hz"] as? UInt32, 145_500_000)
        XCTAssertEqual(channel["tx_frequency_hz"] as? UInt32, 145_500_000)
        XCTAssertEqual(channel["color_code"] as? Int, 1)
        XCTAssertEqual(channel["time_slot"] as? Int, 2)
        XCTAssertEqual(channel["contact_index"] as? Int, 3)
        XCTAssertEqual(channel["radio_id_index"] as? Int, 4)
        XCTAssertEqual(channel["group_list_index"] as? Int, 5)
    }

    func testStructuralArraysUseSnakeCaseKeys() throws {
        var spec = EditSpec()
        spec.addZones = [ZoneEdit()]
        spec.removeZones = [2]
        spec.moveZones = [MoveOp(from: 1, to: 7)]
        spec.addRadioIds = [RadioIDEdit()]
        spec.removeGroupLists = [3]
        let json = try encode(spec)

        XCTAssertNotNil(json["add_zones"])
        XCTAssertEqual(json["remove_zones"] as? [Int], [2])
        XCTAssertNotNil(json["add_radio_ids"])
        XCTAssertEqual(json["remove_group_lists"] as? [Int], [3])

        let moves = try XCTUnwrap(json["move_zones"] as? [[String: Any]])
        XCTAssertEqual(moves[0]["from"] as? Int, 1)
        XCTAssertEqual(moves[0]["to"] as? Int, 7)
    }

    func testContactEditUsesCallTypeKey() throws {
        var edit = ContactEdit(index: 1)
        edit.callType = "group"
        let contact = try XCTUnwrap(try encode(EditSpec(contacts: [edit]))["contacts"]
            as? [[String: Any]])[0]
        XCTAssertEqual(contact["call_type"] as? String, "group")
    }

    func testEmptySpecIsEmpty() {
        XCTAssertTrue(EditSpec().isEmpty)
    }

    func testSpecWithAnyOperationIsNotEmpty() {
        var spec = EditSpec()
        spec.moveRadioIds = [MoveOp(from: 0, to: 1)]
        XCTAssertFalse(spec.isEmpty, "isEmpty must account for every operation array")
    }
}

/// Dump decoding. The core emits snake_case and capitalized enum values; a
/// mismatch here means the app silently shows an empty codeplug.
final class CodeplugDumpTests: XCTestCase {

    func testDecodesCoreJSON() throws {
        let json = """
        {
          "channels": [{
            "index": 0, "name": "Simplex", "rx_frequency_hz": 146520000,
            "tx_frequency_hz": 146520000, "mode": "Analog", "power": "High",
            "bandwidth": "Wide", "color_code": 1, "time_slot": 1,
            "contact_index": 0, "radio_id_index": 0, "group_list_index": 255,
            "rx_signaling_mode": "Ctcss", "tx_signaling_mode": "None",
            "rx_ctcss": 12, "tx_ctcss": 0, "rx_dcs": 0, "tx_dcs": 0,
            "squelch_mode": "Carrier", "optional_signaling": "Off", "admit": "Always",
            "scan_list_index": 255, "dtmf_id_index": 0, "two_tone_id_index": 0,
            "five_tone_id_index": 0, "two_tone_decode_index": 0,
            "rx_only": true, "talk_around": false, "call_confirm": false,
            "work_alone": false, "simplex_tdma": false, "rx_aprs": false
          }],
          "zones": [{"index": 0, "name": "Local", "channels": [0]}],
          "scan_lists": [{
            "index": 0, "name": "Personal", "members": [0], "priority_channel_select": 3,
            "priority_channel_1": 0, "priority_channel_2": 65535, "look_back_a": 20,
            "look_back_b": 30, "dropout_delay": 31, "dwell_time": 31, "revert_channel": 2
          }],
          "contacts": [{"index": 0, "name": "Parrot", "number": 9990, "call_type": "Private"}],
          "group_lists": [{"index": 0, "name": "Wide", "members": [0]}],
          "radio_ids": [{"index": 0, "name": "Me", "number": 3113043}],
          "aprs": {
            "source_call": "K7AOS", "source_ssid": 7, "destination_call": "APAT81",
            "destination_ssid": 0, "symbol_table": 47, "symbol": 60,
            "manual_tx_interval": 5, "auto_tx_interval": 45, "fm_power": 3,
            "fm_tx_frequency_hz": 144390000
          }
        }
        """
        let dump = try JSONDecoder().decode(CodeplugDump.self, from: Data(json.utf8))

        XCTAssertEqual(dump.channels.first?.rxFrequencyHz, 146_520_000)
        XCTAssertEqual(dump.channels.first?.groupListIndex, 255)
        XCTAssertEqual(dump.channels.first?.rxSignalingMode, "Ctcss")
        XCTAssertEqual(dump.channels.first?.rxCtcss, 12)
        XCTAssertEqual(dump.channels.first?.rxOnly, true)
        XCTAssertEqual(dump.channels.first?.admit, "Always")
        XCTAssertEqual(dump.scanLists.first?.name, "Personal")
        XCTAssertEqual(dump.scanLists.first?.members, [0])
        XCTAssertEqual(dump.scanLists.first?.priorityChannel2, 65535)
        XCTAssertEqual(dump.contacts.first?.callType, "Private")
        XCTAssertEqual(dump.groupLists.first?.members, [0])
        XCTAssertEqual(dump.radioIds.first?.number, 3_113_043)
        XCTAssertEqual(dump.aprs?.sourceCall, "K7AOS")
        XCTAssertEqual(dump.aprs?.fmTxFrequencyHz, 144_390_000)
    }
}

/// Store behavior that doesn't need the radio or a real .bin. `load` is split
/// out of `open` precisely so a synthetic dump can drive it.
@MainActor
final class CodeplugStoreTests: XCTestCase {

    private func makeDump() -> CodeplugDump {
        CodeplugDump(
            channels: [DumpChannel(index: 0, name: "Simplex", rxFrequencyHz: 146_520_000,
                                   txFrequencyHz: 146_520_000, mode: "Analog", power: "High",
                                   bandwidth: "Wide", colorCode: 1, timeSlot: 1,
                                   contactIndex: 0, radioIdIndex: 0, groupListIndex: 255)],
            zones: [DumpZone(index: 0, name: "Local", channels: [0])],
            contacts: [DumpContact(index: 0, name: "Parrot", number: 9990, callType: "Private")],
            groupLists: [DumpGroupList(index: 0, name: "Wide", members: [0])],
            radioIds: [DumpRadioID(index: 0, name: "Me", number: 3_113_043)]
        )
    }

    func testLoadPopulatesRecordsAndIsNotDirty() {
        let store = CodeplugStore()
        store.load(makeDump(), url: URL(fileURLWithPath: "/tmp/codeplug.bin"))

        XCTAssertEqual(store.channels.count, 1)
        XCTAssertEqual(store.zones.first?.name, "Local")
        XCTAssertEqual(store.radioIds.first?.number, 3_113_043)
        XCTAssertEqual(store.fileURL?.lastPathComponent, "codeplug.bin")
        XCTAssertFalse(store.isDirty, "a freshly loaded codeplug has no staged changes")
    }

    /// Records are keyed by slot, not array position — that identity is what
    /// lets a sorted table commit an edit to the right record.
    func testRecordIDIsItsSlot() {
        let store = CodeplugStore()
        store.load(makeDump(), url: URL(fileURLWithPath: "/tmp/codeplug.bin"))
        XCTAssertEqual(store.channels.first?.id, store.channels.first?.index)
    }
}

/// The membership rules behind the two-column transfer picker used to add
/// channels to a zone and contacts to a group list.
final class MemberTransferTests: XCTestCase {

    /// Five candidates at slots 0...4, labeled A...E.
    private let candidates = (0..<5).map {
        MemberCandidate(index: $0, label: String(UnicodeScalar(UInt8(65 + $0))))
    }

    private func labels(_ items: [MemberCandidate]) -> [String] { items.map(\.label) }

    /// The left column offers only what isn't already a member — that exclusion
    /// is the whole reason a member can't be added to a zone twice.
    func testAvailableExcludesCurrentMembers() {
        let available = MemberTransfer.available(candidates: candidates, members: [1, 3])
        XCTAssertEqual(labels(available), ["A", "C", "E"])
    }

    /// A multi-select add lands in candidate order, not the arbitrary order a
    /// Set iterates in — otherwise the same three picks could produce different
    /// channel orders in the zone from run to run.
    func testAddingAppendsInCandidateOrderNotSelectionOrder() {
        let result = MemberTransfer.adding([4, 0, 2], candidates: candidates, to: [])
        XCTAssertEqual(result, [0, 2, 4])
    }

    /// Adds append, so an existing zone keeps the order the user built.
    func testAddingAppendsAfterExistingMembers() {
        let result = MemberTransfer.adding([0, 2], candidates: candidates, to: [3])
        XCTAssertEqual(result, [3, 0, 2])
    }

    /// Selecting something already a member can't duplicate it: it isn't in the
    /// available column to begin with.
    func testAddingCannotDuplicateAnExistingMember() {
        let result = MemberTransfer.adding([1], candidates: candidates, to: [1])
        XCTAssertEqual(result, [1])
    }

    /// Double-clicking a single available row moves just that one across,
    /// appended after the existing members like any other add.
    func testDoubleClickAddMovesOneMemberAppended() {
        let result = MemberTransfer.adding([2], candidates: candidates, to: [3])
        XCTAssertEqual(result, [3, 2])
    }

    func testRemovingDropsOnlyTheSelectionAndKeepsOrder() {
        let result = MemberTransfer.removing([2], from: [3, 2, 0])
        XCTAssertEqual(result, [3, 0])
    }

    /// Double-clicking a single member row removes just that one.
    func testDoubleClickRemoveMovesOneMemberOut() {
        let result = MemberTransfer.removing([2], from: [3, 2, 0])
        XCTAssertEqual(result, [3, 0])
    }

    func testRemovingHandlesAMultiSelection() {
        let result = MemberTransfer.removing([3, 0], from: [3, 2, 0])
        XCTAssertEqual(result, [2])
    }

    /// The right column shows membership order, not slot order.
    func testMemberItemsFollowMembershipOrder() {
        let items = MemberTransfer.memberItems(candidates: candidates, members: [4, 1])
        XCTAssertEqual(labels(items), ["E", "B"])
    }

    /// A zone can reference a channel that was deleted. It still needs a row, or
    /// the user has no way to remove the dangling reference.
    func testMemberItemsKeepARowForAMissingRecord() {
        let items = MemberTransfer.memberItems(candidates: candidates, members: [9])
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?.index, 9)
        XCTAssertEqual(items.first?.label, "#10 (missing)", "labeled by 1-based slot")
    }

    /// Removing a dangling reference has to work, since that's the only way to
    /// clean one up.
    func testRemovingAMissingMemberWorks() {
        XCTAssertEqual(MemberTransfer.removing([9], from: [0, 9]), [0])
    }
}
