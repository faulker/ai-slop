import Foundation

#if DEBUG
/// Lightweight assertions for media-key classification (run via --selftest path
/// only if wired in; kept as compile-checked documentation of the key codes).
enum MediaKeyFilterTests {
    /// Volume / mute / transport keys must be swallowed; other sysdefined ignored.
    static func run() -> Bool {
        // subtype 8, SOUND_UP key-down style data1
        let soundUp = (0 << 16) | (0x0A << 8)
        let soundDown = (1 << 16) | (0x0A << 8)
        let mute = (7 << 16) | (0x0A << 8)
        let play = (16 << 16) | (0x0A << 8)
        let brightness = (2 << 16) | (0x0A << 8)
        guard MediaKeyFilter.shouldSwallow(subtype: 8, data1: soundUp) else { return false }
        guard MediaKeyFilter.shouldSwallow(subtype: 8, data1: soundDown) else { return false }
        guard MediaKeyFilter.shouldSwallow(subtype: 8, data1: mute) else { return false }
        guard MediaKeyFilter.shouldSwallow(subtype: 8, data1: play) else { return false }
        guard !MediaKeyFilter.shouldSwallow(subtype: 8, data1: brightness) else { return false }
        guard !MediaKeyFilter.shouldSwallow(subtype: 0, data1: soundUp) else { return false }
        return checkProtectedInBackgroundInvariant()
    }

    /// `protectedInBackground` must always equal whether the tap actually
    /// exists. A prior version declared the property but never set it true on
    /// the success path, so it reported "Input Monitoring missing" no matter
    /// what was actually granted. This cannot force the true branch without
    /// mocking CGEventTapCreate (Input Monitoring may or may not be granted
    /// to whatever process runs --selftest), but it always catches the
    /// property drifting from the tap's real state, which is the exact defect
    /// that shipped.
    private static func checkProtectedInBackgroundInvariant() -> Bool {
        let filter = MediaKeyFilter()
        guard !filter.active, !filter.protectedInBackground else {
            print("MEDIAKEYFILTER FAIL: a fresh filter should start inactive")
            return false
        }
        filter.start()
        guard filter.protectedInBackground == filter.hasLiveEventTap else {
            print(
                "MEDIAKEYFILTER FAIL: protectedInBackground (\(filter.protectedInBackground)) "
                    + "does not match the real tap state (\(filter.hasLiveEventTap))")
            return false
        }
        filter.stop()
        guard !filter.protectedInBackground, !filter.hasLiveEventTap else {
            print("MEDIAKEYFILTER FAIL: stop() should clear both active and background protection")
            return false
        }
        return true
    }
}
#endif
