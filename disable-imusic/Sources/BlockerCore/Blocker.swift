import AppKit

/// Bundle identifiers that get force-quit the moment they launch.
public let blockedBundleIDs: Set<String> = ["com.apple.Music", "com.apple.iTunes"]

/// Returns true when an app with this bundle id should be terminated.
public func shouldBlock(bundleID: String?, enabled: Bool) -> Bool {
    guard enabled, let bundleID else { return false }
    return blockedBundleIDs.contains(bundleID)
}

/// Watches app launches and force-quits anything in `blockedBundleIDs`.
/// Nothing else is touched, so media keys keep reaching whichever player is active.
@MainActor
public final class Blocker {
    private let isEnabled: () -> Bool
    private let onBlocked: () -> Void
    private var observers: [NSObjectProtocol] = []
    /// PIDs already killed, so will/did launch notifications for one launch count once.
    private var killedPIDs: Set<pid_t> = []

    /// Creates a blocker. `isEnabled` is checked on every launch; `onBlocked` runs after each kill.
    public init(isEnabled: @escaping () -> Bool, onBlocked: @escaping () -> Void) {
        self.isEnabled = isEnabled
        self.onBlocked = onBlocked
    }

    /// Starts observing launches and kills any blocked app that is already running.
    public func start() {
        let center = NSWorkspace.shared.notificationCenter
        let names = [NSWorkspace.willLaunchApplicationNotification, NSWorkspace.didLaunchApplicationNotification]
        for name in names {
            let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] note in
                let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
                MainActor.assumeIsolated { self?.handle(app) }
            }
            observers.append(token)
        }
        sweep()
    }

    /// Kills every currently running blocked app. Called on start and when blocking is re-enabled.
    public func sweep() {
        for app in NSWorkspace.shared.runningApplications {
            handle(app)
        }
    }

    /// Force-quits `app` if it is blocked and blocking is on.
    private func handle(_ app: NSRunningApplication?) {
        guard let app, shouldBlock(bundleID: app.bundleIdentifier, enabled: isEnabled()) else { return }
        let pid = app.processIdentifier
        if killedPIDs.contains(pid) { return }
        if app.forceTerminate() {
            killedPIDs.insert(pid)
            onBlocked()
        } else {
            NSLog("DisableIMusic: failed to terminate \(app.bundleIdentifier ?? "?") pid \(pid)")
        }
    }
}
