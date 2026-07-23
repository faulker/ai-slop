package me.faulk.aetr

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import uniffi.aetr_core.AetrSession
import uniffi.aetr_core.ModemMode
import uniffi.aetr_core.PayloadKind
import uniffi.aetr_core.RxEvent
import uniffi.aetr_core.SessionConfig
import uniffi.aetr_core.estimateAirtimeSecs
import java.io.File

/**
 * Host-JVM digital loopback through the real core: encode a text message and
 * feed the PCM straight back into the receiver. Runs against a host-built
 * cdylib (aetr/target/release, produced by scripts/gen-bindings.sh) loaded
 * via JNA; skips cleanly when that library hasn't been built on this machine.
 */
class SessionLoopbackTest {

    /** True when a host libaetr_core is present on the JNA search path. */
    private fun hostLibraryAvailable(): Boolean {
        val dir = System.getProperty("jna.library.path") ?: return false
        return File(dir, "libaetr_core.dylib").exists() ||
            File(dir, "libaetr_core.so").exists()
    }

    @Test
    fun textRoundTripsThroughDigitalLoopback() {
        assumeTrue(
            "host libaetr_core not built; run scripts/gen-bindings.sh first",
            hostLibraryAvailable()
        )

        // Zero TX delay keeps the loopback burst free of key-up padding.
        val config = SessionConfig(
            "unit-test-passphrase", ModemMode.B170, 30u, txDelayMs = 0u, voxPrimer = false
        )
        AetrSession(config).use { session ->
            val burst = session.encodeText("hello from android")
            session.pushRx(burst)
            // A second of trailing silence flushes the demodulator.
            session.pushRx(List(48_000) { 0.0f })

            val deadline = System.currentTimeMillis() + 10_000
            var text: RxEvent.Text? = null
            while (text == null && System.currentTimeMillis() < deadline) {
                text = session.pollEvents().filterIsInstance<RxEvent.Text>().firstOrNull()
                if (text == null) Thread.sleep(50)
            }

            assertNotNull("no Text event decoded from loopback burst", text)
            assertEquals("hello from android", text!!.text)
        }
    }

    /**
     * The configured TX key-up delay must round-trip through the session and
     * lengthen both the airtime estimate and the encoded burst itself.
     */
    @Test
    fun txDelayLengthensEstimateAndBurst() {
        assumeTrue(
            "host libaetr_core not built; run scripts/gen-bindings.sh first",
            hostLibraryAvailable()
        )

        val base = estimateAirtimeSecs(ModemMode.B170, PayloadKind.TEXT, 10u, 0u)
        val delayed = estimateAirtimeSecs(ModemMode.B170, PayloadKind.TEXT, 10u, 1000u)
        assertEquals(base + 1.0, delayed, 0.05)

        val zeroDelay = SessionConfig(
            "unit-test-passphrase", ModemMode.B170, 30u, txDelayMs = 0u, voxPrimer = false
        )
        val halfSecond = SessionConfig(
            "unit-test-passphrase", ModemMode.B170, 30u, txDelayMs = 500u, voxPrimer = false
        )
        AetrSession(zeroDelay).use { plain ->
            AetrSession(halfSecond).use { padded ->
                assertEquals(0u, plain.txDelayMs())
                assertEquals(500u, padded.txDelayMs())
                // 500 ms at 48 kHz is 24 000 samples of extra lead-in; allow
                // slack for symbol-boundary rounding in the core.
                val delta =
                    padded.encodeText("delay").size - plain.encodeText("delay").size
                assertTrue(
                    "expected ~24000 extra lead-in samples, got $delta",
                    delta in 19_200..28_800
                )
            }
        }
    }
}
