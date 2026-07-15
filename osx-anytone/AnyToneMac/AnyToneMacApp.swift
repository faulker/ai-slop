import SwiftUI
import AppKit

/// AnyToneMac: native macOS front end for the anytone-core Rust library.
/// Device tab talks to the radio over USB serial; Codeplug tab edits .bin
/// files offline. All protocol and codeplug logic lives in Rust.
@main
struct AnyToneMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(delegate.codeplug)
                .environmentObject(delegate.device)
                .frame(minWidth: 900, minHeight: 560)
        }
        .commands {
            // There is no "new codeplug" concept: a codeplug only comes from a
            // radio backup or an existing file.
            CommandGroup(replacing: .newItem) {
                Button("Open Codeplug…") { delegate.codeplug.openWithPanel() }
                    .keyboardShortcut("o")
            }
            CommandGroup(replacing: .saveItem) {
                Button("Save") { delegate.codeplug.save() }
                    .keyboardShortcut("s")
                    .disabled(!delegate.codeplug.isDirty)
                Button("Revert to Saved") { delegate.codeplug.discardChanges() }
                    .disabled(!delegate.codeplug.isDirty)
            }
        }
    }
}

/// Owns the stores. They live here rather than in a view because ⌘S in
/// `.commands` and the quit guard in `applicationShouldTerminate` both need to
/// reach them from outside the view hierarchy.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let codeplug = CodeplugStore()
    let device = DeviceStore()

    /// Block quit while the work file holds changes the document doesn't have.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard codeplug.isDirty, let name = codeplug.fileURL?.lastPathComponent else {
            return .terminateNow
        }

        let alert = NSAlert()
        alert.messageText = "Save changes to \(name) before quitting?"
        alert.informativeText = "Your changes are staged but haven't been written to the file yet. "
            + "If you don't save them, they'll be offered back to you the next time you open the app."
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Discard")
        alert.addButton(withTitle: "Cancel")

        switch alert.runModal() {
        case .alertFirstButtonReturn:
            codeplug.save()
            // A failed save leaves an error posted; don't quit over the top of it.
            return codeplug.isDirty ? .terminateCancel : .terminateNow
        case .alertSecondButtonReturn:
            Recovery.clear()
            return .terminateNow
        default:
            return .terminateCancel
        }
    }
}
