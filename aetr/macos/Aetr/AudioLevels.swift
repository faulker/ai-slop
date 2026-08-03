import Foundation

/// Level helpers for the RX meter. Peak or RMS of a PCM block, then a fast
/// attack / slow decay so the bar is readable at the UI poll rate.
enum AudioLevels {
    /// Absolute peak of a mono float block (clamped to ≥ 0).
    static func peakAbs(_ samples: [Float]) -> Float {
        var peak: Float = 0
        for s in samples {
            let a = abs(s)
            if a > peak { peak = a }
        }
        return max(0, peak)
    }

    /// RMS of a mono float block.
    ///
    /// Peak alone does not say whether a signal clears the receiver's sync
    /// floor, which is an RMS threshold near -57 dBFS: below it nothing
    /// decodes and every counter reads zero, exactly like a dead route.
    static func rms(_ samples: [Float]) -> Float {
        guard !samples.isEmpty else { return 0 }
        var sum = 0.0
        for s in samples { sum += Double(s) * Double(s) }
        return Float((sum / Double(samples.count)).squareRoot())
    }

    /// dBFS for a 0...1 level; -infinity for silence.
    static func dbFs(_ level: Float) -> Float {
        guard level > 1e-9 else { return -.infinity }
        return 20 * log10(level)
    }

    /// Combines a fresh sample peak into the displayed envelope.
    static func decay(current: Float, samplePeak: Float, decay: Float = 0.86) -> Float {
        min(1, max(samplePeak, current * decay))
    }

    /// Rough dBFS label for the meter; silence shows as −∞.
    static func dbFsLabel(_ level: Float) -> String {
        if level <= 1e-6 { return "−∞ dBFS" }
        let db = 20 * log10(Double(level))
        return String(format: "%.0f dBFS", db)
    }
}
