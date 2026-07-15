import SwiftUI

/// A row in the sidebar: the radio itself, or one entity family in the open
/// codeplug.
enum SidebarItem: Hashable, Identifiable {
    case device
    case codeplug(CodeplugSection)

    var id: Self { self }

    var label: String {
        switch self {
        case .device: return "Device"
        case .codeplug(let section): return section.rawValue
        }
    }

    var symbol: String {
        switch self {
        case .device: return "antenna.radiowaves.left.and.right"
        case .codeplug(let section): return section.symbol
        }
    }
}

/// Window shell: a sidebar selecting between the radio and the open codeplug's
/// entity families, over a status bar that reports the last action.
struct ContentView: View {
    @EnvironmentObject private var store: CodeplugStore
    @EnvironmentObject private var device: DeviceStore
    @State private var selection: SidebarItem? = .device

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
                .safeAreaInset(edge: .bottom, spacing: 0) { statusBar }
        }
        .navigationTitle(store.fileURL?.lastPathComponent ?? "AnyTone Mac")
        .navigationSubtitle(store.isDirty ? "Edited" : "")
        .onAppear { store.checkForRecovery() }
        // A closed file can't have a section selected.
        .onChange(of: store.fileURL) { url in
            if url == nil, case .codeplug = selection { selection = .device }
        }
        .alert("Error", isPresented: errorBinding) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(store.errorMessage ?? "")
        }
        .alert("Restore unsaved changes?", isPresented: recoveryBinding,
               presenting: store.pendingRecovery) { manifest in
            Button("Restore") { store.restoreRecovery(manifest) }
            Button("Discard", role: .destructive) { store.discardRecovery() }
        } message: { manifest in
            Text("""
            AnyTone Mac quit unexpectedly with unsaved changes to \
            "\(URL(fileURLWithPath: manifest.originalPath).lastPathComponent)". \
            You can pick up where you left off, or discard them and open the file as saved.
            """)
        }
    }

    private var sidebar: some View {
        List(selection: $selection) {
            Section("Radio") {
                row(.device)
            }
            Section("Codeplug") {
                ForEach(CodeplugSection.allCases) { section in
                    row(.codeplug(section))
                        // Kept visible but inert with no file open: hiding these
                        // would make the sidebar jump on open and would hide what
                        // the app is for.
                        .disabled(store.fileURL == nil)
                }
            }
        }
        .listStyle(.sidebar)
        .navigationSplitViewColumnWidth(min: 180, ideal: 200, max: 260)
    }

    private func row(_ item: SidebarItem) -> some View {
        Label(item.label, systemImage: item.symbol).tag(item)
    }

    @ViewBuilder
    private var detail: some View {
        switch selection {
        case .device:
            DeviceView()
        case .codeplug(let section):
            CodeplugView(section: section)
        case nil:
            Text("Select a section.")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// Fixed-height footer. Always present so the layout doesn't shift when a
    /// message appears or clears.
    private var statusBar: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: Spacing.inline) {
                Text(activeStatus ?? "")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                if store.isDirty {
                    Label("Unsaved changes", systemImage: "pencil.circle.fill")
                        .font(.callout)
                        .foregroundStyle(.orange)
                }
            }
            .padding(.horizontal, Spacing.stack)
            .frame(height: 28)
        }
        .background(.bar)
    }

    /// The status of whatever the user is looking at. The two stores report
    /// independently, and showing a codeplug message while the Device pane is up
    /// (or vice versa) would just be confusing.
    private var activeStatus: String? {
        switch selection {
        case .device: return device.statusMessage
        default: return store.statusMessage
        }
    }

    private var errorBinding: Binding<Bool> {
        Binding(get: { store.errorMessage != nil },
                set: { if !$0 { store.errorMessage = nil } })
    }

    private var recoveryBinding: Binding<Bool> {
        Binding(get: { store.pendingRecovery != nil },
                set: { if !$0 { store.pendingRecovery = nil } })
    }
}
