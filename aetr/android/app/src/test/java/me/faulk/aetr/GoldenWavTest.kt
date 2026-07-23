package me.faulk.aetr

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assume.assumeTrue
import org.junit.Test
import uniffi.aetr_core.AetrSession
import uniffi.aetr_core.ModemMode
import uniffi.aetr_core.RxEvent
import uniffi.aetr_core.SessionConfig
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Cross-platform golden vector check: decodes testdata/golden_text.wav
 * (written by the core test golden_text_wav_cross_platform) through the real
 * core on the host JVM via JNA and verifies the expected plaintext, proving
 * the Kotlin binding path decodes byte-identically to Rust and macOS.
 * Constants stay in sync with core/src/tests.rs. Skips cleanly when the
 * host cdylib (scripts/gen-bindings.sh) or the wav isn't present.
 */
class GoldenWavTest {

    private val goldenPassphrase = "correct horse battery staple"
    private val goldenText =
        "aetr golden vector: the quick brown fox jumps over the lazy dog 0123456789"

    /** True when a host libaetr_core is present on the JNA search path. */
    private fun hostLibraryAvailable(): Boolean {
        val dir = System.getProperty("jna.library.path") ?: return false
        return File(dir, "libaetr_core.dylib").exists() ||
            File(dir, "libaetr_core.so").exists()
    }

    /** Walks up from the test working dir to find testdata/golden_text.wav. */
    private fun findGoldenWav(): File? {
        var dir: File? = File("").absoluteFile
        repeat(6) {
            val candidate = File(dir, "testdata/golden_text.wav")
            if (candidate.isFile) return candidate
            dir = dir?.parentFile ?: return null
        }
        return null
    }

    /** Reads a canonical 48 kHz mono 16-bit PCM WAV into f32 samples. */
    private fun readWav16Mono48k(file: File): List<Float> {
        val bytes = file.readBytes()
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        require(bytes.size > 44) { "wav too short" }
        require(String(bytes, 0, 4) == "RIFF" && String(bytes, 8, 4) == "WAVE") {
            "not a RIFF/WAVE file"
        }
        var pos = 12
        var data: Pair<Int, Int>? = null
        while (pos + 8 <= bytes.size) {
            val id = String(bytes, pos, 4)
            val len = buf.getInt(pos + 4)
            when (id) {
                "fmt " -> {
                    require(buf.getShort(pos + 8).toInt() == 1) { "not PCM" }
                    require(buf.getShort(pos + 10).toInt() == 1) { "not mono" }
                    require(buf.getInt(pos + 12) == 48_000) { "not 48 kHz" }
                    require(buf.getShort(pos + 22).toInt() == 16) { "not 16-bit" }
                }
                "data" -> data = Pair(pos + 8, len)
            }
            pos += 8 + len + (len and 1) // chunks are word-aligned
        }
        val (offset, length) = requireNotNull(data) { "no data chunk" }
        return (0 until length / 2).map { i ->
            buf.getShort(offset + 2 * i).toFloat() / 32768.0f
        }
    }

    @Test
    fun goldenTextWavDecodes() {
        assumeTrue(
            "host libaetr_core not built; run scripts/gen-bindings.sh first",
            hostLibraryAvailable()
        )
        val wav = findGoldenWav()
        assumeTrue(
            "testdata/golden_text.wav not found; run cargo test -p aetr-core --release first",
            wav != null
        )

        val pcm = readWav16Mono48k(wav!!)
        // Zero TX delay: the golden wav predates the key-up padding and the
        // receive path must not depend on it either way.
        val config = SessionConfig(
            goldenPassphrase, ModemMode.B170, 30u, txDelayMs = 0u, voxPrimer = false
        )
        AetrSession(config).use { session ->
            session.pushRx(pcm)
            // A second of trailing silence flushes the demodulator.
            session.pushRx(List(48_000) { 0.0f })

            val deadline = System.currentTimeMillis() + 10_000
            var text: RxEvent.Text? = null
            while (text == null && System.currentTimeMillis() < deadline) {
                text = session.pollEvents().filterIsInstance<RxEvent.Text>().firstOrNull()
                if (text == null) Thread.sleep(50)
            }

            assertNotNull("no Text event decoded from golden wav", text)
            assertEquals(goldenText, text!!.text)
        }
    }
}
