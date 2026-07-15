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

    /// Re-enumerate serial ports, preselecting the first likely radio.
    func refreshPorts() {
        do {
            ports = try RadioCore.listPorts()
            if selectedPort == nil || !ports.contains(where: { $0.name == selectedPort }) {
                selectedPort = ports.first(where: { $0.likelyRadio })?.name
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
                self?.statusMessage = "Backup saved to \(url.lastPathComponent)"
            }
        }
    }

    /// Write the codeplug file at `url` back to the radio. Only called after
    /// the user confirmed the destructive dialog, hence force: true.
    func restore(from url: URL) {
        guard let port = selectedPort else { return }
        run(label: "Writing codeplug… do not disconnect!") { [weak self] in
            try RadioCore.restore(port: port, from: url.path, force: true) { done, total in
                Task { @MainActor in self?.setProgress(done: done, total: total) }
            }
            await MainActor.run {
                self?.statusMessage = "Restore complete and verified (\(url.lastPathComponent))"
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
            Table(store.ports, selection: $store.selectedPort) {
                TableColumn("Port") { port in
                    Text(port.name)
                }
                TableColumn("Product") { port in
                    Text(port.product ?? "—")
                }
                TableColumn("Radio") { port in
                    if port.likelyRadio {
                        Label("AnyTone", systemImage: "antenna.radiowaves.left.and.right")
                            .foregroundStyle(.green)
                    }
                }
            }
            .overlay { if store.ports.isEmpty { noPortsState } }

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
                .help("Re-scan for serial ports")

                Button("Identify") { store.identify() }
                    .help("Ask the radio what model it is")
                    .disabled(store.selectedPort == nil || store.busy)

                Button("Backup…") { chooseBackupDestination() }
                    .help("Read the radio's codeplug to a file")
                    .disabled(store.selectedPort == nil || store.busy)

                Button("Restore…") { chooseRestoreSource() }
                    .help("Overwrite the radio's codeplug from a file")
                    .disabled(store.selectedPort == nil || store.busy)
            }
        }
        .alert("Error", isPresented: errorBinding) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(store.errorMessage ?? "")
        }
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

    /// Shown when no serial ports are present at all — usually the radio isn't
    /// plugged in, which is worth saying rather than showing bare headers.
    private var noPortsState: some View {
        VStack(spacing: Spacing.inline) {
            Image(systemName: "cable.connector.slash")
                .font(.system(size: 32))
                .foregroundStyle(.secondary)
            Text("No serial ports found")
                .font(.title3.weight(.medium))
            Text("Connect the radio over USB and turn it on, then Refresh.")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
    }

    /// Binding that shows the error alert whenever an error message is set.
    private var errorBinding: Binding<Bool> {
        Binding(
            get: { store.errorMessage != nil },
            set: { if !$0 { store.errorMessage = nil } }
        )
    }

    /// Ask where to save the backup .bin, then start the backup.
    private func chooseBackupDestination() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [UTType(filenameExtension: "bin") ?? .data]
        panel.nameFieldStringValue = "codeplug.bin"
        panel.title = "Save Codeplug Backup"
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
