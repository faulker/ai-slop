import Foundation

/// Persists user-configurable app settings via UserDefaults.
final class Settings {

    static let shared = Settings()

    private let defaults = UserDefaults.standard

    // MARK: - Keys

    private enum Key {
        static let maxTraversalDepth = "maxTraversalDepth"
    }

    // MARK: - Accessibility Traversal

    /// Maximum AX tree traversal depth. Falls back to the compiled default if never set.
    var maxTraversalDepth: Int {
        get {
            let stored = defaults.integer(forKey: Key.maxTraversalDepth)
            return stored > 0 ? stored : Constants.maxTraversalDepth
        }
        set {
            let clamped = min(max(newValue, 4), 30)
            defaults.set(clamped, forKey: Key.maxTraversalDepth)
        }
    }
}
