import AppKit
import ApplicationServices

protocol EventTapDelegate: AnyObject {
    func eventTapDidReceiveKeystroke()
}

/// Installs a CGEventTap to monitor keyboard input system-wide.
/// The tap signals the debouncer on every keyDown but returns events immediately (no blocking).
final class EventTapManager {

    weak var delegate: EventTapDelegate?

    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private let logger = Logger(category: "EventTapManager")

    // MARK: - Public

    /// Installs the CGEventTap. Returns false if installation fails.
    @discardableResult
    func install() -> Bool {
        // Don't double-install
        if eventTap != nil {
            return true
        }

        let mask: CGEventMask = (1 << CGEventType.keyDown.rawValue)

        let selfPtr = Unmanaged.passUnretained(self).toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .listenOnly,
            eventsOfInterest: mask,
            callback: EventTapManager.eventTapCallback,
            userInfo: selfPtr
        ) else {
            let trusted = AXIsProcessTrusted()
            logger.warning("Failed to create CGEventTap (AXIsProcessTrusted=\(trusted)) — check Accessibility permission")
            return false
        }

        eventTap = tap
        runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)

        if let source = runLoopSource {
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
        }

        CGEvent.tapEnable(tap: tap, enable: true)
        logger.info("CGEventTap installed")
        return true
    }

    /// Removes the event tap and cleans up.
    func uninstall() {
        if let source = runLoopSource {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, .commonModes)
        }
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: false)
        }
        eventTap = nil
        runLoopSource = nil
        logger.info("CGEventTap uninstalled")
    }

    // MARK: - Event Tap Callback

    private static let eventTapCallback: CGEventTapCallBack = { proxy, type, event, userInfo in
        guard let userInfo = userInfo else {
            return Unmanaged.passUnretained(event)
        }

        let manager = Unmanaged<EventTapManager>.fromOpaque(userInfo).takeUnretainedValue()

        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            // Re-enable the tap
            if let tap = manager.eventTap {
                CGEvent.tapEnable(tap: tap, enable: true)
            }
            return Unmanaged.passUnretained(event)
        }

        #if DEBUG
        let startTime = DispatchTime.now()
        #endif

        // Signal the delegate immediately (already on main run loop)
        manager.delegate?.eventTapDidReceiveKeystroke()

        #if DEBUG
        let endTime = DispatchTime.now()
        let nanoTime = endTime.uptimeNanoseconds - startTime.uptimeNanoseconds
        let milliTime = Double(nanoTime) / 1_000_000

        if milliTime > 1.0 {
            manager.logger.warning("Event tap callback took \(milliTime)ms (target < 1ms)")
        }
        #endif

        return Unmanaged.passUnretained(event)
    }

    deinit {
        uninstall()
    }
}
