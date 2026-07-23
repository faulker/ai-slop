import SwiftUI

/// App entry point. `--selftest` runs a headless digital loopback check
/// (two Rust sessions, no audio hardware) plus a golden-wav decode when the
/// checkout's testdata is found, and exits before any UI appears.
/// `--decode-golden <wav>` decodes one golden vector file and exits.
/// Otherwise a single chat-style window is shown.
@main
struct AetrApp: App {
    @StateObject private var appState: AppState

    /// Handles the headless launch arguments before SwiftUI spins up.
    init() {
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "--decode-golden"), i + 1 < args.count {
            exit(runGoldenDecode(path: args[i + 1]) ? 0 : 1)
        }
        if args.contains("--selftest") {
            exit(runSelfTest() && runGoldenDecodeIfPresent() ? 0 : 1)
        }
        _appState = StateObject(wrappedValue: AppState())
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(appState)
                .frame(minWidth: 640, minHeight: 560)
        }
    }
}
