import AppKit

/// Replaces text in an AX text element.
///
/// Strategy 1: Set selectedTextRange → verify with readback → set selectedText
/// Strategy 2: Full-text value replacement (works when AX range indexing differs from value indexing)
/// Strategy 3: Clipboard paste fallback
enum TextReplacer {

    private static let logger = Logger(category: "TextReplacer")

    private static func debugLog(_ message: String) {
        let path = "/tmp/spell-i-debug.log"
        let entry = "\(Date()): \(message)\n"
        if let handle = FileHandle(forWritingAtPath: path) {
            handle.seekToEndOfFile()
            handle.write(entry.data(using: .utf8)!)
            handle.closeFile()
        } else {
            FileManager.default.createFile(atPath: path, contents: entry.data(using: .utf8))
        }
    }

    static func replaceText(in element: AXUIElement, range: NSRange, with replacement: String, originalWord: String? = nil) {
        debugLog("=== replaceText called ===")
        debugLog("range=(\(range.location), \(range.length)), replacement='\(replacement)', originalWord='\(originalWord ?? "nil")'")

        // Read current text for verification and fallback
        var textValue: AnyObject?
        let hasText = AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &textValue) == .success
        let currentText = hasText ? (textValue as? String) : nil

        if let text = currentText {
            debugLog("AX text length=\(text.count), first 200 chars: \(String(text.prefix(200)).debugDescription)")
            let nsText = text as NSString
            debugLog("NSString length=\(nsText.length)")
            if range.location + range.length <= nsText.length {
                let textAtRange = nsText.substring(with: range)
                debugLog("Text at input range (\(range.location),\(range.length)): '\(textAtRange)'")
            } else {
                debugLog("Input range OUT OF BOUNDS for NSString")
            }
        } else {
            debugLog("Could not read AX text value")
        }

        // Determine the correct range — verify against actual text
        var targetRange = range
        if let expected = originalWord, !expected.isEmpty, let text = currentText {
            let nsText = text as NSString
            if targetRange.location + targetRange.length <= nsText.length {
                let actual = nsText.substring(with: targetRange)
                if actual != expected {
                    debugLog("Range mismatch: expected '\(expected)' but found '\(actual)'. Searching full text...")
                    let found = nsText.range(of: expected, options: .literal)
                    if found.location != NSNotFound {
                        debugLog("Found '\(expected)' at offset \(found.location)")
                        targetRange = found
                    } else {
                        debugLog("Could NOT find '\(expected)' in text at all!")
                    }
                } else {
                    debugLog("Range verified: text at (\(targetRange.location),\(targetRange.length)) matches '\(expected)'")
                }
            }
        }

        debugLog("Final targetRange=(\(targetRange.location), \(targetRange.length))")

        // Strategy 1: AX selectedTextRange → verified readback → selectedText
        debugLog("Trying Strategy 1 (AX range-based)...")
        if tryRangeBasedReplacement(element: element, range: targetRange, replacement: replacement, expectedWord: originalWord) {
            debugLog("Strategy 1 SUCCEEDED")
            return
        }
        debugLog("Strategy 1 FAILED")

        // Strategy 2: Full-text value replacement
        debugLog("Trying Strategy 2 (full-text value)...")
        if let expected = originalWord, !expected.isEmpty, let text = currentText {
            let nsText = text as NSString
            let found = nsText.range(of: expected, options: .literal)
            if found.location != NSNotFound {
                let newText = nsText.replacingCharacters(in: found, with: replacement)
                let setResult = AXUIElementSetAttributeValue(element, kAXValueAttribute as CFString, newText as CFString)
                debugLog("Strategy 2 setAttributeValue result: \(setResult.rawValue)")
                if setResult == .success {
                    debugLog("Strategy 2 SUCCEEDED")
                    return
                }
            } else {
                debugLog("Strategy 2: could not find word in text")
            }
        }
        debugLog("Strategy 2 FAILED")

        // Strategy 3: Clipboard paste fallback (last resort)
        debugLog("Strategy 3 (clipboard paste) - last resort")
        var cfRange = CFRange(location: targetRange.location, length: targetRange.length)
        if let rangeValue = AXValueCreate(.cfRange, &cfRange) {
            AXUIElementSetAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, rangeValue)
        }
        fallbackReplace(replacement: replacement)
    }

    /// Attempts range-based replacement with selection verification.
    /// Returns true if the replacement succeeded with verified selection.
    private static func tryRangeBasedReplacement(element: AXUIElement, range: NSRange, replacement: String, expectedWord: String?) -> Bool {
        var cfRange = CFRange(location: range.location, length: range.length)
        guard let rangeValue = AXValueCreate(.cfRange, &cfRange) else {
            debugLog("  S1: Failed to create AXValue for range")
            return false
        }

        let rangeResult = AXUIElementSetAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, rangeValue)
        debugLog("  S1: setSelectedTextRange(\(range.location), \(range.length)) → \(rangeResult.rawValue)")
        guard rangeResult == .success else {
            return false
        }

        // Verify selection: read back what's actually selected
        if let expected = expectedWord, !expected.isEmpty {
            var selectedValue: AnyObject?
            let readResult = AXUIElementCopyAttributeValue(element, kAXSelectedTextAttribute as CFString, &selectedValue)
            debugLog("  S1: readback selectedText result=\(readResult.rawValue)")
            if readResult == .success, let selectedText = selectedValue as? String {
                debugLog("  S1: selectedText='\(selectedText)', expected='\(expected)'")
                if selectedText != expected {
                    debugLog("  S1: MISMATCH — aborting")
                    var emptyRange = CFRange(location: range.location, length: 0)
                    if let emptyValue = AXValueCreate(.cfRange, &emptyRange) {
                        AXUIElementSetAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, emptyValue)
                    }
                    return false
                }
                debugLog("  S1: verification PASSED")
            } else {
                debugLog("  S1: Could not read selectedText (readResult=\(readResult.rawValue), isString=\(selectedValue is String)). Proceeding without verification.")
            }
        }

        let valResult = AXUIElementSetAttributeValue(element, kAXSelectedTextAttribute as CFString, replacement as CFString)
        debugLog("  S1: setSelectedText('\(replacement)') → \(valResult.rawValue)")
        if valResult != .success {
            return false
        }
        return true
    }

    /// Fallback: Copy to clipboard and simulate Cmd+V
    private static func fallbackReplace(replacement: String) {
        let pb = NSPasteboard.general
        let changeCountBefore = pb.changeCount

        // Save current clipboard contents
        var savedTypes: [(NSPasteboard.PasteboardType, Data)] = []
        if let items = pb.pasteboardItems {
            for item in items {
                for type in item.types {
                    if let data = item.data(forType: type) {
                        savedTypes.append((type, data))
                    }
                }
            }
        }

        pb.clearContents()
        pb.setString(replacement, forType: .string)

        // Simulate Cmd+V
        let src = CGEventSource(stateID: .hidSystemState)
        let vDown = CGEvent(keyboardEventSource: src, virtualKey: 0x09, keyDown: true)
        let vUp = CGEvent(keyboardEventSource: src, virtualKey: 0x09, keyDown: false)

        vDown?.flags = .maskCommand
        vUp?.flags = .maskCommand

        vDown?.post(tap: .cgAnnotatedSessionEventTap)
        vUp?.post(tap: .cgAnnotatedSessionEventTap)

        // Restore clipboard after delay, but only if the user hasn't copied something new
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            guard pb.changeCount == changeCountBefore + 1 else { return }
            pb.clearContents()
            for (type, data) in savedTypes {
                pb.setData(data, forType: type)
            }
        }
    }
}
