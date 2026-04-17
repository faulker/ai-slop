import Cocoa
import Carbon.HIToolbox
import os

private let log = Logger(subsystem: "com.txtmem.app", category: "HotkeyManager")

final class HotkeyManager {
    private let onQuickCapture: () -> Void
    private let onDetailedCapture: () -> Void
    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var retainedSelf: Unmanaged<HotkeyManager>?
    private var permissionTimer: Timer?
    private var permissionPollCount = 0
    private static let maxPermissionPolls = 60 // 2 minutes at 2s intervals

    init(onQuickCapture: @escaping () -> Void, onDetailedCapture: @escaping () -> Void) {
        self.onQuickCapture = onQuickCapture
        self.onDetailedCapture = onDetailedCapture
    }

    func register() {
        let mask: CGEventMask = (1 << CGEventType.keyDown.rawValue)

        let callback: CGEventTapCallBack = { proxy, type, event, refcon in
            guard let refcon = refcon else { return Unmanaged.passRetained(event) }
            let manager = Unmanaged<HotkeyManager>.fromOpaque(refcon).takeUnretainedValue()
            return manager.handleEvent(proxy: proxy, type: type, event: event)
        }

        // Use passRetained so the event tap callback always has a valid reference
        let retained = Unmanaged.passRetained(self)
        let refcon = retained.toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: mask,
            callback: callback,
            userInfo: refcon
        ) else {
            // Balance the retain since we won't store the tap
            retained.release()
            log.error("Failed to create event tap — Accessibility permission not yet granted")
            NSLog("[ThoughtQueue] Event tap failed — waiting for accessibility permission...")
            startPollingForPermission()
            return
        }

        retainedSelf = retained
        eventTap = tap
        let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        runLoopSource = source
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        NSLog("[ThoughtQueue] Event tap registered successfully — hotkeys active")
    }

    private func startPollingForPermission() {
        guard permissionTimer == nil else { return }
        permissionPollCount = 0
        permissionTimer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak self] timer in
            guard let self = self else { timer.invalidate(); return }
            self.permissionPollCount += 1

            if AXIsProcessTrusted() {
                NSLog("[ThoughtQueue] Accessibility permission granted, registering hotkeys...")
                timer.invalidate()
                self.permissionTimer = nil
                self.register()
            } else if self.permissionPollCount >= Self.maxPermissionPolls {
                NSLog("[ThoughtQueue] Accessibility permission not granted after %d attempts, stopping poll", Self.maxPermissionPolls)
                timer.invalidate()
                self.permissionTimer = nil
                DispatchQueue.main.async {
                    self.showPermissionAlert()
                }
            }
        }
    }

    private func showPermissionAlert() {
        let alert = NSAlert()
        alert.messageText = "Accessibility Permission Required"
        alert.informativeText = "ThoughtQueue needs Accessibility permission to capture global hotkeys. Please grant it in System Settings > Privacy & Security > Accessibility, then relaunch the app."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Open System Settings")
        alert.addButton(withTitle: "Dismiss")

        if alert.runModal() == .alertFirstButtonReturn {
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
                NSWorkspace.shared.open(url)
            }
        }
    }

    private func handleEvent(proxy: CGEventTapProxy, type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            log.warning("Event tap was disabled (timeout/user), re-enabling")
            if let tap = eventTap {
                CGEvent.tapEnable(tap: tap, enable: true)
            }
            return Unmanaged.passRetained(event)
        }

        guard type == .keyDown else { return Unmanaged.passRetained(event) }

        let flags = event.flags
        let keyCode = event.getIntegerValueField(.keyboardEventKeycode)

        let quickKey = PreferencesManager.shared.quickCaptureKey
        let detailedKey = PreferencesManager.shared.detailedCaptureKey

        if keyCode == quickKey.keyCode && flags.contains(quickKey.modifiers) && !hasExtraModifiers(flags, expected: quickKey.modifiers) {
            log.info("Quick capture hotkey detected")
            DispatchQueue.main.async { [weak self] in self?.onQuickCapture() }
            return nil
        }

        if keyCode == detailedKey.keyCode && flags.contains(detailedKey.modifiers) && !hasExtraModifiers(flags, expected: detailedKey.modifiers) {
            log.info("Detailed capture hotkey detected")
            DispatchQueue.main.async { [weak self] in self?.onDetailedCapture() }
            return nil
        }

        return Unmanaged.passRetained(event)
    }

    private func hasExtraModifiers(_ flags: CGEventFlags, expected: CGEventFlags) -> Bool {
        let relevantMask: CGEventFlags = [.maskShift, .maskControl, .maskAlternate, .maskCommand]
        let actual = flags.intersection(relevantMask)
        let exp = expected.intersection(relevantMask)
        return actual != exp
    }

    func unregister() {
        permissionTimer?.invalidate()
        permissionTimer = nil
        if let source = runLoopSource {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, .commonModes)
            runLoopSource = nil
        }
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: false)
            retainedSelf?.release()
            retainedSelf = nil
            eventTap = nil
            log.info("Event tap unregistered")
        }
    }

    deinit {
        unregister()
    }
}
