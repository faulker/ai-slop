package me.faulk.aetr

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.os.Build
import android.os.SystemClock
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Owns both halves of the audio path at the rates the core expects:
 * a 48 kHz mono float AudioRecord capture loop that hands every PCM block to
 * a callback (which the ViewModel feeds into `AetrSession.pushRx`), and a
 * persistent 48 kHz AudioTrack for TX bursts and received voice clips.
 *
 * Cable/USB radio interfaces (APRS TRRS, USB sound cards) get PCM 16-bit
 * (stereo L=R, falling back to mono), peak-normalized to [txPeakTarget]
 * because phone headphone levels into a radio mic are usually too quiet.
 *
 * The same track is reused across bursts. Reopening one per burst glitches
 * USB/TRRS links and can look like headset-button presses (Gemini/Assistant),
 * so the only thing that reopens it is the communication-route fallback in
 * `playBlocking`, and that only fires when the sink verifiably came up wrong.
 */
class AudioEngine(
    context: Context,
    private val router: AudioRouter? = null,
) {

    /** Which headphone channel(s) carry TX into the radio mic. */
    enum class CableTxChannel {
        /** Duplicate mono on L and R. */
        BOTH,
        /** Left only (right silent). */
        LEFT,
        /** Right only (left silent). */
        RIGHT,
    }

    companion object {
        const val SAMPLE_RATE = 48_000

        /** Frames per capture read; ~21 ms at 48 kHz. */
        private const val CAPTURE_BLOCK = 1024

        /**
         * Peak target after normalizing cable/USB TX. Near-full-scale (0.95)
         * overdrives many radio mics so the far end sees energy but cannot
         * decode; keep this moderate.
         */
        const val CABLE_TX_PEAK = 0.55f

        /**
         * Ordered AudioSource candidates for capture. Bluetooth SCO uses
         * VOICE_COMMUNICATION only; built-in/wired paths prefer UNPROCESSED
         * so platform AEC/AGC/NS do not chew the modem. Cable/USB skips
         * UNPROCESSED (often silent on USB ADCs) and starts at MIC.
         */
        fun captureSources(communicationRoute: Boolean, cable: Boolean = false): List<Int> =
            when {
                communicationRoute -> listOf(MediaRecorder.AudioSource.VOICE_COMMUNICATION)
                cable -> listOf(
                    MediaRecorder.AudioSource.MIC,
                    MediaRecorder.AudioSource.VOICE_RECOGNITION,
                    MediaRecorder.AudioSource.UNPROCESSED,
                )
                else -> listOf(
                    MediaRecorder.AudioSource.UNPROCESSED,
                    MediaRecorder.AudioSource.VOICE_RECOGNITION,
                    MediaRecorder.AudioSource.MIC,
                )
            }

        /** Converts a PCM16 mono buffer to float -1..1. */
        fun pcm16ToFloat(samples: ShortArray, count: Int = samples.size): FloatArray {
            val n = count.coerceIn(0, samples.size)
            return FloatArray(n) { i -> samples[i] / 32768f }
        }

        /**
         * Downmixes interleaved stereo PCM16 to mono float, keeping the louder
         * channel per frame (APRS cables often wire audio on only one side).
         */
        fun pcm16StereoToMonoFloat(interleaved: ShortArray, shortCount: Int): FloatArray {
            val frames = (shortCount.coerceIn(0, interleaved.size) / 2)
            return FloatArray(frames) { i ->
                val l = interleaved[i * 2] / 32768f
                val r = interleaved[i * 2 + 1] / 32768f
                if (kotlin.math.abs(l) >= kotlin.math.abs(r)) l else r
            }
        }

        /** 1 kHz test tone near full scale for cable TX path checks. */
        fun testTonePcm(seconds: Float = 1.5f, hz: Float = 1000f): FloatArray {
            val n = (SAMPLE_RATE * seconds).toInt().coerceAtLeast(1)
            val twoPiF = (2.0 * Math.PI * hz / SAMPLE_RATE).toFloat()
            return FloatArray(n) { i -> CABLE_TX_PEAK * kotlin.math.sin(twoPiF * i) }
        }

        /**
         * Scales a mono burst so its peak is exactly [target].
         *
         * Note this normalizes rather than boosts: the core's burst already
         * peaks around 0.49, so at the default target this is about +1 dB, and
         * raising [target] above the burst's own peak is the only way it gains
         * anything. Phone headphone outs into radio mics are weak, so the
         * useful adjustment is usually upward.
         */
        fun peakNormalize(mono: FloatArray, target: Float = CABLE_TX_PEAK): FloatArray {
            var peak = 0f
            for (s in mono) {
                val a = kotlin.math.abs(s)
                if (a > peak) peak = a
            }
            if (peak < 1e-6f) return mono
            val g = target / peak
            return FloatArray(mono.size) { i -> (mono[i] * g).coerceIn(-1f, 1f) }
        }

        /** True for TRRS/USB radio interfaces that need cable-friendly TX. */
        fun isCableInterface(device: AudioDeviceInfo?): Boolean =
            device != null && isCableInterfaceType(device.type)

        /** Type-constant form of [isCableInterface] for tests and callers. */
        fun isCableInterfaceType(type: Int): Boolean = when (type) {
            AudioDeviceInfo.TYPE_WIRED_HEADSET,
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
            AudioDeviceInfo.TYPE_USB_DEVICE,
            AudioDeviceInfo.TYPE_USB_HEADSET,
            AudioDeviceInfo.TYPE_USB_ACCESSORY,
            -> true
            else -> false
        }

        /**
         * Converts mono float to interleaved stereo PCM16.
         * Many APRS TRRS cables wire the radio mic to only tip or only ring —
         * try [CableTxChannel.LEFT] / [RIGHT] if BOTH fails.
         */
        fun monoFloatToStereoPcm16(
            mono: FloatArray,
            channel: CableTxChannel = CableTxChannel.BOTH,
        ): ShortArray {
            val out = ShortArray(mono.size * 2)
            var o = 0
            for (s in mono) {
                val v = (s.coerceIn(-1f, 1f) * 32767f).toInt().toShort()
                val zero: Short = 0
                when (channel) {
                    CableTxChannel.BOTH -> {
                        out[o++] = v
                        out[o++] = v
                    }
                    CableTxChannel.LEFT -> {
                        out[o++] = v
                        out[o++] = zero
                    }
                    CableTxChannel.RIGHT -> {
                        out[o++] = zero
                        out[o++] = v
                    }
                }
            }
            return out
        }

        /** Converts mono float PCM to mono PCM16. */
        fun monoFloatToPcm16(mono: FloatArray): ShortArray =
            ShortArray(mono.size) { i ->
                (mono[i].coerceIn(-1f, 1f) * 32767f).toInt().toShort()
            }
    }

    private enum class TxFormat {
        FLOAT_MONO,
        PCM16_STEREO,
        PCM16_MONO,
    }

    private enum class CaptureFormat {
        FLOAT_MONO,
        PCM16_MONO,
        PCM16_STEREO,
    }

    private val appContext = context.applicationContext
    private val audioManager =
        appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager

    // --- Device routing (set by the ViewModel before startCapture) ---

    /** Capture device to pin via setPreferredDevice (null = system default). */
    @Volatile
    var preferredInput: AudioDeviceInfo? = null

    /** Playback device to pin via setPreferredDevice (null = system default). */
    @Volatile
    var preferredOutput: AudioDeviceInfo? = null

    /**
     * When true (a Bluetooth SCO route is active), capture uses
     * VOICE_COMMUNICATION and playback uses USAGE_VOICE_COMMUNICATION so the
     * streams follow the communication route the platform was switched to.
     */
    @Volatile
    var communicationRoute = false

    /** Cable/USB stereo channel mapping for TX into the radio mic. */
    @Volatile
    var cableTxChannel: CableTxChannel = CableTxChannel.BOTH

    /**
     * Capture still runs (level meter) but the session must not feed pushRx
     * while this is true. Deadline-based so a crashed TX path can never leave
     * RX muted permanently (that looked like "nothing works" after a send).
     */
    val rxMuted: Boolean
        get() = SystemClock.elapsedRealtime() < rxMuteUntilElapsed

    /** True while a TX burst is playing (for UI). */
    @Volatile
    var transmitting = false
        private set

    private var record: AudioRecord? = null
    private var captureThread: Thread? = null

    @Volatile
    private var capturing = false

    @Volatile
    private var captureFormat: CaptureFormat = CaptureFormat.FLOAT_MONO

    private val playExecutor: ExecutorService = Executors.newSingleThreadExecutor { r ->
        Thread(r, "aetr-playback")
    }

    /** Persistent track kept open across bursts (USB/TRRS hate reopen). */
    private var playTrack: AudioTrack? = null

    @Volatile
    private var txFormat: TxFormat = TxFormat.FLOAT_MONO

    private var focusRequest: AudioFocusRequest? = null

    /** ElapsedRealtime deadline; RX decode suppressed until this time. */
    @Volatile
    private var rxMuteUntilElapsed = 0L

    /** Last cable TX routing note for the UI (null = fine). */
    @Volatile
    var lastTxRouteNote: String? = null

    /**
     * Peak the cable TX burst is normalized to. Exposed because the usable
     * window into a radio mic is bounded at both ends and varies by cable:
     * too quiet and the far end never syncs, too hot and the radio's own
     * stages distort. The modem itself tolerates hard clipping remarkably
     * well (see survives_severe_hard_clipping in core/src/tests.rs), so when
     * in doubt, err loud.
     */
    @Volatile
    var txPeakTarget: Float = CABLE_TX_PEAK

    /** Peak of the last burst actually written, for the TX meter. */
    @Volatile
    var lastTxPeak: Float = 0f

    /**
     * When set, every transmitted burst is written here as a 48 kHz mono WAV
     * before it reaches the AudioTrack. Pairs with the receiving end's capture
     * so the two can be compared: same file, different path.
     */
    @Volatile
    var txCaptureDir: java.io.File? = null

    /** Path of the most recent TX capture, for the UI to show. */
    @Volatile
    var lastTxCapturePath: String? = null

    /** Extra hang time after TX before unmuting RX (squelch / cable settle). */
    private val postTxMuteMs = 400L

    /**
     * Starts the mic capture loop. `onPcm` is invoked on the capture thread
     * with each freshly read block as 48 kHz mono float. Returns a short
     * status line (format + routed device) for the UI.
     *
     * Cable/USB inputs prefer PCM16 (stereo then mono): many APRS USB ADCs
     * open a float mono AudioRecord but deliver silence.
     */
    @SuppressLint("MissingPermission")
    fun startCapture(onPcm: (FloatArray) -> Unit): String {
        if (capturing) return "RX already capturing"
        val cable = !communicationRoute && isCableInterface(preferredInput)
        val (rec, format) = openRecordForRoute(cable)
        preferredInput?.let { device -> rec.setPreferredDevice(device) }
        record = rec
        captureFormat = format
        capturing = true
        rec.startRecording()
        Thread.sleep(50)
        val routed = rec.routedDevice
        captureThread = Thread({
            when (format) {
                CaptureFormat.FLOAT_MONO -> captureFloatLoop(rec, onPcm)
                CaptureFormat.PCM16_MONO -> capturePcm16MonoLoop(rec, onPcm)
                CaptureFormat.PCM16_STEREO -> capturePcm16StereoLoop(rec, onPcm)
            }
        }, "aetr-capture").also { it.start() }

        val fmt = when (format) {
            CaptureFormat.FLOAT_MONO -> "float mono"
            CaptureFormat.PCM16_MONO -> "PCM16 mono"
            CaptureFormat.PCM16_STEREO -> "PCM16 stereo→mono"
        }
        val parts = mutableListOf("RX $fmt")
        preferredInput?.let { parts += "preferred ${AudioRouter.label(it)}" }
        routed?.let { parts += "routed ${AudioRouter.label(it)}" }
        if (isCableInterface(preferredInput) && AudioRouter.isBuiltinMic(routed)) {
            parts += "WARNING: capture still on built-in mic — pick the cable as Input"
        }
        return parts.joinToString(" · ")
    }

    /** Plays a 1 kHz tone on the TX path so you can verify the radio/cable hears Android. */
    fun playTestTone() {
        play(testTonePcm())
    }

    /** Float mono read loop. */
    private fun captureFloatLoop(rec: AudioRecord, onPcm: (FloatArray) -> Unit) {
        val buf = FloatArray(CAPTURE_BLOCK)
        while (capturing) {
            val n = rec.read(buf, 0, buf.size, AudioRecord.READ_BLOCKING)
            if (n > 0) onPcm(buf.copyOf(n))
        }
    }

    /** PCM16 mono read loop, converted to float for the core. */
    private fun capturePcm16MonoLoop(rec: AudioRecord, onPcm: (FloatArray) -> Unit) {
        val buf = ShortArray(CAPTURE_BLOCK)
        while (capturing) {
            val n = rec.read(buf, 0, buf.size, AudioRecord.READ_BLOCKING)
            if (n > 0) onPcm(pcm16ToFloat(buf, n))
        }
    }

    /** PCM16 stereo read loop, downmixed to mono float. */
    private fun capturePcm16StereoLoop(rec: AudioRecord, onPcm: (FloatArray) -> Unit) {
        val buf = ShortArray(CAPTURE_BLOCK * 2)
        while (capturing) {
            val n = rec.read(buf, 0, buf.size, AudioRecord.READ_BLOCKING)
            if (n > 1) onPcm(pcm16StereoToMonoFloat(buf, n))
        }
    }

    /**
     * Opens an AudioRecord for the route. Cable/USB tries PCM16 stereo, then
     * PCM16 mono, then float; other routes keep the float-first source list.
     */
    @SuppressLint("MissingPermission")
    private fun openRecordForRoute(cable: Boolean): Pair<AudioRecord, CaptureFormat> {
        val formats = if (cable) {
            listOf(
                CaptureFormat.PCM16_STEREO,
                CaptureFormat.PCM16_MONO,
                CaptureFormat.FLOAT_MONO,
            )
        } else {
            listOf(CaptureFormat.FLOAT_MONO)
        }
        var lastError: Throwable? = null
        for (format in formats) {
            for (source in captureSources(communicationRoute, cable)) {
                try {
                    val rec = tryOpenRecord(source, format) ?: continue
                    return rec to format
                } catch (t: Throwable) {
                    lastError = t
                }
            }
        }
        throw lastError
            ?: IllegalStateException("AudioRecord failed to initialize for the selected input")
    }

    /**
     * Tries to open a 48 kHz AudioRecord for [source] and [format].
     * Returns null when the device rejects that combo.
     */
    @SuppressLint("MissingPermission")
    private fun tryOpenRecord(source: Int, format: CaptureFormat): AudioRecord? {
        val encoding = when (format) {
            CaptureFormat.FLOAT_MONO -> AudioFormat.ENCODING_PCM_FLOAT
            else -> AudioFormat.ENCODING_PCM_16BIT
        }
        val channels = when (format) {
            CaptureFormat.PCM16_STEREO -> AudioFormat.CHANNEL_IN_STEREO
            else -> AudioFormat.CHANNEL_IN_MONO
        }
        val bytesPerFrame = when (format) {
            CaptureFormat.FLOAT_MONO -> 4
            CaptureFormat.PCM16_MONO -> 2
            CaptureFormat.PCM16_STEREO -> 4
        }
        val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, channels, encoding)
        if (minBuf <= 0) return null
        val rec = AudioRecord(
            source,
            SAMPLE_RATE,
            channels,
            encoding,
            maxOf(minBuf * 4, CAPTURE_BLOCK * bytesPerFrame)
        )
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            return null
        }
        return rec
    }

    /** Stops the capture loop and releases the AudioRecord. */
    fun stopCapture() {
        capturing = false
        clearHalfDuplex()
        captureThread?.join(1000)
        captureThread = null
        record?.let {
            runCatching { it.stop() }
            it.release()
        }
        record = null
    }

    /** Clears TX/RX mute flags (safe to call anytime). */
    fun clearHalfDuplex() {
        transmitting = false
        rxMuteUntilElapsed = 0L
    }

    /**
     * Opens (or reopens) the persistent playback track for the current route.
     * Returns a short status line for the UI, or null when using defaults.
     */
    fun openPlayback(): String? {
        closePlaybackTrack()
        // Cable/USB always use PCM16, even when we temporarily flip
        // communicationRoute on for setCommunicationDevice TX. (The old
        // `!communicationRoute && cable` check fell back to float mono and
        // left Android→Mac silent.)
        val cable = isCableInterface(preferredOutput)
        val (track, format) = openTrackForRoute(cable)
        preferredOutput?.let { device ->
            if (!track.setPreferredDevice(device)) {
                track.release()
                error(
                    "Could not pin playback to ${AudioRouter.label(device)}. " +
                        "Android would TX on the speaker and the radio would hear nothing."
                )
            }
        }
        track.play()
        // Give the USB stack a moment, then verify the real sink.
        Thread.sleep(50)
        val routed = track.routedDevice
        playTrack = track
        txFormat = format

        val parts = mutableListOf<String>()
        if (preferredOutput != null) {
            val fmt = when (format) {
                TxFormat.PCM16_STEREO ->
                    "PCM16 stereo/${cableTxChannel.name} peak-norm"
                TxFormat.PCM16_MONO -> "PCM16 mono peak-norm"
                TxFormat.FLOAT_MONO -> "float mono"
            }
            parts += "TX pinned to ${AudioRouter.label(preferredOutput!!)} ($fmt)"
        }
        if (routed != null) {
            parts += "routed now: ${AudioRouter.label(routed)}"
            if (isCableInterface(preferredOutput) && AudioRouter.isBuiltinSink(routed)) {
                // Not fatal here. playBlocking retries this on the communication
                // route before giving up, and reports what actually happened.
                parts += "TX came up on ${AudioRouter.label(routed)} rather than the " +
                    "cable/USB device; the first transmission will try to force it."
            }
        }
        // Check the stream this track actually plays on. A communication-route
        // track is on STREAM_VOICE_CALL, whose level is a separate slider from
        // media volume, so warning about the wrong one sends you hunting in the
        // wrong place.
        val stream =
            if (communicationRoute) AudioManager.STREAM_VOICE_CALL else AudioManager.STREAM_MUSIC
        if (audioManager.getStreamVolume(stream) == 0) {
            val name = if (communicationRoute) "In-call volume" else "Media volume"
            parts += "$name is 0 — raise it or the radio hears silence."
        }
        if (cable) {
            parts += "Disable Gemini/Assistant if headset keys keep popping up."
        }
        return parts.takeIf { it.isNotEmpty() }?.joinToString("\n")
    }

    /**
     * Picks a working track format: cable prefers PCM16 stereo, then mono;
     * other routes use float mono.
     */
    private fun openTrackForRoute(cable: Boolean): Pair<AudioTrack, TxFormat> {
        if (!cable) {
            return buildPlayTrack(TxFormat.FLOAT_MONO) to TxFormat.FLOAT_MONO
        }
        val attempts = listOf(TxFormat.PCM16_STEREO, TxFormat.PCM16_MONO, TxFormat.FLOAT_MONO)
        var lastError: Throwable? = null
        for (fmt in attempts) {
            try {
                val track = buildPlayTrack(fmt)
                return track to fmt
            } catch (t: Throwable) {
                lastError = t
            }
        }
        throw lastError ?: IllegalStateException("No playback format worked for cable/USB")
    }

    /** Releases the persistent playback track. */
    fun closePlayback() {
        stopPlayback()
        closePlaybackTrack()
        abandonTxFocus()
    }

    private fun closePlaybackTrack() {
        playTrack?.let {
            runCatching { it.pause() }
            runCatching { it.flush() }
            runCatching { it.stop() }
            it.release()
        }
        playTrack = null
    }

    /** Builds a stream-mode track for [format]. */
    private fun buildPlayTrack(format: TxFormat): AudioTrack {
        val encoding = when (format) {
            TxFormat.FLOAT_MONO -> AudioFormat.ENCODING_PCM_FLOAT
            TxFormat.PCM16_STEREO, TxFormat.PCM16_MONO -> AudioFormat.ENCODING_PCM_16BIT
        }
        val channels = when (format) {
            TxFormat.PCM16_STEREO -> AudioFormat.CHANNEL_OUT_STEREO
            else -> AudioFormat.CHANNEL_OUT_MONO
        }
        val minBuf = AudioTrack.getMinBufferSize(SAMPLE_RATE, channels, encoding)
        check(minBuf > 0) { "AudioTrack buffer unsupported for $format" }
        // CONTENT_TYPE_UNKNOWN everywhere the platform lets us: MUSIC invites
        // OEM equalizers and "audio effects", and SPEECH invites the voice
        // pipeline (see the note on communicationRoute below). Both mangle a
        // COFDM burst. UNKNOWN is the most unprocessed thing we can ask for.
        //
        // USAGE_VOICE_COMMUNICATION is used only when we genuinely are on a
        // communication route (Bluetooth SCO, or the cable fallback in
        // playBlocking). It puts playback on STREAM_VOICE_CALL and enables the
        // platform's downlink voice processing: speech EQ, noise suppression,
        // and an AGC. The AGC is the problem. core/src/tests.rs
        // (syllabic_agc_destroys_the_payload) measures a gain that moves on
        // the symbol timescale destroying the payload outright, because the
        // modem demodulates each carrier differentially against the previous
        // symbol 180 ms earlier and only a *constant* gain cancels.
        val attrs = AudioAttributes.Builder()
            .setUsage(
                if (communicationRoute) AudioAttributes.USAGE_VOICE_COMMUNICATION
                else AudioAttributes.USAGE_MEDIA
            )
            .setContentType(
                if (communicationRoute) AudioAttributes.CONTENT_TYPE_SPEECH
                else AudioAttributes.CONTENT_TYPE_UNKNOWN
            )
            .build()
        val audioFormat = AudioFormat.Builder()
            .setEncoding(encoding)
            .setSampleRate(SAMPLE_RATE)
            .setChannelMask(channels)
            .build()
        fun build(lowLatency: Boolean): AudioTrack {
            val builder = AudioTrack.Builder()
                .setAudioAttributes(attrs)
                .setAudioFormat(audioFormat)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(minBuf * 4)
            if (lowLatency) {
                builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            }
            return builder.build()
        }
        val track = build(lowLatency = format == TxFormat.FLOAT_MONO).let { t ->
            if (t.state == AudioTrack.STATE_INITIALIZED) t
            else {
                t.release()
                build(lowLatency = false)
            }
        }
        check(track.state == AudioTrack.STATE_INITIALIZED) {
            "AudioTrack failed to initialize for $format"
        }
        track.setVolume(1.0f)
        return track
    }

    /**
     * Queues a PCM burst for playback. Bursts play back-to-back in submission
     * order; `onDone` fires (on the playback thread) after the burst drains.
     * When [halfDuplex] is true, RX stays muted until the caller clears it
     * via [clearHalfDuplex] (after modem flush). Voice clip replay leaves RX open.
     */
    fun play(pcm: FloatArray, halfDuplex: Boolean = false, onDone: (() -> Unit)? = null) {
        if (pcm.isEmpty()) {
            onDone?.invoke()
            return
        }
        playExecutor.execute {
            try {
                playBlocking(pcm, halfDuplex)
            } finally {
                onDone?.invoke()
            }
        }
    }

    /**
     * Writes one burst through the persistent track and waits until the
     * playback head has advanced by the burst's frame count.
     * When [halfDuplex], mutes RX for the duration so cable TX→RX crosstalk
     * cannot look like a successful over-the-air receive on the same phone.
     * Mute is left armed for the caller to clear after modem flush.
     */
    private fun playBlocking(pcm: FloatArray, halfDuplex: Boolean) {
        val out = preferredOutput
        val cable = isCableInterface(out)
        if (halfDuplex) {
            // Cap so a hung TX path can never mute forever; ViewModel also
            // clears in finally after flush.
            val approxMs = (pcm.size * 1000L) / SAMPLE_RATE +
                if (cable) postTxMuteMs + 2_000L else 2_000L
            muteRxFor(approxMs.coerceAtMost(60_000L))
            transmitting = true
        }
        lastTxRouteNote = null
        // Exclusive focus mutes the mic on many OEMs. Soft focus for non-cable.
        if (!cable) requestTxFocus()
        var cableRouteHeld = false
        val notes = mutableListOf<String>()
        try {
            var t = ensurePlayingTrack()

            // Prefer the plain media route. setPreferredDevice pins the sink on
            // most hardware, and staying off the communication route keeps the
            // burst away from the platform's voice AGC, which destroys it
            // outright (see buildPlayTrack). Only when the sink demonstrably
            // came up on a built-in speaker do we take the communication route
            // instead: a mangled burst still beats one the radio never hears.
            if (cable && AudioRouter.isBuiltinSink(t.routedDevice) && out != null && router != null) {
                notes += "TX came up on ${AudioRouter.label(t.routedDevice)}; " +
                    "forcing the communication route (voice processing may degrade decode)"
                router.activateCableTx(out)?.let { notes += it }
                cableRouteHeld = true
                val savedComm = communicationRoute
                communicationRoute = true
                try {
                    closePlaybackTrack()
                    openPlayback()
                } finally {
                    communicationRoute = savedComm
                }
                // Give USB a beat to switch before writing samples.
                Thread.sleep(100)
                t = ensurePlayingTrack()
            }

            val routed = t.routedDevice
            if (cable && AudioRouter.isBuiltinSink(routed)) {
                // Both routes landed on a speaker. Transmit anyway (the burst
                // may still reach an acoustically coupled radio) but say so
                // plainly, rather than throwing off the playback thread.
                notes += "TX is on ${routed?.let(AudioRouter::label) ?: "the speaker"}, " +
                    "not the cable/USB device — check the Output picker"
            } else if (cable && routed != null) {
                notes += "TX sink ${AudioRouter.label(routed)}"
            }

            // Cable output into a radio mic is level-sensitive at the quiet
            // end, so normalize to the configured peak. Session VOX primer is
            // already inside [pcm] and scales with it.
            val txPcm = if (cable) peakNormalize(pcm, txPeakTarget) else pcm
            lastTxPeak = AudioLevels.peakAbs(txPcm)
            dumpTxCapture(txPcm)
            val frames = when (txFormat) {
                TxFormat.PCM16_STEREO -> {
                    writeFully(t, monoFloatToStereoPcm16(txPcm, cableTxChannel))
                    txPcm.size
                }
                TxFormat.PCM16_MONO -> {
                    writeFully(t, monoFloatToPcm16(txPcm))
                    txPcm.size
                }
                TxFormat.FLOAT_MONO -> {
                    writeFully(t, txPcm)
                    txPcm.size
                }
            }
            waitForFrames(t, frames)
            if (cable && halfDuplex) Thread.sleep(postTxMuteMs)
        } catch (t: Throwable) {
            // This runs on the playback executor, where an escaping throwable
            // reaches the default uncaught handler and kills the process. A
            // failed transmission is a thing to report, not to crash over.
            notes += "TX failed: ${t.message ?: t::class.java.simpleName}"
        } finally {
            lastTxRouteNote = notes.takeIf { it.isNotEmpty() }?.joinToString(" · ")
            // Half-duplex TX: leave rxMuted armed until ViewModel flushes the
            // modem and calls clearHalfDuplex (clearing here raced decode→wipe).
            if (!halfDuplex) {
                transmitting = false
            }
            if (!cable) abandonTxFocus()
            if (cableRouteHeld) {
                router?.releaseCableTx()
                // Rebuild a normal (non-communication) idle track for later TX.
                runCatching {
                    closePlaybackTrack()
                    openPlayback()
                }
            }
            // Re-pin capture; communication mode can steal the USB mic route.
            repinCaptureDevice()
        }
    }

    /**
     * Returns the persistent track, opening it if needed and making sure it is
     * playing. Throws if no track can be opened at all.
     */
    private fun ensurePlayingTrack(): AudioTrack {
        var track = playTrack
        if (track == null || track.state != AudioTrack.STATE_INITIALIZED) {
            openPlayback()
            track = playTrack
        }
        val t = track ?: error("Playback track unavailable")
        if (t.playState != AudioTrack.PLAYSTATE_PLAYING) {
            t.play()
        }
        return t
    }

    /**
     * Writes the exact samples handed to the AudioTrack to a WAV, when
     * [txCaptureDir] is set. Capture failures are swallowed: a diagnostic
     * must never be the reason a transmission does not go out.
     */
    private fun dumpTxCapture(pcm: FloatArray) {
        val dir = txCaptureDir ?: return
        runCatching {
            dir.mkdirs()
            val file = java.io.File(dir, "aetr-tx-${System.currentTimeMillis()}.wav")
            uniffi.aetr_core.writeWav48k(file.absolutePath, pcm.toList())
            lastTxCapturePath = file.absolutePath
        }
    }

    /** Suppress pushRx until [durationMs] from now (extends if already muted). */
    private fun muteRxFor(durationMs: Long) {
        val until = SystemClock.elapsedRealtime() + durationMs
        if (until > rxMuteUntilElapsed) rxMuteUntilElapsed = until
    }

    /** Re-applies setPreferredDevice on the live AudioRecord after TX. */
    private fun repinCaptureDevice() {
        val rec = record ?: return
        val device = preferredInput ?: return
        runCatching { rec.setPreferredDevice(device) }
    }

    /** Blocking write of a full float buffer. */
    private fun writeFully(track: AudioTrack, pcm: FloatArray) {
        var offset = 0
        while (offset < pcm.size) {
            val n = track.write(pcm, offset, pcm.size - offset, AudioTrack.WRITE_BLOCKING)
            if (n <= 0) break
            offset += n
        }
        check(offset == pcm.size) { "AudioTrack write short (${offset}/${pcm.size} floats)" }
    }

    /** Blocking write of a full PCM16 buffer. */
    private fun writeFully(track: AudioTrack, pcm: ShortArray) {
        var offset = 0
        while (offset < pcm.size) {
            val n = track.write(pcm, offset, pcm.size - offset, AudioTrack.WRITE_BLOCKING)
            if (n <= 0) break
            offset += n
        }
        check(offset == pcm.size) { "AudioTrack write short (${offset}/${pcm.size} shorts)" }
    }

    /**
     * Waits until [frames] have left the track (or a timeout). Head position
     * can wrap on long sessions, so we track a relative delta.
     */
    private fun waitForFrames(track: AudioTrack, frames: Int) {
        val start = track.playbackHeadPosition.toLong() and 0xffffffffL
        val deadline = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < deadline) {
            if (track.playState == AudioTrack.PLAYSTATE_STOPPED) break
            val now = track.playbackHeadPosition.toLong() and 0xffffffffL
            val advanced = if (now >= start) now - start else (0x100000000L - start) + now
            if (advanced >= frames.toLong()) break
            Thread.sleep(20)
        }
    }

    /**
     * Soft transient focus only (never EXCLUSIVE). Exclusive focus is what
     * left the mic dead on several phones after Android TX.
     */
    private fun requestTxFocus() {
        val attrs = AudioAttributes.Builder()
            .setUsage(
                if (communicationRoute) AudioAttributes.USAGE_VOICE_COMMUNICATION
                else AudioAttributes.USAGE_MEDIA
            )
            .setContentType(AudioAttributes.CONTENT_TYPE_UNKNOWN)
            .build()
        val req = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK)
            .setAudioAttributes(attrs)
            .setAcceptsDelayedFocusGain(false)
            .setOnAudioFocusChangeListener { }
            .build()
        focusRequest = req
        audioManager.requestAudioFocus(req)
        if (Build.VERSION.SDK_INT < 31 && !communicationRoute) {
            @Suppress("DEPRECATION")
            audioManager.mode = AudioManager.MODE_NORMAL
        }
    }

    private fun abandonTxFocus() {
        focusRequest?.let { audioManager.abandonAudioFocusRequest(it) }
        focusRequest = null
    }

    /** Cuts any burst that is currently playing. */
    fun stopPlayback() {
        playTrack?.let { runCatching { it.pause(); it.flush() } }
    }

    /** Releases everything; the engine is unusable afterwards. */
    fun shutdown() {
        clearHalfDuplex()
        stopCapture()
        closePlayback()
        playExecutor.shutdownNow()
    }
}
