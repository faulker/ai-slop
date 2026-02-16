import AppKit

enum Constants {

    /// App bundle identifier.
    static let bundleIdentifier = "com.spell-i.app"

    /// Logging subsystem identifier.
    static let subsystem = "com.spell-i.app"

    /// App display name.
    static let appName = "Spell-i"

    // MARK: - Underline Colors

    static let spellingErrorColor = NSColor.systemRed
    static let grammarErrorColor = NSColor.systemBlue

    // MARK: - Timing

    /// Default debounce interval for typing detection (seconds).
    static let defaultDebounceInterval: TimeInterval = 0.4

    // MARK: - Overlay

    /// Extra padding around hit-test areas for squiggly underlines.
    static let hitTestPadding: CGFloat = 4.0

    /// Squiggly line amplitude.
    static let squigglyAmplitude: CGFloat = 3.0

    /// Squiggly line wavelength (period).
    static let squigglyWavelength: CGFloat = 6.0

    /// Squiggly line stroke width.
    static let squigglyStrokeWidth: CGFloat = 1.5

    // MARK: - Engine

    /// GCD queue label for spell engine operations.
    static let engineQueueLabel = "com.spell-i.engine"

    /// Maximum number of suggestions to show in the popup.
    static let maxSuggestions = 5

    // MARK: - Popup

    /// Minimum popup width.
    static let popupMinWidth: CGFloat = 150.0

    /// Maximum popup width.
    static let popupMaxWidth: CGFloat = 250.0

    /// Row height in the correction popup.
    static let popupRowHeight: CGFloat = 24.0
}
