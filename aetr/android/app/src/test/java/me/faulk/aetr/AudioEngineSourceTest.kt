package me.faulk.aetr

import android.media.AudioDeviceInfo
import android.media.MediaRecorder
import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

/**
 * Capture source preference and cable TX helpers for APRS/USB interfaces.
 */
class AudioEngineSourceTest {

    /** Built-in/wired path prefers the cleanest available source first. */
    @Test
    fun nonBluetoothPrefersUnprocessedThenRecognition() {
        assertEquals(
            listOf(
                MediaRecorder.AudioSource.UNPROCESSED,
                MediaRecorder.AudioSource.VOICE_RECOGNITION,
                MediaRecorder.AudioSource.MIC,
            ),
            AudioEngine.captureSources(communicationRoute = false, cable = false),
        )
    }

    /** Cable/USB skips UNPROCESSED-first (often silent on USB ADCs). */
    @Test
    fun cablePrefersMicSourceFirst() {
        assertEquals(
            MediaRecorder.AudioSource.MIC,
            AudioEngine.captureSources(communicationRoute = false, cable = true).first(),
        )
    }

    /** SCO communication route must not fall back to UNPROCESSED. */
    @Test
    fun bluetoothUsesVoiceCommunicationOnly() {
        assertEquals(
            listOf(MediaRecorder.AudioSource.VOICE_COMMUNICATION),
            AudioEngine.captureSources(communicationRoute = true),
        )
    }

    /** USB and TRRS outputs use the cable-friendly TX path. */
    @Test
    fun cableInterfaceDetectsUsbAndWired() {
        assertTrue(AudioEngine.isCableInterfaceType(AudioDeviceInfo.TYPE_USB_DEVICE))
        assertTrue(AudioEngine.isCableInterfaceType(AudioDeviceInfo.TYPE_WIRED_HEADSET))
        assertFalse(AudioEngine.isCableInterfaceType(AudioDeviceInfo.TYPE_BUILTIN_SPEAKER))
    }

    /** BOTH duplicates; LEFT/RIGHT silence the unused channel. */
    @Test
    fun stereoChannelMapping() {
        val both = AudioEngine.monoFloatToStereoPcm16(
            floatArrayOf(1f), AudioEngine.CableTxChannel.BOTH
        )
        assertEquals(32767.toShort(), both[0])
        assertEquals(32767.toShort(), both[1])

        val left = AudioEngine.monoFloatToStereoPcm16(
            floatArrayOf(1f), AudioEngine.CableTxChannel.LEFT
        )
        assertEquals(32767.toShort(), left[0])
        assertEquals(0.toShort(), left[1])

        val right = AudioEngine.monoFloatToStereoPcm16(
            floatArrayOf(1f), AudioEngine.CableTxChannel.RIGHT
        )
        assertEquals(0.toShort(), right[0])
        assertEquals(32767.toShort(), right[1])
    }

    /** Peak normalize brings the loudest sample to the target. */
    @Test
    fun peakNormalizeScalesToTarget() {
        val out = AudioEngine.peakNormalize(floatArrayOf(0.1f, -0.2f), target = 0.55f)
        assertEquals(0.275f, out[0], 1e-5f)
        assertEquals(-0.55f, out[1], 1e-5f)
    }

    /** Stereo PCM16 downmix keeps the louder channel. */
    @Test
    fun pcm16StereoToMonoFloatKeepsLouderChannel() {
        val interleaved = shortArrayOf(100, 3000, -4000, 200)
        val mono = AudioEngine.pcm16StereoToMonoFloat(interleaved, 4)
        assertEquals(2, mono.size)
        assertEquals(3000 / 32768f, mono[0], 1e-6f)
        assertEquals(-4000 / 32768f, mono[1], 1e-6f)
    }

    /** Test tone is non-silent near the cable peak target. */
    @Test
    fun testToneHasEnergy() {
        val tone = AudioEngine.testTonePcm(seconds = 0.1f)
        assertEquals(4800, tone.size)
        assertTrue(tone.maxOf { abs(it) } > 0.5f)
    }

    /** Headset assist keys used by MainActivity to block Gemini. */
    @Test
    fun headsetAssistKeysCoverHookAndAssist() {
        assertTrue(MainActivity.isHeadsetAssistKey(KeyEvent.KEYCODE_HEADSETHOOK))
        assertFalse(MainActivity.isHeadsetAssistKey(KeyEvent.KEYCODE_A))
    }
}
