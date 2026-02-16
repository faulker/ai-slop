import AppKit

/// Manages a transparent, floating overlay window that displays squiggly underlines
/// over misspelled words in any application.
final class OverlayWindowController: NSWindowController {

    private let contentView = OverlayContentView()

    /// Called when the user clicks an underline. Delivers the result index and screen rect.
    var onUnderlineClicked: ((Int, CGRect) -> Void)?

    private var clickMonitor: Any?

    // MARK: - Initialization

    convenience init() {
        let window = NSWindow(
            contentRect: .zero,
            styleMask: .borderless,
            backing: .buffered,
            defer: true
        )

        // Transparent, floating overlay that passes all clicks through
        window.level = .floating
        window.backgroundColor = .clear
        window.isOpaque = false
        window.hasShadow = false
        window.ignoresMouseEvents = true
        window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]

        self.init(window: window)

        contentView.frame = window.contentView?.bounds ?? .zero
        contentView.autoresizingMask = [.width, .height]
        window.contentView = contentView
    }

    /// The overlay content view, exposed for popup menu anchoring.
    var popupAnchorView: NSView { contentView }

    // MARK: - Public

    /// Updates the underlines displayed on the overlay.
    func updateUnderlines(_ underlines: [SquigglyUnderlineView.Underline]) {
        // Position window on the screen containing the results
        if let firstRect = underlines.first?.rect,
           let targetScreen = NSScreen.screens.first(where: { $0.frame.contains(firstRect.origin) }) ?? NSScreen.main {
            window?.setFrame(targetScreen.frame, display: true)
        }

        contentView.squigglyView.underlines = underlines
        contentView.squigglyView.needsDisplay = true
    }

    /// Clears all underlines from the overlay.
    func clearUnderlines() {
        contentView.squigglyView.underlines = []
        contentView.squigglyView.needsDisplay = true
    }

    /// Shows the overlay window and installs the global click monitor.
    func showOverlay() {
        showWindow(nil)
        window?.orderFrontRegardless()
        installClickMonitor()
    }

    /// Hides the overlay window and removes the click monitor.
    func hideOverlay() {
        window?.orderOut(nil)
        removeClickMonitor()
    }

    // MARK: - Global Click Monitor

    /// Installs a global event monitor that detects clicks on underline positions.
    /// Since the overlay window has ignoresMouseEvents=true, clicks pass through to
    /// underlying apps. This monitor checks if a click landed on an underline and
    /// triggers the correction popup if so.
    private func installClickMonitor() {
        guard clickMonitor == nil else { return }
        clickMonitor = NSEvent.addGlobalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
            self?.handleGlobalClick(event)
        }
    }

    private func removeClickMonitor() {
        if let monitor = clickMonitor {
            NSEvent.removeMonitor(monitor)
            clickMonitor = nil
        }
    }

    private func handleGlobalClick(_ event: NSEvent) {
        // Convert click location (bottom-left screen coords) to view coords
        guard let window = self.window, window.isVisible else { return }

        let screenPoint = event.locationInWindow  // For global events, this is in screen coords
        let viewPoint = contentView.convert(screenPoint, from: nil)

        // Check if click hit any underline (with padding)
        if let underline = contentView.squigglyView.underline(at: viewPoint) {
            onUnderlineClicked?(underline.resultIndex, underline.rect)
        }
    }

    deinit {
        removeClickMonitor()
    }
}
