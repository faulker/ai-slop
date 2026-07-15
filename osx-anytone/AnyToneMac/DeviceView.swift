import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// State and device operations for the Device tab. All radio I/O runs on a
/// background task; published state is only touched on the main actor.
@MainActor
final class DeviceStore: ObservableObject {
    @Published var ports: [PortEntry] = []
    @Published var selectedPort: String?
    @Published var model: String?
    @Published var busy = false
    @Published var progressFraction: Double?
    @Published var progressText = ""
    @Published var statusMessage: String?
    @Published var errorMessage: String?

    /// The connected AnyTone radios. The raw port list is mostly Bluetooth and
    /// debug serial devices that can't be programmed, so only VID/PID matches
    /// are ever shown or selectable.
    var radios: [PortEntry] { ports.filter(\.likelyRadio) }

    /// Re-enumerate serial ports, keeping the selection on a still-connected
    /// radio and otherwise falling to the first one found.
    func refreshPorts() {
        do {
            ports = try RadioCore.listPorts()
            if selectedPort == nil || !radios.contains(where: { $0.name == selectedPort }) {
                selectedPort = radios.first?.name
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Identify the selected radio and show its model string.
    func identify() {
        guard let port = selectedPort else { return }
        run(label: "Identifying…") { [weak self] in
            let model = try RadioCore.identify(port: port)
            await MainActor.run {
                self?.model = model
                self?.statusMessage = "Identified: \(model)"
            }
        }
    }

    /// Read the full codeplug from the radio into `url`.
    func backup(to url: URL) {
        guard let port = selectedPort else { return }
        run(label: "Reading codeplug…") { [weak self] in
            try RadioCore.backup(port: port, to: url.path) { done, total in
                Task { @MainActor in self?.setProgress(done: done, total: total) }
            }
            await MainActor.run {
                self?.statusMessage = "Read complete — saved to \(url.lastPathComponent)"
            }
        }
    }

    /// Write the codeplug file at `url` back to the radio. Only called after
    /// the user confirmed the destructive dialog, hence force: true.
    ///
    /// `displayName` names the codeplug in the status message. The Codeplug tab
    /// writes the staging work file, whose own filename ("work.bin") would mean
    /// nothing to the user.
    func restore(from url: URL, displayName: String? = nil) {
        guard let port = selectedPort else { return }
        let name = displayName ?? url.lastPathComponent
        run(label: "Writing codeplug… do not disconnect!") { [weak self] in
            try RadioCore.restore(port: port, from: url.path, force: true) { done, total in
                Task { @MainActor in self?.setProgress(done: done, total: total) }
            }
            await MainActor.run {
                self?.statusMessage = "Write complete and verified (\(name))"
            }
        }
    }

    /// Update the progress bar from a block-progress callback.
    private func setProgress(done: Int, total: Int) {
        progressFraction = total > 0 ? Double(done) / Double(total) : nil
        progressText = "\(done)/\(total) blocks"
    }

    /// Run one device operation on a background task, tracking busy/error
    /// state on the main actor.
    private func run(label: String, _ work: @escaping () async throws -> Void) {
        busy = true
        errorMessage = nil
        statusMessage = label
        progressFraction = nil
        progressText = ""
        Task.detached(priority: .userInitiated) { [weak self] in
            // Rebind to a constant: the store must outlive the operation so
            // completion state lands, and weak vars can't cross actors.
            guard let store = self else { return }
            do {
                try await work()
            } catch {
                await MainActor.run {
                    store.errorMessage = error.localizedDescription
                    store.statusMessage = nil
                }
            }
            await MainActor.run {
                store.busy = false
                store.progressFraction = nil
            }
        }
    }
}

/// Device tab: port list, identify, backup, and (gated) restore.
struct DeviceView: View {
    @EnvironmentObject private var store: DeviceStore
    @State private var showRestoreConfirm = false
    @State private var pendingRestoreURL: URL?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.stack) {
            Table(store.radios, selection: $store.selectedPort) {
                TableColumn("Radio") { port in
                    Label(port.product ?? "AnyTone",
                          systemImage: "antenna.radiowaves.left.and.right")
                        .foregroundStyle(.green)
                }
                TableColumn("Port") { port in
                    Text(port.name).foregroundStyle(.secondary)
                }
            }
            .overlay { if store.radios.isEmpty { noRadioState } }

            if let model = store.model {
                Text("Model: \(model)").font(.callout).textSelection(.enabled)
            }

            if store.busy {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    if let fraction = store.progressFraction {
                        ProgressView(value: fraction)
                        Text(store.progressText).font(.caption).foregroundStyle(.secondary)
                    } else {
                        ProgressView()
                    }
                }
            }
        }
        .padding(Spacing.section)
        .onAppear { store.refreshPorts() }
        .toolbar {
            ToolbarItemGroup {
                Button { store.refreshPorts() } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .help("Re-scan for connected radios")

                Button("Identify") { store.identify() }
                    .help("Ask the radio what model it is")
                    .disabled(store.selectedPort == nil || store.busy)

                Button("Read from Radio…") { chooseBackupDestination() }
                    .help("Read the radio's codeplug to a file")
                    .disabled(store.selectedPort == nil || store.busy)

                Button("Write to Radio…") { chooseRestoreSource() }
                    .help("Overwrite the radio's codeplug from a file")
                    .disabled(store.selectedPort == nil || store.busy)
            }
        }
        // The device error alert lives on ContentView, not here: a write started
        // from the Codeplug tab has to be able to report a failure too.
        .alert("Write codeplug to radio?", isPresented: $showRestoreConfirm, presenting: pendingRestoreURL) { url in
            Button("Write to Radio", role: .destructive) { store.restore(from: url) }
            Button("Cancel", role: .cancel) {}
        } message: { url in
            Text("""
            This OVERWRITES the radio's entire configuration with \
            "\(url.lastPathComponent)". Take a fresh backup first. \
            Do not disconnect the cable or power off the radio during the write.
            """)
        }
    }

    /// Shown when no AnyTone radio is connected — the normal state with the cable
    /// unplugged, which is worth saying rather than showing bare headers. The
    /// Refresh button is here as well as in the toolbar because this is where the
    /// user is looking when the list is empty.
    private var noRadioState: some View {
        VStack(spacing: Spacing.inline) {
            Image(systemName: "cable.connector.slash")
                .font(.system(size: 32))
                .foregroundStyle(.secondary)
            Text("No AnyTone radio connected")
                .font(.title3.weight(.medium))
            Text("Connect the radio over USB and turn it on.")
                .foregroundStyle(.secondary)
            Button("Refresh") { store.refreshPorts() }
                .padding(.top, Spacing.tight)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
    }

    /// Ask where to save the codeplug read from the radio, then start the read.
    private func chooseBackupDestination() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [UTType(filenameExtension: "bin") ?? .data]
        panel.nameFieldStringValue = "codeplug.bin"
        panel.title = "Save Codeplug Read from Radio"
        if panel.runModal() == .OK, let url = panel.url {
            store.backup(to: url)
        }
    }

    /// Ask which .bin to restore, then show the destructive confirmation.
    private func chooseRestoreSource() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [UTType(filenameExtension: "bin") ?? .data]
        panel.allowsMultipleSelection = false
        panel.title = "Choose Codeplug to Write to Radio"
        if panel.runModal() == .OK, let url = panel.url {
            pendingRestoreURL = url
            showRestoreConfirm = true
        }
    }
}
