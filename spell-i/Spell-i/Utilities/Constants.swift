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

    /// Debounce interval for window move/scroll events (seconds).
    static let windowMoveDebounceInterval: TimeInterval = 0.3

    /// Delays between engine init retries (seconds). Index = attempt number.
    static let engineRetryDelays: [TimeInterval] = [2.0, 5.0, 10.0]

    // MARK: - Overlay

    /// Extra padding around hit-test areas for squiggly underlines.
    static let hitTestPadding: CGFloat = 4.0

    /// Squiggly line amplitude.
    static let squigglyAmplitude: CGFloat = 3.0

    /// Squiggly line wavelength (period).
    static let squigglyWavelength: CGFloat = 6.0

    /// Squiggly line stroke width.
    static let squigglyStrokeWidth: CGFloat = 1.5

    // MARK: - Accessibility Traversal

    /// Maximum depth to traverse when searching for a text element in the AX tree.
    /// Bumped from 8 to handle deeply nested Chromium/Electron hierarchies.
    static let maxTraversalDepth = 12

    /// Roles that represent editable text elements supporting bounds queries.
    /// Includes `AXStaticText` for Chrome contenteditable elements (guarded by isEditable check).
    static let textEditRoles: Set<String> = [
        "AXTextArea", "AXTextField", "AXComboBox", "AXSearchField", "AXStaticText"
    ]

    /// Container roles that should be recursed into when searching for text elements.
    static let containerRolesForTraversal: Set<String> = [
        "AXWebArea", "AXGroup", "AXScrollArea", "AXList", "AXCell",
        "AXSection", "AXLayoutArea", "AXSplitGroup", "AXTabGroup",
        "AXRow", "AXOutline", "AXTable"
    ]

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
