package me.faulk.aetr

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the RX input-level meter helpers. */
class AudioLevelsTest {

    /** Peak follows the largest absolute sample. */
    @Test
    fun peakAbsFindsLargestMagnitude() {
        assertEquals(0.75f, AudioLevels.peakAbs(floatArrayOf(0.1f, -0.75f, 0.2f)), 1e-6f)
        assertEquals(0f, AudioLevels.peakAbs(floatArrayOf()), 0f)
    }

    /** New peaks attack immediately; quieter blocks decay. */
    @Test
    fun decayAttacksAndFalls() {
        assertEquals(0.9f, AudioLevels.decay(0.1f, 0.9f), 1e-6f)
        val fallen = AudioLevels.decay(1.0f, 0f, decay = 0.5f)
        assertEquals(0.5f, fallen, 1e-6f)
    }

    /** Silence label and a known mid-level dB string. */
    @Test
    fun dbFsLabelFormats() {
        assertEquals("−∞ dBFS", AudioLevels.dbFsLabel(0f))
        assertTrue(AudioLevels.dbFsLabel(1f).contains("0"))
    }

    /** RMS of a square wave is its amplitude; of silence, zero. */
    @Test
    fun rmsMeasuresAveragePower() {
        val square = FloatArray(64) { i -> if (i % 2 == 0) 0.5f else -0.5f }
        assertEquals(0.5f, AudioLevels.rms(square), 1e-6f)
        assertEquals(0f, AudioLevels.rms(FloatArray(32)), 1e-6f)
        assertEquals(0f, AudioLevels.rms(floatArrayOf()), 0f)
    }

    /**
     * RMS has to separate a quiet burst from silence, because the receiver's
     * sync floor sits near -57 dBFS RMS and below it the two are
     * indistinguishable from the decoder's point of view.
     */
    @Test
    fun rmsDistinguishesQuietFromSilent() {
        val quiet = FloatArray(64) { i -> if (i % 2 == 0) 0.0014f else -0.0014f }
        val db = AudioLevels.dbFs(AudioLevels.rms(quiet))
        assertTrue("expected roughly -57 dBFS, got $db", db > -60f && db < -54f)
        assertEquals(Float.NEGATIVE_INFINITY, AudioLevels.dbFs(0f), 0f)
    }

    /** Full scale is 0 dBFS; half scale is about -6. */
    @Test
    fun dbFsConvertsLinearLevels() {
        assertEquals(0f, AudioLevels.dbFs(1f), 1e-3f)
        assertEquals(-6.02f, AudioLevels.dbFs(0.5f), 0.01f)
    }
}
