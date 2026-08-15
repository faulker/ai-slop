import SwiftUI

/// The QYT KT-5000's radio settings.
///
/// # Why this pane is short
///
/// It exposes exactly the fifty-four-byte header at `0x0E00` that
/// `core/src/qyt/codeplug/settings.rs` models, and nothing else. CHIRP parses a
/// 58-byte settings struct here, borrowed from the WP-9900, then sets
/// `has_settings = False` for this model because the map does not hold: their
/// own note says setting "C display mode" overwrites VOX, and three fields in a
/// real capture of this radio decode out of range for the lists CHIRP pairs them
/// with. Offering VOX, scrambler or power-on message from that map would write
/// bytes to a radio that mean something else.
///
/// Everything from `0x0E36` up is byte-preserved and never rewritten, which is
/// what makes a short pane safe rather than merely unfinished. Beep, scan
/// resume, and remember-power are here because one-field diffs confirmed their
/// offsets on this radio (and showed two of them disagree with CHIRP's map).
struct QytSettingsPane: View {
    @EnvironmentObject private var store: CodeplugStore

    /// Pane-local draft. Edits commit on Apply, not per keystroke: each staging
    /// call is a full parse, edit, serialize and re-dump in the core, and it
    /// makes one undo entry.
    @State private var draft: DumpQytSettings = .empty

    var body: some View {
        Group {
            if store.qytSettings == nil {
                noSettingsState
            } else {
                form
            }
        }
        .onAppear { reseed() }
        // Re-seed from the document whenever it changes underneath the pane, so
        // an undo of an applied change shows up here instead of leaving the
        // draft holding a value the file no longer has.
        .onChange(of: store.qytSettings) { draft = $0 ?? .empty }
    }

    private var form: some View {
        Form {
            Section("Radio") {
                LabeledContent("Model tag",
                               value: store.qytModelTag.isEmpty ? "—" : store.qytModelTag)
                    .monospaced()
            }

            Section("Settings") {
                QytRawPicker(title: "Dual watch", value: $draft.tmr, labels: Self.watchModes)
                QytRawPicker(title: "Squelch", value: $draft.squelch, labels: Self.squelchLevels)
                QytRawPicker(title: "Transmit timeout", value: $draft.tot,
                             labels: Self.timeoutSteps)
                Stepper(value: $draft.volume, in: 0...255) {
                    LabeledContent("Power-on volume", value: String(draft.volume))
                }
                QytRawPicker(title: "Backlight timer", value: $draft.backlightTimer,
                             labels: Self.backlightTimes)
                Toggle("Key beep", isOn: $draft.beep)
                QytScanRevPicker(selection: $draft.scanRev)
                Toggle("Remember power state", isOn: $draft.rememberPower)
            }

            Section {
                Text("These are the settings bytes confirmed against a capture of this "
                    + "radio. The rest of the settings block round-trips untouched, and "
                    + "more controls appear here as each field is confirmed against the "
                    + "radio itself rather than inherited from another model's memory map.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .safeAreaInset(edge: .bottom) { ApplyRevertFooter(hasChanges: hasChanges,
                                                          onRevert: reseed,
                                                          onApply: apply) }
    }

    private var noSettingsState: some View {
        VStack(spacing: Spacing.inline) {
            Text("No settings")
                .font(.title3.weight(.medium))
            Text("This codeplug has no QYT settings header.")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    /// True when the draft holds something the document doesn't.
    private var hasChanges: Bool { draft != (store.qytSettings ?? .empty) }

    /// Discard the draft and take the document's values again.
    private func reseed() { draft = store.qytSettings ?? .empty }

    private func apply() { store.updateQytSettings(draft) }

    // MARK: - Value labels

    /// Dual-band waiting (Menu 0 TMR). The KT-5000 is Off/On only; the WP-9900
    /// `LIST_TMR7` M+A/B/C choices do not exist on this radio.
    private static let watchModes = ["Off", "On"]

    /// Squelch levels 0-9; 0 is an open squelch.
    private static let squelchLevels = (0...9).map { $0 == 0 ? "0 (open)" : String($0) }

    /// Transmit timeout, a step index counting 15-second steps from 15 s.
    private static let timeoutSteps = (0...38).map { "\(($0 + 1) * 15) s" }

    /// Backlight timer: 0 is off, otherwise seconds (CHIRP's `LIST_OFF1TO50`).
    private static let backlightTimes = (0...50).map { $0 == 0 ? "Off" : "\($0) s" }
}

/// Picker for scan resume (`TO` / `CO` / `SE`).
///
/// The binding is optional because an unrecognised stored byte must not be
/// silently rewritten to one of the three known values. An unknown value gets
/// its own row; picking a known label replaces it.
struct QytScanRevPicker: View {
    @Binding var selection: String?

    private static let options: [(tag: String, label: String)] = [
        ("TO", "TO (timeout)"),
        ("CO", "CO (carrier)"),
        ("SE", "SE (search)"),
    ]

    var body: some View {
        Picker("Scan resume", selection: Binding(
            get: { selection ?? "" },
            set: { selection = $0.isEmpty ? nil : $0 }
        )) {
            ForEach(Self.options, id: \.tag) { Text($0.label).tag($0.tag) }
            if let current = selection,
               !Self.options.contains(where: { $0.tag == current }) {
                Text("Unknown (\(current))").tag(current)
            }
        }
    }
}

/// A picker over a run of raw byte values, one label per value.
///
/// A value the label list does not cover is offered as its own entry rather than
/// clamped into range. That case is real on this radio: an unprogrammed image
/// reads `0xFF` everywhere, and parts of the settings map are still unconfirmed.
/// Clamping would make the pane rewrite a byte the user never touched.
struct QytRawPicker: View {
    let title: String
    @Binding var value: Int
    let labels: [String]

    var body: some View {
        Picker(title, selection: $value) {
            ForEach(labels.indices, id: \.self) { Text(labels[$0]).tag($0) }
            if !labels.indices.contains(value) {
                Text("Unknown (\(value))").tag(value)
            }
        }
    }
}

/// The Apply / Revert bar the QYT settings and VFO panes share.
///
/// These panes commit explicitly instead of live, because a live `TextField`
/// would stage one core round trip and one undo entry per character typed.
struct ApplyRevertFooter: View {
    let hasChanges: Bool
    let onRevert: () -> Void
    let onApply: () -> Void

    var body: some View {
        HStack(spacing: Spacing.inline) {
            if hasChanges {
                Text("Unapplied changes")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Revert", action: onRevert)
            Button("Apply", action: onApply)
                .keyboardShortcut(.defaultAction)
        }
        .disabled(!hasChanges)
        .padding(.horizontal, Spacing.section)
        .padding(.vertical, Spacing.stack)
        .background(.bar)
    }
}
