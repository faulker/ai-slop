package me.faulk.aetr

import kotlin.math.abs
import kotlin.math.ln
import kotlin.math.max
import kotlin.math.sqrt

/**
 * Level helpers for the RX and TX meters. Peak or RMS of a PCM block, then a
 * fast attack / slow decay so the bar is readable at the UI poll rate.
 */
object AudioLevels {

    /**
     * RMS of a mono float block. Peak alone does not say whether a signal is
     * above the receiver's sync floor, which is an RMS threshold around
     * -57 dBFS; below it nothing decodes and the failure looks identical to
     * silence.
     */
    fun rms(samples: FloatArray): Float {
        if (samples.isEmpty()) return 0f
        var sum = 0.0
        for (s in samples) sum += s.toDouble() * s
        return sqrt(sum / samples.size).toFloat()
    }

    /** dBFS for a 0..1 level; returns -inf for silence. */
    fun dbFs(level: Float): Float {
        if (level <= 1e-9f) return Float.NEGATIVE_INFINITY
        return 20f * (ln(level.toDouble()) / ln(10.0)).toFloat()
    }

    /** Absolute peak of a mono float block in 0..1 (values may exceed 1). */
    fun peakAbs(samples: FloatArray): Float {
        var peak = 0f
        for (s in samples) {
            val a = abs(s)
            if (a > peak) peak = a
        }
        return peak.coerceAtLeast(0f)
    }

    /**
     * Combines a fresh sample peak into the displayed envelope.
     * [decay] is applied when the new peak is lower (hold + fall).
     */
    fun decay(current: Float, samplePeak: Float, decay: Float = 0.86f): Float =
        max(samplePeak, current * decay).coerceIn(0f, 1f)

    /** Rough dBFS label for the meter; silence shows as −∞. */
    fun dbFsLabel(level: Float): String {
        if (level <= 1e-6f) return "−∞ dBFS"
        val db = 20f * (ln(level.toDouble()) / ln(10.0)).toFloat()
        return "%.0f dBFS".format(db)
    }
}
