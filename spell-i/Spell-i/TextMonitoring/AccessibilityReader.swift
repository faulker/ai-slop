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
    /// Reads the user-configured value each time so changes take effect without restart.
    private var maxTraversalDepth: Int {
        Settings.shared.maxTraversalDepth
    }

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

        // Follow the focus chain as deep as possible, even if the deepest
        // element doesn't have text itself.
        let deepElement = deepestFocusedElement(from: axElement)

        // Strategy 1: Check if the deepest focused element has text directly
        if let deep = deepElement, let context = textContext(from: deep) {
            return context
        }

        // Strategy 2: Search children starting from the deepest focused element
        // (much closer to the actual text input than the top-level element)
        if let deep = deepElement {
            if let context = findTextElementInChildren(of: deep, depth: 0) {
                return context
            }
        }

        // Strategy 3: Search children from the original focused element
        if let childContext = findTextElementInChildren(of: axElement, depth: 0) {
            return childContext
        }

        return nil
    }

    // MARK: - Deep Focus Traversal

    /// Follows kAXFocusedUIElementAttribute recursively to find the deepest focused element.
    /// Returns the deepest element reached in the chain, even if it doesn't have text.
    /// This lets callers search that element's children for the actual text input.
    /// Electron/Chromium apps expose nested focus: App → Window → WebArea → Group → TextArea.
    private func deepestFocusedElement(from element: AXUIElement) -> AXUIElement? {
        var current = element
        var deepest: AXUIElement?
        for _ in 0..<maxTraversalDepth {
            var childFocus: AnyObject?
            let result = AXUIElementCopyAttributeValue(current, kAXFocusedUIElementAttribute as CFString, &childFocus)
            guard result == .success, let child = childFocus else {
                break
            }
            // swiftlint:disable:next force_cast
            let childElement = child as! AXUIElement
            // Avoid infinite loops if element reports itself as focused
            if CFEqual(childElement, current) {
                break
            }
            deepest = childElement
            // If this element has text, return it immediately (best case)
            if hasTextValue(childElement) {
                return childElement
            }
            current = childElement
        }
        return deepest
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

        // First pass: look for text-bearing editable elements (all text edit roles)
        for child in children {
            if let role = axRole(of: child), Constants.textEditRoles.contains(role) {
                if let context = textContext(from: child) {
                    return context
                }
            }
        }

        // Also check for AXGroup/unknown roles with settable text (Electron contenteditable)
        for child in children {
            if hasSettableTextValue(child) {
                if let context = textContextPermissive(from: child) {
                    return context
                }
            }
        }

        // Second pass: recurse into known containers
        for child in children {
            if let role = axRole(of: child), Constants.containerRolesForTraversal.contains(role) {
                if let context = findTextElementInChildren(of: child, depth: depth + 1) {
                    return context
                }
            }
        }

        // Third pass: recurse into any child with children (aggressive fallback for
        // Electron apps that use unexpected roles as containers)
        for child in children {
            let role = axRole(of: child)
            // Skip roles we already tried and leaf-like roles
            if let r = role, Constants.containerRolesForTraversal.contains(r) { continue }
            if role == "AXButton" || role == "AXImage" || role == "AXMenuItem" { continue }
            if hasChildren(child) {
                if let context = findTextElementInChildren(of: child, depth: depth + 1) {
                    return context
                }
            }
        }

        return nil
    }

    // MARK: - AX Helpers

    /// Attempts to build a TextContext from the given element.
    /// Only returns a context for text-editor elements (AXTextArea, AXTextField, etc.)
    /// that are likely to support kAXBoundsForRangeParameterizedAttribute.
    private func textContext(from element: AXUIElement) -> TextContext? {
        // Only accept text-editor roles — other elements (lists, tables, buttons)
        // may expose kAXValueAttribute but don't support character-level bounds queries.
        guard let role = axRole(of: element),
              Constants.textEditRoles.contains(role) else {
            return nil
        }

        // AXStaticText can match labels/headings — only accept if the value is settable
        if role == "AXStaticText" {
            var settable: DarwinBoolean = false
            let err = AXUIElementIsAttributeSettable(element, kAXValueAttribute as CFString, &settable)
            if err != .success || !settable.boolValue {
                return nil
            }
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

    /// Checks whether an element has a settable (writable) text value.
    /// Used for Electron contenteditable elements that may have non-standard roles.
    private func hasSettableTextValue(_ element: AXUIElement) -> Bool {
        guard hasTextValue(element) else { return false }
        var settable: DarwinBoolean = false
        let err = AXUIElementIsAttributeSettable(element, kAXValueAttribute as CFString, &settable)
        return err == .success && settable.boolValue
    }

    /// Builds a TextContext from any element with settable text, regardless of role.
    /// Used as a fallback for Electron contenteditable divs that appear as AXGroup.
    private func textContextPermissive(from element: AXUIElement) -> TextContext? {
        guard hasSettableTextValue(element) else { return nil }

        var textValue: AnyObject?
        let textResult = AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &textValue)
        guard textResult == .success, let text = textValue as? String else {
            return nil
        }

        let role = axRole(of: element) ?? "unknown"
        logger.debug("textContextPermissive: accepted element with role=\(role)")

        let selectedRange = readSelectedRange(from: element)
        return TextContext(text: text, selectedRange: selectedRange, element: element)
    }

    /// Checks whether an element has children (used to decide if it's worth recursing into).
    private func hasChildren(_ element: AXUIElement) -> Bool {
        var count: AnyObject?
        if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &count) == .success,
           let children = count as? [AnyObject] {
            return !children.isEmpty
        }
        return false
    }

    /// Returns the AX subrole string for an element, or nil.
    private func axSubrole(of element: AXUIElement) -> String? {
        var subroleValue: AnyObject?
        if AXUIElementCopyAttributeValue(element, kAXSubroleAttribute as CFString, &subroleValue) == .success {
            return subroleValue as? String
        }
        return nil
    }

    /// Searches child/grandchild elements for one containing the given word,
    /// returning the tightest available bounds. Falls back to the parent element's
    /// bounds if no matching child is found. This produces much better underline
    /// positioning in Electron/Chromium apps where the parent element covers the
    /// entire compose area (including toolbars).
    func boundsForTextChild(containing word: String, in element: AXUIElement) -> CGRect? {
        var childrenValue: AnyObject?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenValue) == .success,
              let children = childrenValue as? [AXUIElement] else {
            return boundsForElement(element)
        }

        // Search children and grandchildren for the tightest bounds containing the word
        var bestBounds: CGRect?
        var bestArea: CGFloat = .greatestFiniteMagnitude

        for child in children {
            if let bounds = matchingBounds(for: word, in: child) {
                let area = bounds.width * bounds.height
                if area < bestArea {
                    bestBounds = bounds
                    bestArea = area
                }
            }
            // Check grandchildren (Slack: AXTextArea → AXGroup → AXStaticText)
            var gcValue: AnyObject?
            if AXUIElementCopyAttributeValue(child, kAXChildrenAttribute as CFString, &gcValue) == .success,
               let grandchildren = gcValue as? [AXUIElement] {
                for gc in grandchildren {
                    if let bounds = matchingBounds(for: word, in: gc) {
                        let area = bounds.width * bounds.height
                        if area < bestArea {
                            bestBounds = bounds
                            bestArea = area
                        }
                    }
                }
            }
        }

        return bestBounds ?? boundsForElement(element)
    }

    /// Returns the bounds of an element if it contains the given word and has non-zero size.
    private func matchingBounds(for word: String, in element: AXUIElement) -> CGRect? {
        var textValue: AnyObject?
        guard AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &textValue) == .success,
              let text = textValue as? String,
              text.contains(word) else {
            return nil
        }
        guard let bounds = boundsForElement(element),
              bounds.width > 0, bounds.height > 0 else {
            return nil
        }
        return bounds
    }

    /// Returns the element's full bounding box (position + size) as a coarse fallback
    /// when per-character bounds are unavailable.
    func boundsForElement(_ element: AXUIElement) -> CGRect? {
        var posValue: AnyObject?
        var sizeValue: AnyObject?
        guard AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &posValue) == .success,
              AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &sizeValue) == .success else {
            return nil
        }

        var point = CGPoint.zero
        var size = CGSize.zero
        // swiftlint:disable:next force_cast
        guard AXValueGetValue(posValue as! AXValue, .cgPoint, &point),
              // swiftlint:disable:next force_cast
              AXValueGetValue(sizeValue as! AXValue, .cgSize, &size) else {
            return nil
        }

        return CGRect(origin: point, size: size)
    }

    // MARK: - AX Tree Dump (Debug)

    /// Dumps the AX tree of the frontmost app's focused element to the logger.
    /// Useful for diagnosing why Electron/Chromium apps aren't detected.
    func dumpFocusedElementTree() {
        guard let app = NSWorkspace.shared.frontmostApplication,
              let bundleID = app.bundleIdentifier else {
            logger.info("dumpAXTree: no frontmost app")
            return
        }

        let axApp = AXUIElementCreateApplication(app.processIdentifier)
        var focusedElement: AnyObject?
        let result = AXUIElementCopyAttributeValue(axApp, kAXFocusedUIElementAttribute as CFString, &focusedElement)
        guard result == .success, let element = focusedElement else {
            logger.info("dumpAXTree: no focused element in \(bundleID)")
            return
        }

        // swiftlint:disable:next force_cast
        let axElement = element as! AXUIElement
        var lines: [String] = ["AX Tree for \(bundleID):"]
        dumpElement(axElement, depth: 0, maxDepth: maxTraversalDepth, into: &lines)

        let output = lines.joined(separator: "\n")
        logger.info("\(output)")
    }

    private func dumpElement(_ element: AXUIElement, depth: Int, maxDepth: Int, into lines: inout [String]) {
        guard depth <= maxDepth else { return }
        let indent = String(repeating: "  ", count: depth)
        let role = axRole(of: element) ?? "?"
        let subrole = axSubrole(of: element)
        let hasText = hasTextValue(element)
        let settable = hasSettableTextValue(element)

        var focusChain = ""
        var childFocus: AnyObject?
        if AXUIElementCopyAttributeValue(element, kAXFocusedUIElementAttribute as CFString, &childFocus) == .success {
            focusChain = " [has-focused-child]"
        }

        let sub = subrole.map { " sub=\($0)" } ?? ""
        lines.append("\(indent)\(role)\(sub) text=\(hasText) settable=\(settable)\(focusChain)")

        var childrenValue: AnyObject?
        if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenValue) == .success,
           let children = childrenValue as? [AXUIElement] {
            // Limit children output to avoid flooding
            let limit = min(children.count, 20)
            for i in 0..<limit {
                dumpElement(children[i], depth: depth + 1, maxDepth: maxDepth, into: &lines)
            }
            if children.count > limit {
                lines.append("\(indent)  ... and \(children.count - limit) more children")
            }
        }
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
