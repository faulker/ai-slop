import SwiftUI

/// App entry point. `--selftest` runs a headless digital loopback check
/// (two Rust sessions, no audio hardware) plus a golden-wav decode when the
/// checkout's testdata is found, and exits before any UI appears.
/// `--decode-golden <wav>` decodes one golden vector file and exits.
/// `--analyze-wav <wav> [--passphrase P] [--mode b85|b128|b170]` measures and
/// decodes a field capture, reporting which stage the burst died at.
/// Otherwise a single chat-style window is shown.
@main
struct AetrApp: App {
    @StateObject private var appState: AppState

    /// Handles the headless launch arguments before SwiftUI spins up.
    init() {
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "--analyze-wav"), i + 1 < args.count {
            let passphrase = Self.value(of: "--passphrase", in: args) ?? ""
            let mode = Self.value(of: "--mode", in: args) ?? "b85"
            exit(runWavAnalysis(path: args[i + 1], passphrase: passphrase, mode: mode) ? 0 : 1)
        }
        if let i = args.firstIndex(of: "--decode-golden"), i + 1 < args.count {
            exit(runGoldenDecode(path: args[i + 1]) ? 0 : 1)
        }
        if args.contains("--selftest") {
            exit(runSelfTest() && runGoldenDecodeIfPresent() ? 0 : 1)
        }
        _appState = StateObject(wrappedValue: AppState())
    }

    /// Reads `--flag value` out of the argument list.
    private static func value(of flag: String, in args: [String]) -> String? {
        guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
        return args[i + 1]
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(appState)
                .frame(minWidth: 640, minHeight: 560)
        }
    }
}
