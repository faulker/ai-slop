import AppKit
import CoreGraphics

/// Blocks spurious headset volume / media keys from APRS TRRS and USB-C
/// audio cables. Those cables often float the remote-button line; macOS
/// treats the noise as VOLUME UP/DOWN and shows the on-screen volume HUD.
///
/// Uses a local NSEvent monitor (focused windows) plus a session CGEvent
/// tap when permitted, so keys are eaten even when another window is focused.
final class MediaKeyFilter {
    /// NX_KEYTYPE_* codes carried in NSSystemDefined media-key events.
    private enum KeyType: Int {
        case soundUp = 0
        case soundDown = 1
        case mute = 7
        case play = 16
        case next = 17
        case previous = 18
        case fast = 19
        case rewind = 20
    }

    /// Subtype for auxiliary / media control buttons on NSSystemDefined events.
    private static let auxControlSubtype: Int16 = 8

    /// CGEvent type for NSSystemDefined (not in the public CGEventType enum).
    private static let cgSystemDefined = CGEventType(rawValue: 14)!

    private var localMonitor: Any?
    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?

    /// True while filters are installed.
    private(set) var active = false

    /// True once the session-wide tap is up, meaning phantom media keys are
    /// swallowed even while Aetr is in the background. False means only the
    /// local monitor is protecting you (focused-window only), because Input
    /// Monitoring was never granted.
    ///
    /// Must always equal `eventTap != nil` (see `hasLiveEventTap`): this is
    /// the invariant a prior version of this file broke by declaring the
    /// property but never setting it true on the success path, which reported
    /// "permission missing" unconditionally regardless of the real OS state.
    private(set) var protectedInBackground = false

    /// Test-only view into whether the tap actually exists, so
    /// MediaKeyFilterTests can catch `protectedInBackground` drifting from
    /// reality without needing to mock CGEventTapCreate.
    var hasLiveEventTap: Bool { eventTap != nil }

    deinit {
        stop()
    }

    /// Starts swallowing volume and headset media keys. Returns false when
    /// Input Monitoring is not granted: the local monitor still helps while
    /// an Aetr window is focused, but a phantom key while backgrounded (the
    /// case that actually matters, since it launches Music unfiltered) will
    /// not be caught until the permission is granted and Aetr is restarted.
    @discardableResult
    func start() -> Bool {
        if active { return protectedInBackground }
        active = true
        localMonitor = NSEvent.addLocalMonitorForEvents(matching: .systemDefined) { event in
            Self.shouldSwallow(event) ? nil : event
        }
        // Triggers the Input Monitoring system prompt on first launch if the
        // user has never been asked. Without this call, macOS may leave the
        // permission in an undetermined state forever and the tap below
        // fails every time with nothing ever showing in System Settings.
        _ = CGRequestListenEventAccess()
        startEventTap()
        return protectedInBackground
    }

    /// Removes monitors and taps.
    func stop() {
        active = false
        protectedInBackground = false
        if let localMonitor {
            NSEvent.removeMonitor(localMonitor)
            self.localMonitor = nil
        }
        stopEventTap()
    }

    /// Returns true for volume / mute / transport keys from a headset remote.
    static func shouldSwallow(subtype: Int16, data1: Int) -> Bool {
        guard subtype == auxControlSubtype else { return false }
        let keyCode = (data1 & 0xFFFF_0000) >> 16
        switch keyCode {
        case KeyType.soundUp.rawValue,
            KeyType.soundDown.rawValue,
            KeyType.mute.rawValue,
            KeyType.play.rawValue,
            KeyType.next.rawValue,
            KeyType.previous.rawValue,
            KeyType.fast.rawValue,
            KeyType.rewind.rawValue:
            return true
        default:
            return false
        }
    }

    /// NSEvent convenience wrapper around [shouldSwallow(subtype:data1:)].
    static func shouldSwallow(_ event: NSEvent) -> Bool {
        guard event.type == .systemDefined else { return false }
        return shouldSwallow(subtype: event.subtype.rawValue, data1: event.data1)
    }

    /// Session-wide tap so volume keys are eaten even when Aetr is not focused.
    private func startEventTap() {
        let mask = CGEventMask(1 << Self.cgSystemDefined.rawValue)
        let callback: CGEventTapCallBack = { _, type, event, refcon in
            guard type != .tapDisabledByTimeout, type != .tapDisabledByUserInput else {
                if let refcon {
                    let filter = Unmanaged<MediaKeyFilter>.fromOpaque(refcon).takeUnretainedValue()
                    if let tap = filter.eventTap {
                        CGEvent.tapEnable(tap: tap, enable: true)
                    }
                }
                return Unmanaged.passUnretained(event)
            }
            if let nsEvent = NSEvent(cgEvent: event), MediaKeyFilter.shouldSwallow(nsEvent) {
                return nil
            }
            return Unmanaged.passUnretained(event)
        }
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: mask,
            callback: callback,
            userInfo: Unmanaged.passUnretained(self).toOpaque()
        ) else {
            // Input Monitoring / Accessibility not granted: local monitor still helps.
            return
        }
        eventTap = tap
        runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        if let runLoopSource {
            CFRunLoopAddSource(CFRunLoopGetMain(), runLoopSource, .commonModes)
        }
        CGEvent.tapEnable(tap: tap, enable: true)
        protectedInBackground = true
    }

    private func stopEventTap() {
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: false)
        }
        if let runLoopSource {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), runLoopSource, .commonModes)
        }
        runLoopSource = nil
        eventTap = nil
    }
}
