import BlockerCore
import SwiftUI

/// The small settings window: the same options as the menu bar menu, plus a block counter.
struct SettingsView: View {
    @ObservedObject var settings: BlockerSettings
    @State private var launchAtLogin = LoginItem.isEnabled
    @State private var loginError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Toggle("Block Apple Music", isOn: $settings.blockingEnabled)
            Toggle("Show menu bar icon", isOn: $settings.showMenuBarIcon)
            Toggle("Launch at login", isOn: $launchAtLogin)
                .onChange(of: launchAtLogin) { _, newValue in
                    // Skip when this change is just the revert below, or already matches macOS.
                    guard newValue != LoginItem.isEnabled else { return }
                    do {
                        try LoginItem.set(newValue)
                        loginError = nil
                    } catch {
                        loginError = "Couldn't change login item: \(error.localizedDescription)"
                        launchAtLogin = LoginItem.isEnabled
                    }
                }

            if let loginError {
                Text(loginError).font(.caption).foregroundStyle(.red)
            }

            Divider()

            HStack {
                Text(statusText).font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Quit") { NSApp.terminate(nil) }
            }

            if !settings.showMenuBarIcon {
                Text("Icon hidden. Open the app again to get back here.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(20)
        .frame(width: 320)
        .onAppear { launchAtLogin = LoginItem.isEnabled }
    }

    private var statusText: String {
        let times = settings.blockedCount == 1 ? "time" : "times"
        return "Blocked Music \(settings.blockedCount) \(times) this session"
    }
}
