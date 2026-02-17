import AppKit
import ApplicationServices

protocol FocusTrackerDelegate: AnyObject {
    func focusTrackerDidChangeApp()
    func focusTrackerDidChangeElement()
    func focusTrackerDidDetectWindowMove()
}

/// Tracks which application is currently focused.
/// Uses NSWorkspace notifications to detect app switches.
/// Observes AX notifications for focus element changes and window move/resize.
final class FocusTracker {

    weak var delegate: FocusTrackerDelegate?

    private(set) var currentBundleID: String?
    private var observer: NSObjectProtocol?
    private var axObserver: AXObserver?
    private var observedWindow: AXUIElement?
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
        // Remove window notifications before tearing down observer
        if let obs = axObserver, let window = observedWindow {
            AXObserverRemoveNotification(obs, window, kAXMovedNotification as CFString)
            AXObserverRemoveNotification(obs, window, kAXResizedNotification as CFString)
            observedWindow = nil
        }

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
            let notifString = notification as String

            if notifString == kAXMovedNotification || notifString == kAXResizedNotification {
                tracker.delegate?.focusTrackerDidDetectWindowMove()
            } else {
                // On focus element change, update window observers (user may have switched windows)
                tracker.updateWindowObservers()
                tracker.delegate?.focusTrackerDidChangeElement()
            }
        }, &observer) == .success, let obs = observer else { return }

        let axApp = AXUIElementCreateApplication(pid)
        let refcon = Unmanaged.passUnretained(self).toOpaque()

        // Observe focus element changes on the app
        AXObserverAddNotification(obs, axApp, kAXFocusedUIElementChangedNotification as CFString, refcon)

        // Observe window move/resize on the focused window
        var windowValue: AnyObject?
        if AXUIElementCopyAttributeValue(axApp, kAXFocusedWindowAttribute as CFString, &windowValue) == .success,
           let window = windowValue {
            // AXUIElement is a CF type — downcast always succeeds
            let windowElement = window as! AXUIElement // swiftlint:disable:this force_cast
            AXObserverAddNotification(obs, windowElement, kAXMovedNotification as CFString, refcon)
            AXObserverAddNotification(obs, windowElement, kAXResizedNotification as CFString, refcon)
            self.observedWindow = windowElement
        }

        CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(obs), .commonModes)
        self.axObserver = obs
    }

    /// Updates window move/resize observers to track the currently focused window.
    /// Called when focus element changes within the same app (e.g. switching between windows).
    private func updateWindowObservers() {
        guard let obs = axObserver else { return }
        guard let app = NSWorkspace.shared.frontmostApplication else { return }
        let refcon = Unmanaged.passUnretained(self).toOpaque()

        // Remove old window notifications
        if let window = observedWindow {
            AXObserverRemoveNotification(obs, window, kAXMovedNotification as CFString)
            AXObserverRemoveNotification(obs, window, kAXResizedNotification as CFString)
            observedWindow = nil
        }

        // Observe the new focused window
        let axApp = AXUIElementCreateApplication(app.processIdentifier)
        var windowValue: AnyObject?
        if AXUIElementCopyAttributeValue(axApp, kAXFocusedWindowAttribute as CFString, &windowValue) == .success,
           let window = windowValue {
            let windowElement = window as! AXUIElement // swiftlint:disable:this force_cast
            AXObserverAddNotification(obs, windowElement, kAXMovedNotification as CFString, refcon)
            AXObserverAddNotification(obs, windowElement, kAXResizedNotification as CFString, refcon)
            observedWindow = windowElement
        }
    }

    /// Internal method for handling notifications, allows testing.
    func handleAppActivation(_ notification: Notification) {
        let newBundleID = (notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)?.bundleIdentifier
        if newBundleID != self.currentBundleID {
            self.currentBundleID = newBundleID
            self.logger.debug("App changed: \(newBundleID ?? "unknown")")

            // Re-setup AX observer for the new app (tears down old window observers)
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
