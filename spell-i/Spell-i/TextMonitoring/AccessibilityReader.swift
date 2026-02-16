import AppKit
import ApplicationServices

/// Reads text content and cursor position from the focused UI element via Accessibility API.
final class AccessibilityReader {

    struct TextContext {
        /// The full text value of the focused element.
        let text: String
        /// The selected text range (cursor position).
        let selectedRange: NSRange
        /// The AX element (retained for bounds queries).
        let element: AXUIElement
    }

    private let logger = Logger(category: "AccessibilityReader")

    private let blacklistedBundleIDs: Set<String> = [
        "com.apple.Terminal",
        "com.googlecode.iterm2",
        "com.agilebits.onepassword7",
        "com.1password.1password"
    ]

    /// Maximum depth to traverse when searching for a text element in the AX tree.
    private let maxTraversalDepth = 8

    // MARK: - Public

    /// Returns the text context of the currently focused text element, or nil if unavailable.
    func readFocusedElement() -> TextContext? {
        guard let app = NSWorkspace.shared.frontmostApplication,
              let bundleID = app.bundleIdentifier else {
            logger.debug("readFocusedElement: no frontmost app")
            return nil
        }

        if blacklistedBundleIDs.contains(bundleID) {
            return nil
        }

        let axApp = AXUIElementCreateApplication(app.processIdentifier)

        var focusedElement: AnyObject?
        let result = AXUIElementCopyAttributeValue(axApp, kAXFocusedUIElementAttribute as CFString, &focusedElement)
        guard result == .success, let element = focusedElement else {
            logger.debug("readFocusedElement: no focused element in \(bundleID) (AX error: \(result.rawValue))")
            return nil
        }

        // swiftlint:disable:next force_cast
        let axElement = element as! AXUIElement

        // Try reading text directly from the focused element
        if let context = textContext(from: axElement) {
            return context
        }

        // For Electron/Chromium apps (Slack, VS Code, Discord, etc.):
        // The app-level focused element may be a web area or container
        // that doesn't directly expose kAXValueAttribute.
        // Try deeper focus traversal and child search.
        let role = axRole(of: axElement)
        logger.debug("readFocusedElement: no text on focused element (role=\(role ?? "nil"), app=\(bundleID)), trying deeper traversal")

        // Strategy 1: Follow nested kAXFocusedUIElementAttribute down the tree
        if let deepElement = deepFocusedElement(from: axElement),
           let context = textContext(from: deepElement) {
            return context
        }

        // Strategy 2: Search children for a text-bearing element
        if let childContext = findTextElementInChildren(of: axElement, depth: 0) {
            return childContext
        }

        return nil
    }

    // MARK: - Deep Focus Traversal

    /// Follows kAXFocusedUIElementAttribute recursively to find the deepest focused element.
    /// Electron/Chromium apps expose nested focus: App → Window → WebArea → Group → TextArea.
    private func deepFocusedElement(from element: AXUIElement) -> AXUIElement? {
        var current = element
        for _ in 0..<maxTraversalDepth {
            var childFocus: AnyObject?
            let result = AXUIElementCopyAttributeValue(current, kAXFocusedUIElementAttribute as CFString, &childFocus)
            guard result == .success, let child = childFocus else {
                return nil
            }
            // swiftlint:disable:next force_cast
            let childElement = child as! AXUIElement
            // Avoid infinite loops if element reports itself as focused
            if CFEqual(childElement, current) {
                return nil
            }
            // Check if this deeper element has text
            if hasTextValue(childElement) {
                return childElement
            }
            current = childElement
        }
        return nil
    }

    /// Searches children of the given element for one that has kAXValueAttribute (text).
    /// Prioritizes elements with editable text roles (AXTextArea, AXTextField).
    private func findTextElementInChildren(of element: AXUIElement, depth: Int) -> TextContext? {
        guard depth < maxTraversalDepth else { return nil }

        var childrenValue: AnyObject?
        let result = AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenValue)
        guard result == .success, let children = childrenValue as? [AXUIElement] else {
            return nil
        }

        // First pass: look for text-bearing editable elements
        for child in children {
            let role = axRole(of: child)
            if role == "AXTextArea" || role == "AXTextField" {
                if let context = textContext(from: child) {
                    return context
                }
            }
        }

        // Second pass: recurse into web areas and groups
        for child in children {
            let role = axRole(of: child)
            if role == "AXWebArea" || role == "AXGroup" || role == "AXScrollArea" {
                if let context = findTextElementInChildren(of: child, depth: depth + 1) {
                    return context
                }
            }
        }

        return nil
    }

    // MARK: - AX Helpers

    /// Roles that represent editable text elements supporting kAXBoundsForRangeParameterizedAttribute.
    private static let textEditRoles: Set<String> = [
        "AXTextArea", "AXTextField", "AXComboBox", "AXSearchField"
    ]

    /// Attempts to build a TextContext from the given element.
    /// Only returns a context for text-editor elements (AXTextArea, AXTextField, etc.)
    /// that are likely to support kAXBoundsForRangeParameterizedAttribute.
    private func textContext(from element: AXUIElement) -> TextContext? {
        // Only accept text-editor roles — other elements (lists, tables, buttons)
        // may expose kAXValueAttribute but don't support character-level bounds queries.
        guard let role = axRole(of: element),
              Self.textEditRoles.contains(role) else {
            return nil
        }

        var textValue: AnyObject?
        let textResult = AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &textValue)
        guard textResult == .success, let text = textValue as? String else {
            return nil
        }

        // Read selected range
        let selectedRange = readSelectedRange(from: element)
        return TextContext(text: text, selectedRange: selectedRange, element: element)
    }

    /// Reads the selected text range from an element, returning a default if unavailable.
    private func readSelectedRange(from element: AXUIElement) -> NSRange {
        var rangeValue: AnyObject?
        var selectedRange = NSRange(location: 0, length: 0)
        if AXUIElementCopyAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, &rangeValue) == .success,
           let rangeObj = rangeValue {
            // swiftlint:disable:next force_cast
            let rangeRef = rangeObj as! AXValue
            var range = CFRange(location: 0, length: 0)
            if AXValueGetValue(rangeRef, .cfRange, &range) {
                selectedRange = NSRange(location: range.location, length: range.length)
            }
        }
        return selectedRange
    }

    /// Returns the AX role string for an element, or nil.
    private func axRole(of element: AXUIElement) -> String? {
        var roleValue: AnyObject?
        if AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &roleValue) == .success {
            return roleValue as? String
        }
        return nil
    }

    /// Checks whether an element has a non-empty kAXValueAttribute string.
    private func hasTextValue(_ element: AXUIElement) -> Bool {
        var textValue: AnyObject?
        return AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &textValue) == .success
            && textValue is String
    }

    /// Returns the screen-space bounds of a character range in the given AX element.
    /// AX returns bottom-left origin coords.
    func boundsForRange(_ range: NSRange, in element: AXUIElement) -> CGRect? {
        var cfRange = CFRange(location: range.location, length: range.length)
        guard let rangeValue = AXValueCreate(.cfRange, &cfRange) else { return nil }

        var boundsValue: AnyObject?
        let result = AXUIElementCopyParameterizedAttributeValue(
            element,
            kAXBoundsForRangeParameterizedAttribute as CFString,
            rangeValue,
            &boundsValue
        )
        guard result == .success, let axValue = boundsValue else { return nil }

        var rect = CGRect.zero
        // swiftlint:disable:next force_cast
        let value = axValue as! AXValue
        if AXValueGetValue(value, .cgRect, &rect) {
            return rect
        }
        return nil
    }
}
