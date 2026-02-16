import AppKit
import ApplicationServices

protocol FocusTrackerDelegate: AnyObject {
    func focusTrackerDidChangeApp()
    func focusTrackerDidChangeElement()
}

/// Tracks which application is currently focused.
/// Uses NSWorkspace notifications to detect app switches.
final class FocusTracker {

    weak var delegate: FocusTrackerDelegate?

    private(set) var currentBundleID: String?
    private var observer: NSObjectProtocol?
    private var axObserver: AXObserver?
    private let logger = Logger(category: "FocusTracker")

    // MARK: - Public

    /// Starts observing focus changes.
    func startTracking() {
        observer = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification,
            object: nil,
            queue: .main
        ) { [weak self] notification in
            self?.handleAppActivation(notification)
        }

        setupAXObserver()

        // Set initial app
        currentBundleID = NSWorkspace.shared.frontmostApplication?.bundleIdentifier
        logger.info("Focus tracking started")
    }

    private func teardownAXObserver() {
        if let obs = axObserver {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(obs), .commonModes)
            axObserver = nil
        }
    }

    private func setupAXObserver() {
        // Clean up previous observer to prevent leaks
        teardownAXObserver()

        // Observe focused element changes globally
        guard let app = NSWorkspace.shared.frontmostApplication else { return }
        let pid = app.processIdentifier

        var observer: AXObserver?
        guard AXObserverCreate(pid, { (observer, element, notification, refcon) in
            guard let refcon = refcon else { return }
            let tracker = Unmanaged<FocusTracker>.fromOpaque(refcon).takeUnretainedValue()
            tracker.delegate?.focusTrackerDidChangeElement()
        }, &observer) == .success, let obs = observer else { return }

        let axApp = AXUIElementCreateApplication(pid)
        AXObserverAddNotification(obs, axApp, kAXFocusedUIElementChangedNotification as CFString, Unmanaged.passUnretained(self).toOpaque())
        CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(obs), .commonModes)
        self.axObserver = obs
    }

    /// Internal method for handling notifications, allows testing.
    func handleAppActivation(_ notification: Notification) {
        let newBundleID = (notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)?.bundleIdentifier
        if newBundleID != self.currentBundleID {
            self.currentBundleID = newBundleID
            self.logger.debug("App changed: \(newBundleID ?? "unknown")")

            // Re-setup AX observer for the new app
            setupAXObserver()

            self.delegate?.focusTrackerDidChangeApp()
        }
    }

    #if DEBUG
    func setCurrentBundleIDForTesting(_ id: String?) {
        self.currentBundleID = id
    }
    #endif

    /// Stops observing focus changes.
    func stopTracking() {
        if let obs = observer {
            NSWorkspace.shared.notificationCenter.removeObserver(obs)
            observer = nil
        }
        teardownAXObserver()
        logger.info("Focus tracking stopped")
    }

    deinit {
        stopTracking()
    }
}
