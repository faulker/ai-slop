import Foundation

#if DEBUG
/// Assertions for the input-level helpers and the stereo channel picker.
/// Run from the `--selftest` path so a broken build fails before hardware
/// testing, where these are much harder to spot.
enum AudioLevelsTests {
    /// Returns true when every check passes; prints the first failure.
    static func run() -> Bool {
        guard checkPeakAndRms() else { return false }
        guard checkDbFs() else { return false }
        guard checkChannelPick() else { return false }
        return true
    }

    private static func fail(_ what: String) -> Bool {
        print("AUDIOLEVELS FAIL: \(what)")
        return false
    }

    private static func checkPeakAndRms() -> Bool {
        guard AudioLevels.peakAbs([0.1, -0.75, 0.2]) == 0.75 else {
            return fail("peakAbs should follow the largest magnitude")
        }
        guard AudioLevels.peakAbs([]) == 0, AudioLevels.rms([]) == 0 else {
            return fail("empty blocks should measure zero")
        }
        // RMS of a square wave is its amplitude.
        let square = (0..<64).map { $0 % 2 == 0 ? Float(0.5) : Float(-0.5) }
        guard abs(AudioLevels.rms(square) - 0.5) < 1e-6 else {
            return fail("rms of a 0.5 square wave should be 0.5")
        }
        return true
    }

    private static func checkDbFs() -> Bool {
        guard abs(AudioLevels.dbFs(1)) < 1e-3 else { return fail("full scale should be 0 dBFS") }
        guard abs(AudioLevels.dbFs(0.5) + 6.02) < 0.01 else {
            return fail("half scale should be about -6 dBFS")
        }
        guard AudioLevels.dbFs(0) == -.infinity else { return fail("silence should be -inf") }
        // The receiver stops syncing around -57 dBFS RMS, so that boundary has
        // to land where the UI warning expects it.
        let quiet = (0..<64).map { $0 % 2 == 0 ? Float(0.0014) : Float(-0.0014) }
        let db = AudioLevels.dbFs(AudioLevels.rms(quiet))
        guard db > -60, db < -54 else { return fail("expected roughly -57 dBFS, got \(db)") }
        return true
    }

    private static func checkChannelPick() -> Bool {
        // Auto keeps the louder leg. A one-legged APRS cable must not be
        // averaged down to half amplitude against a silent partner.
        guard AudioEngine.RxChannel.auto.pick(left: 0.4, right: 0) == 0.4 else {
            return fail("auto should keep a signal on the left leg intact")
        }
        guard AudioEngine.RxChannel.auto.pick(left: 0, right: -0.4) == -0.4 else {
            return fail("auto should keep a signal on the right leg intact, sign and all")
        }
        guard AudioEngine.RxChannel.left.pick(left: 0.2, right: 0.9) == 0.2,
            AudioEngine.RxChannel.right.pick(left: 0.2, right: 0.9) == 0.9
        else {
            return fail("explicit channel selection should override level")
        }
        return true
    }
}
#endif
