import Cocoa
import ServiceManagement

struct KeyBinding {
    let keyCode: Int64
    let modifiers: CGEventFlags

    static let defaultQuickCapture = KeyBinding(keyCode: 11, modifiers: [.maskCommand, .maskShift])  // Cmd+Shift+B
    static let defaultDetailedCapture = KeyBinding(keyCode: 11, modifiers: [.maskCommand, .maskShift, .maskAlternate])  // Cmd+Shift+Option+B
}

final class PreferencesManager {
    static let shared = PreferencesManager()

    private let defaults = UserDefaults.standard

    private enum Keys {
        static let quickCaptureKeyCode = "quickCaptureKeyCode"
        static let quickCaptureModifiers = "quickCaptureModifiers"
        static let detailedCaptureKeyCode = "detailedCaptureKeyCode"
        static let detailedCaptureModifiers = "detailedCaptureModifiers"
    }

    var quickCaptureKey: KeyBinding {
        get { loadBinding(keyCodeKey: Keys.quickCaptureKeyCode, modifiersKey: Keys.quickCaptureModifiers, default: .defaultQuickCapture) }
        set { saveBinding(newValue, keyCodeKey: Keys.quickCaptureKeyCode, modifiersKey: Keys.quickCaptureModifiers) }
    }

    var detailedCaptureKey: KeyBinding {
        get { loadBinding(keyCodeKey: Keys.detailedCaptureKeyCode, modifiersKey: Keys.detailedCaptureModifiers, default: .defaultDetailedCapture) }
        set { saveBinding(newValue, keyCodeKey: Keys.detailedCaptureKeyCode, modifiersKey: Keys.detailedCaptureModifiers) }
    }

    private func loadBinding(keyCodeKey: String, modifiersKey: String, default fallback: KeyBinding) -> KeyBinding {
        guard defaults.object(forKey: keyCodeKey) != nil else { return fallback }
        let keyCode = Int64(defaults.integer(forKey: keyCodeKey))
        // Store modifiers as string to avoid UInt64→Int truncation
        let modRaw: UInt64
        if let modString = defaults.string(forKey: modifiersKey) {
            modRaw = UInt64(modString) ?? fallback.modifiers.rawValue
        } else {
            // Legacy: read as integer for backwards compatibility
            modRaw = UInt64(bitPattern: Int64(defaults.integer(forKey: modifiersKey)))
        }
        return KeyBinding(keyCode: keyCode, modifiers: CGEventFlags(rawValue: modRaw))
    }

    var startAtLogin: Bool {
        get { SMAppService.mainApp.status == .enabled }
        set {
            do {
                if newValue {
                    try SMAppService.mainApp.register()
                } else {
                    try SMAppService.mainApp.unregister()
                }
            } catch {
                NSLog("Failed to \(newValue ? "enable" : "disable") start at login: \(error)")
            }
        }
    }

    private func saveBinding(_ binding: KeyBinding, keyCodeKey: String, modifiersKey: String) {
        defaults.set(Int(binding.keyCode), forKey: keyCodeKey)
        defaults.set(String(binding.modifiers.rawValue), forKey: modifiersKey)
    }

    private init() {}
}
