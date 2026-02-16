import AppKit
import ApplicationServices

/// Checks and manages Accessibility permission state.
struct AccessibilityPermissionChecker {

    /// Returns true if the app has Accessibility permission.
    static func isAccessibilityEnabled() -> Bool {
        return AXIsProcessTrusted()
    }

    /// Opens System Settings to the Accessibility privacy pane.
    static func openAccessibilitySettings() {
        let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!
        NSWorkspace.shared.open(url)
    }
}
