import AppKit
import ApplicationServices

/// Central coordinator for the text monitoring pipeline.
/// Owns the event tap, debouncer, AX reader, focus tracker, and spell engine.
/// Dispatches lint operations to a background queue and results to the main thread.
final class TextMonitorCoordinator: EventTapDelegate, FocusTrackerDelegate {

    // MARK: - Sub-components

    private let eventTapManager = EventTapManager()
    private let debouncer = TypingDebouncer()
    private let accessibilityReader = AccessibilityReader()
    private let focusTracker = FocusTracker()

    // MARK: - Engine

    private var engine: SpellEngine?
    private let engineQueue = DispatchQueue(label: Constants.engineQueueLabel, qos: .userInitiated)

    // MARK: - State

    /// Session-level ignore set (words ignored this session).
    private var sessionIgnoreList = Set<String>()

    /// Generation counter to ignore stale lint results.
    private var lintGeneration: Int = 0

    /// The AX element from the most recent lint pass, used for corrections
    /// even if focus has shifted (e.g. to the correction popup).
    private(set) var lastLintElement: AXUIElement?

    /// When true, lint operations and focus change handling are suppressed.
    /// Set while the correction popup is visible to prevent `lastLintElement`
    /// and `currentResults` from being overwritten by focus-change events.
    var isPopupActive = false

    /// Last lint results keyed by word range, used for popup lookups.
    private(set) var currentResults: [LintDisplayItem] = []

    /// Callback for overlay updates.
    var onLintResults: (([LintDisplayItem]) -> Void)?

    /// Callback for requesting a correction popup.
    var onUnderlineClicked: ((LintDisplayItem, CGRect) -> Void)?

    private let logger = Logger(category: "Coordinator")

    // MARK: - Types

    struct LintDisplayItem {
        let errorType: String
        let message: String
        let startOffset: Int
        let endOffset: Int
        let suggestions: [String]
        let screenRect: CGRect
        let originalWord: String
    }

    // MARK: - Lifecycle

    /// Initialize the spell engine on the background queue, then call completion on main thread.
    /// Engine is set on engineQueue to avoid cross-thread access.
    func initializeEngine(completion: @escaping () -> Void) {
        engineQueue.async { [weak self] in
            let eng = SpellEngine()
            self?.engine = eng
            DispatchQueue.main.async {
                self?.logger.info("SpellEngine initialized")
                completion()
            }
        }
    }

    /// Starts the full monitoring pipeline.
    func start() {
        eventTapManager.delegate = self
        let tapInstalled = eventTapManager.install()

        if !tapInstalled {
            logger.error("CGEventTap failed to install — keystrokes will not be captured. Check Accessibility permission.")
            // Retry once after a short delay (macOS sometimes needs a moment after permission grant)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
                guard let self = self else { return }
                if self.eventTapManager.install() {
                    self.logger.info("CGEventTap installed on retry")
                } else {
                    self.logger.error("CGEventTap retry also failed — monitoring will only work on focus changes")
                }
            }
        }

        debouncer.onDebounced = { [weak self] in
            self?.performLint()
        }

        focusTracker.delegate = self
        focusTracker.startTracking()

        // Perform an initial lint on the currently focused element
        performLint()

        logger.info("Monitoring started (eventTap=\(tapInstalled))")
    }

    /// Stops the monitoring pipeline.
    func stop() {
        eventTapManager.uninstall()
        debouncer.cancel()
        focusTracker.stopTracking()
        clearResults()
        logger.info("Monitoring stopped")
    }

    // MARK: - EventTapDelegate

    func eventTapDidReceiveKeystroke() {
        debouncer.keystroke()
    }

    // MARK: - FocusTrackerDelegate

    func focusTrackerDidChangeApp() {
        guard !isPopupActive else { return }
        clearResults()
        // Lint the newly focused app's text after a short delay to let AX settle
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            self?.performLint()
        }
    }

    func focusTrackerDidChangeElement() {
        guard !isPopupActive else { return }
        clearResults()
        // Lint the newly focused element's text after a short delay
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            self?.performLint()
        }
    }

    // MARK: - Dictionary Actions

    func addWordToDictionary(_ word: String) {
        engineQueue.async { [weak self] in
            self?.engine?.add_user_word(word)
            DispatchQueue.main.async {
                self?.performLint()
            }
        }
    }

    func ignoreWord(_ word: String) {
        sessionIgnoreList.insert(word.lowercased())
        // Remove matching results immediately
        currentResults.removeAll { $0.originalWord.lowercased() == word.lowercased() }
        onLintResults?(currentResults)
    }

    // MARK: - Correction

    func applyCorrection(replacement: String, range: NSRange, element: AXUIElement? = nil, originalWord: String? = nil) {
        let target = element ?? lastLintElement
        guard let target = target else { return }
        TextReplacer.replaceText(in: target, range: range, with: replacement, originalWord: originalWord)
        // Re-lint after a short delay to let the text field update
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            self?.performLint()
        }
    }

    // MARK: - Private

    /// Intermediate type for lint data extracted on the background thread (no AX calls).
    private struct RawLint {
        let errorType: String
        let message: String
        let charStart: Int
        let charEnd: Int
        let suggestions: [String]
        let originalWord: String
    }

    private func performLint() {
        // Don't lint while the correction popup is active — it would overwrite
        // lastLintElement and currentResults via focus-change events.
        guard !isPopupActive else { return }

        // MAIN THREAD: Read text from AX
        guard let context = accessibilityReader.readFocusedElement() else {
            logger.debug("performLint: no focused text element available")
            clearResults()
            return
        }

        let text = context.text
        let element = context.element
        lastLintElement = element

        guard !text.isEmpty else {
            clearResults()
            return
        }

        lintGeneration += 1
        let currentGen = lintGeneration
        let ignoreSnapshot = sessionIgnoreList

        logger.info("performLint: checking \(text.count) chars (gen=\(currentGen))")

        // BACKGROUND: Run lint engine + extract raw data (no AX calls)
        engineQueue.async { [weak self] in
            guard let self = self, let engine = self.engine else {
                self?.logger.warning("performLint: engine not initialized")
                return
            }
            let lints = engine.lint_text(text)

            var rawItems: [RawLint] = []
            let lintCount = Int(lints.count())

            for i in 0..<lintCount {
                let idx = UInt(i)
                let startOffset = Int(lints.start_offset(idx))
                let endOffset = Int(lints.end_offset(idx))

                // Harper returns Unicode scalar (character) offsets, NOT UTF-8 byte offsets.
                // Navigate using unicodeScalars view to get the correct String.Index.
                let scalars = text.unicodeScalars
                guard let startIdx = scalars.index(scalars.startIndex, offsetBy: startOffset, limitedBy: scalars.endIndex),
                      startIdx < scalars.endIndex,
                      let endIdx = scalars.index(scalars.startIndex, offsetBy: endOffset, limitedBy: scalars.endIndex),
                      endIdx <= scalars.endIndex else {
                    continue
                }
                let originalWord = String(text[startIdx..<endIdx])

                // Check session ignore list (using main-thread snapshot, no race)
                if ignoreSnapshot.contains(originalWord.lowercased()) {
                    continue
                }

                // Compute UTF-16 offsets for AX APIs (NSRange uses UTF-16 code units)
                var charStart = text[text.startIndex..<startIdx].utf16.count
                var charEnd = text[text.startIndex..<endIdx].utf16.count

                // Verify the computed range matches the expected word.
                // Rich text apps (Notes, Pages) may include invisible formatting
                // characters that shift offsets.
                let nsText = text as NSString
                let computedRange = NSRange(location: charStart, length: charEnd - charStart)
                if computedRange.location + computedRange.length <= nsText.length {
                    let textAtRange = nsText.substring(with: computedRange)
                    if textAtRange != originalWord {
                        // Offset mismatch — search for the word nearby
                        let searchStart = max(0, charStart - 200)
                        let searchLen = min(nsText.length - searchStart, computedRange.length + 400)
                        let searchRange = NSRange(location: searchStart, length: searchLen)
                        let found = nsText.range(of: originalWord, options: .literal, range: searchRange)
                        if found.location != NSNotFound {
                            charStart = found.location
                            charEnd = found.location + found.length
                        }
                    }
                }

                // Collect suggestions
                var suggestions: [String] = []
                let suggCount = Int(lints.suggestion_count(idx))
                for j in 0..<min(suggCount, Constants.maxSuggestions) {
                    suggestions.append(lints.suggestion(idx, UInt(j)).toString())
                }

                rawItems.append(RawLint(
                    errorType: lints.error_type(idx).toString(),
                    message: lints.message(idx).toString(),
                    charStart: charStart,
                    charEnd: charEnd,
                    suggestions: suggestions,
                    originalWord: originalWord
                ))
            }

            self.logger.info("performLint: engine found \(rawItems.count) issues")

            // MAIN THREAD: AX bounds queries + overlay update
            DispatchQueue.main.async { [weak self] in
                guard let self = self else { return }
                // Check generation on main thread (no race — both read/write on main)
                guard self.lintGeneration == currentGen else {
                    self.logger.debug("performLint: stale generation, discarding")
                    return
                }

                var items: [LintDisplayItem] = []
                var boundsFailures = 0
                var screenFailures = 0
                for raw in rawItems {
                    // For the visual underline, cap to the first word of multi-word spans.
                    // This prevents grammar lints (which can span phrases/sentences)
                    // from drawing a squiggly across the entire line.
                    // The full span (charStart..charEnd) is still used for correction.
                    let spanLength = raw.charEnd - raw.charStart
                    let underlineLength: Int
                    if spanLength > 20 {
                        // Find the first space in the original word to cap at word boundary
                        if let spaceIdx = raw.originalWord.firstIndex(of: " ") {
                            underlineLength = raw.originalWord[raw.originalWord.startIndex..<spaceIdx].utf16.count
                        } else {
                            underlineLength = spanLength
                        }
                    } else {
                        underlineLength = spanLength
                    }
                    let underlineRange = NSRange(location: raw.charStart, length: max(underlineLength, 1))
                    guard var axRect = self.accessibilityReader.boundsForRange(underlineRange, in: element) else {
                        boundsFailures += 1
                        continue
                    }

                    // Width sanity check: clamp to a reasonable maximum based on
                    // character count. Some apps (Notes with lists) return oversized
                    // rects that span the full line width.
                    let maxCharWidth: CGFloat = 10.0
                    let expectedMaxWidth = max(CGFloat(underlineLength) * maxCharWidth, 40.0)
                    if axRect.width > expectedMaxWidth {
                        axRect.size.width = expectedMaxWidth
                    }

                    guard let screen = OverlayPositionCalculator.screen(for: axRect) else {
                        self.logger.info("performLint: screen miss for axRect=(\(axRect.origin.x),\(axRect.origin.y),\(axRect.size.width),\(axRect.size.height))")
                        screenFailures += 1
                        continue
                    }
                    let viewRect = OverlayPositionCalculator.viewRect(fromAXRect: axRect, in: screen)

                    items.append(LintDisplayItem(
                        errorType: raw.errorType,
                        message: raw.message,
                        startOffset: raw.charStart,
                        endOffset: raw.charEnd,
                        suggestions: raw.suggestions,
                        screenRect: viewRect,
                        originalWord: raw.originalWord
                    ))
                }

                if boundsFailures > 0 || screenFailures > 0 {
                    self.logger.info("performLint: \(boundsFailures) bounds failures, \(screenFailures) screen failures")
                }
                self.logger.info("performLint: displaying \(items.count) underlines")

                self.currentResults = items
                self.onLintResults?(items)
            }
        }
    }

    private func clearResults() {
        currentResults = []
        onLintResults?([])
    }

    deinit {
        stop()
    }
}
