import Combine
import Foundation

/// User preferences shared by the menu and the settings window, persisted in UserDefaults.
@MainActor
public final class BlockerSettings: ObservableObject {
    enum Key {
        static let blockingEnabled = "blockingEnabled"
        static let showMenuBarIcon = "showMenuBarIcon"
        static let hasLaunched = "hasLaunched"
    }

    private let defaults: UserDefaults

    @Published public var blockingEnabled: Bool {
        didSet { defaults.set(blockingEnabled, forKey: Key.blockingEnabled) }
    }

    @Published public var showMenuBarIcon: Bool {
        didSet { defaults.set(showMenuBarIcon, forKey: Key.showMenuBarIcon) }
    }

    /// How many times Music was killed since the app started. Not persisted.
    @Published public var blockedCount = 0

    /// True only on the very first run, used to show the settings window once.
    public let isFirstLaunch: Bool

    /// Loads settings from `defaults`; both toggles default to on.
    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        defaults.register(defaults: [Key.blockingEnabled: true, Key.showMenuBarIcon: true])
        blockingEnabled = defaults.bool(forKey: Key.blockingEnabled)
        showMenuBarIcon = defaults.bool(forKey: Key.showMenuBarIcon)
        isFirstLaunch = !defaults.bool(forKey: Key.hasLaunched)
        defaults.set(true, forKey: Key.hasLaunched)
    }
}
