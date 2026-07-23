package me.faulk.aetr

import android.app.Application
import android.media.AudioDeviceInfo
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.aetr_core.AetrException
import uniffi.aetr_core.AetrSession
import uniffi.aetr_core.ModemMode
import uniffi.aetr_core.RxEvent
import uniffi.aetr_core.RxState
import uniffi.aetr_core.SessionConfig
import java.util.concurrent.atomic.AtomicLong

/**
 * One row in the message log. Each variant carries a locally unique `uid` so
 * Compose can key rows and the ViewModel can replace in-flight entries when
 * their final event (Text/Voice/Failed) arrives.
 */
sealed interface LogEntry {
    val uid: Long

    /** A text message we transmitted. */
    data class SentText(override val uid: Long, val text: String) : LogEntry

    /** A voice clip we transmitted, kept for local replay. */
    data class SentVoice(
        override val uid: Long,
        val pcm: FloatArray,
        val airtimeSecs: Double,
    ) : LogEntry

    /** A fully received text message. */
    data class ReceivedText(
        override val uid: Long,
        val messageId: ULong,
        val text: String,
    ) : LogEntry

    /** A received voice clip, possibly with silence gaps at missing spans. */
    data class ReceivedVoice(
        override val uid: Long,
        val messageId: ULong,
        val pcm: FloatArray,
        val missingSpans: List<UInt>,
    ) : LogEntry

    /** A message still being assembled; shows a progress bar + repair button. */
    data class InProgress(
        override val uid: Long,
        val messageId: ULong,
        val received: UInt,
        val total: UInt,
        val isVoice: Boolean,
    ) : LogEntry

    /** A message that timed out or hit a receive-path error. */
    data class Failed(
        override val uid: Long,
        val messageId: ULong,
        val reason: String,
    ) : LogEntry

    /** A peer asked us to repair a message; holds the ready-to-send burst. */
    data class RepairRequest(
        override val uid: Long,
        val messageId: ULong,
        val pcm: FloatArray,
    ) : LogEntry
}

/**
 * Single-screen app state: session lifecycle, the audio path, the message
 * log, and TX/RX actions. All FFI calls that can block (KDF, encode, poll)
 * run on Dispatchers.Default; only pushRx runs on the audio capture thread,
 * which is what the core is designed for.
 */
class AetrViewModel(app: Application) : AndroidViewModel(app) {

    private val audio = AudioEngine()
    private val router = AudioRouter(app)
    private var session: AetrSession? = null
    private var pollJob: Job? = null
    private val nextUid = AtomicLong(1)

    // --- Connection settings ---
    var passphrase by mutableStateOf("")
    var mode by mutableStateOf(ModemMode.B170)
    var voiceCapText by mutableStateOf("30")
    var txDelayText by mutableStateOf("1000")
    var voxPrimer by mutableStateOf(false)

    // --- Audio device routing ---
    var inputDevices by mutableStateOf(listOf<AudioDeviceInfo>())
        private set
    var outputDevices by mutableStateOf(listOf<AudioDeviceInfo>())
        private set
    var selectedInput by mutableStateOf<AudioDeviceInfo?>(null)
    var selectedOutput by mutableStateOf<AudioDeviceInfo?>(null)

    /** Route health line shown while connected (BT active / BT failed). */
    var routeStatus by mutableStateOf<String?>(null)
        private set

    init {
        refreshDevices()
        router.registerDeviceCallback { refreshDevices() }
    }

    // --- Session state ---
    var connected by mutableStateOf(false)
        private set
    var connecting by mutableStateOf(false)
        private set
    var rxState by mutableStateOf(RxState.IDLE)
        private set

    /** Debug: pipe encoded bursts straight into pushRx instead of the speaker. */
    var loopback by mutableStateOf(false)

    // --- Log + compose state ---
    val log = mutableStateListOf<LogEntry>()
    var draft by mutableStateOf("")
    var errorMessage by mutableStateOf<String?>(null)

    // --- Voice recording state ---
    var recording by mutableStateOf(false)
        private set
    var recordedSecs by mutableStateOf(0f)
        private set

    private val clipLock = Any()
    private var clip = ArrayList<Float>()
    private var clipCapSamples = 0

    /** The voice cap the UI is currently configured with, clamped sane. */
    fun voiceCapSecs(): UInt = voiceCapText.toUIntOrNull()?.coerceIn(1u, 600u) ?: 30u

    /** The TX key-up delay the UI is configured with, clamped to 0-5000 ms. */
    fun txDelayMs(): UInt = txDelayText.toUIntOrNull()?.coerceIn(0u, 5000u) ?: 1000u

    /** True when either selected audio device rides a Bluetooth link. */
    fun bluetoothSelected(): Boolean =
        (selectedInput?.let(AudioRouter::isBluetooth) == true) ||
            (selectedOutput?.let(AudioRouter::isBluetooth) == true)

    /** Re-reads the device lists, dropping any selection that vanished. */
    fun refreshDevices() {
        inputDevices = router.inputDevices()
        outputDevices = router.outputDevices()
        selectedInput?.let { sel ->
            if (inputDevices.none { it.id == sel.id }) selectedInput = null
        }
        selectedOutput?.let { sel ->
            if (outputDevices.none { it.id == sel.id }) selectedOutput = null
        }
    }

    /**
     * Runs the KDF off the main thread, then starts the mic capture loop
     * (feeding pushRx continuously) and the 10 Hz event poll loop.
     */
    fun connect() {
        if (connected || connecting) return
        connecting = true
        errorMessage = null
        viewModelScope.launch {
            try {
                val config = SessionConfig(
                    passphrase, mode, voiceCapSecs(), txDelayMs(), voxPrimer
                )
                val s = withContext(Dispatchers.Default) { AetrSession(config) }
                session = s
                applyRoute()
                // Start capture before flipping `connected`: opening the mic can
                // fail on the selected device (e.g. a USB-C audio adapter that
                // doesn't support 48 kHz mono float capture), and we don't want
                // to advance into the connected UI with a half-open session.
                startCapture(s)
                connected = true
                startPolling(s)
            } catch (e: AetrException) {
                errorMessage = "Connect failed: ${e.message}"
                cleanupFailedConnect()
            } catch (e: Throwable) {
                // Catch Throwable, not just Exception: two failure classes here are
                // Errors that would otherwise escape the coroutine and crash the app
                // with no in-app message. (1) AudioRecord/AudioTrack setup throws
                // IllegalArgumentException/IllegalStateException when the chosen audio
                // device can't provide the requested format or route. (2) First touch
                // of AetrSession loads the native aetr-core library via JNA; a missing
                // or ABI-mismatched .so throws UnsatisfiedLinkError, and a bindings/.so
                // checksum mismatch surfaces as ExceptionInInitializerError, both of
                // which are Errors. Surface them all instead of letting them crash.
                errorMessage = "Connect failed: ${e.message ?: e.javaClass.simpleName}"
                cleanupFailedConnect()
            } finally {
                connecting = false
            }
        }
    }

    /**
     * Unwinds a partially-started session when connect() fails partway (most
     * often the mic failing to open on the selected device). Mirrors the audio
     * and session teardown in disconnect() without touching connection UI flags
     * the caller manages; leaves `connected` false and settings intact.
     */
    private fun cleanupFailedConnect() {
        audio.stopCapture()
        audio.preferredInput = null
        audio.preferredOutput = null
        audio.communicationRoute = false
        router.release()
        routeStatus = null
        connected = false
        session?.close()
        session = null
    }

    /**
     * Applies the selected audio devices before streaming starts: the
     * platform-level Bluetooth route first (SCO comes up asynchronously, so
     * this suspends until it is active or times out), then per-stream
     * pinning on the engine. A Bluetooth failure degrades to the default
     * route with a visible status line rather than blocking the session.
     */
    private suspend fun applyRoute() {
        val bt = bluetoothSelected()
        audio.preferredInput = selectedInput
        audio.preferredOutput = selectedOutput
        audio.communicationRoute = bt
        routeStatus = if (bt) {
            val failure = router.activate(selectedInput, selectedOutput)
            if (failure != null) {
                audio.preferredInput = null
                audio.preferredOutput = null
                audio.communicationRoute = false
                router.release()
                "Bluetooth routing failed: $failure. Using the default route."
            } else if (mode != ModemMode.B85) {
                "Bluetooth route active. SCO audio is narrowband and lossy; " +
                    "the robust 85 B mode is recommended over Bluetooth."
            } else {
                "Bluetooth route active."
            }
        } else {
            null
        }
    }

    /** Tears down audio, polling, routing, and the session (keeps settings). */
    fun disconnect() {
        pollJob?.cancel()
        pollJob = null
        audio.stopCapture()
        audio.stopPlayback()
        audio.preferredInput = null
        audio.preferredOutput = null
        audio.communicationRoute = false
        router.release()
        routeStatus = null
        synchronized(clipLock) { clip = ArrayList() }
        recording = false
        session?.close()
        session = null
        connected = false
        rxState = RxState.IDLE
    }

    /**
     * Mic capture: every block feeds the modem receiver; while the user holds
     * the record button the same blocks also accumulate into the voice clip,
     * hard-capped at the configured length.
     */
    private fun startCapture(s: AetrSession) {
        audio.startCapture { block ->
            s.pushRx(block.toList())
            if (recording) {
                var full = false
                synchronized(clipLock) {
                    val room = clipCapSamples - clip.size
                    if (room > 0) {
                        val take = minOf(room, block.size)
                        for (i in 0 until take) clip.add(block[i])
                    }
                    recordedSecs = clip.size / AudioEngine.SAMPLE_RATE.toFloat()
                    full = clip.size >= clipCapSamples
                }
                if (full) stopRecording()
            }
        }
    }

    /** Polls the core ~10 Hz for events and the RX badge state. */
    private fun startPolling(s: AetrSession) {
        pollJob = viewModelScope.launch {
            while (isActive) {
                val events = withContext(Dispatchers.Default) { s.pollEvents() }
                events.forEach(::handleEvent)
                rxState = withContext(Dispatchers.Default) { s.rxState() }
                delay(100)
            }
        }
    }

    /** Folds one core event into the log. */
    private fun handleEvent(event: RxEvent) {
        when (event) {
            is RxEvent.Progress -> {
                val i = log.indexOfFirst {
                    it is LogEntry.InProgress && it.messageId == event.messageId
                }
                val entry = LogEntry.InProgress(
                    uid = if (i >= 0) log[i].uid else nextUid.getAndIncrement(),
                    messageId = event.messageId,
                    received = event.received,
                    total = event.total,
                    isVoice = event.isVoice,
                )
                if (i >= 0) log[i] = entry else log.add(entry)
            }

            is RxEvent.Text -> {
                removeInProgress(event.messageId)
                log.add(
                    LogEntry.ReceivedText(
                        nextUid.getAndIncrement(), event.messageId, event.text
                    )
                )
            }

            is RxEvent.Voice -> {
                removeInProgress(event.messageId)
                log.add(
                    LogEntry.ReceivedVoice(
                        nextUid.getAndIncrement(),
                        event.messageId,
                        event.pcm48k.toFloatArray(),
                        event.missingSpans,
                    )
                )
            }

            is RxEvent.Failed -> {
                removeInProgress(event.messageId)
                log.add(
                    LogEntry.Failed(
                        nextUid.getAndIncrement(), event.messageId, event.reason
                    )
                )
            }

            is RxEvent.RepairRequested -> {
                log.add(
                    LogEntry.RepairRequest(
                        nextUid.getAndIncrement(),
                        event.messageId,
                        event.pcmResponse.toFloatArray(),
                    )
                )
            }
        }
    }

    /** Drops the in-flight progress row for a message that just finalized. */
    private fun removeInProgress(messageId: ULong) {
        log.removeAll { it is LogEntry.InProgress && it.messageId == messageId }
    }

    /** Encodes and transmits the drafted text message. */
    fun sendText() {
        val text = draft.trim()
        val s = session ?: return
        if (text.isEmpty()) return
        draft = ""
        viewModelScope.launch {
            try {
                val burst = withContext(Dispatchers.Default) { s.encodeText(text) }
                log.add(LogEntry.SentText(nextUid.getAndIncrement(), text))
                transmit(s, burst)
            } catch (e: AetrException) {
                errorMessage = "Send failed: ${e.message}"
            }
        }
    }

    /** Begins accumulating mic blocks into a voice clip (press of hold-to-talk). */
    fun startRecording() {
        if (!connected || recording) return
        synchronized(clipLock) {
            clip = ArrayList()
            clipCapSamples = (voiceCapSecs().toInt()) * AudioEngine.SAMPLE_RATE
            recordedSecs = 0f
        }
        recording = true
    }

    /**
     * Ends the hold-to-talk gesture: encodes the captured clip and transmits
     * it. Clips shorter than a quarter second are discarded as accidental taps.
     */
    fun stopRecording() {
        if (!recording) return
        recording = false
        val s = session ?: return
        val samples: FloatArray
        synchronized(clipLock) {
            samples = clip.toFloatArray()
            clip = ArrayList()
        }
        if (samples.size < AudioEngine.SAMPLE_RATE / 4) return
        viewModelScope.launch {
            try {
                val burst = withContext(Dispatchers.Default) {
                    s.encodeVoice(samples.toList())
                }
                val airtime = estimateVoiceAirtime(samples.size.toULong())
                log.add(
                    LogEntry.SentVoice(nextUid.getAndIncrement(), samples, airtime)
                )
                transmit(s, burst)
            } catch (e: AetrException) {
                errorMessage = "Voice send failed: ${e.message}"
            }
        }
    }

    /** Asks the original sender to repair an incomplete message. */
    fun requestRepair(messageId: ULong) {
        val s = session ?: return
        viewModelScope.launch {
            try {
                val burst = withContext(Dispatchers.Default) {
                    s.requestRepair(messageId)
                }
                transmit(s, burst)
            } catch (e: AetrException) {
                errorMessage = "Repair request failed: ${e.message}"
            }
        }
    }

    /** Transmits the cached repair burst answering a peer's request. */
    fun sendRepair(entry: LogEntry.RepairRequest) {
        val s = session ?: return
        viewModelScope.launch {
            transmit(s, entry.pcm.toList())
            log.removeAll { it.uid == entry.uid }
        }
    }

    /** Replays a voice clip (sent or received) through the speaker. */
    fun playClip(pcm: FloatArray) {
        audio.play(pcm)
    }

    /** Clears the receiver (stuck sync, stale partials) without reconnecting. */
    fun resetRx() {
        val s = session ?: return
        viewModelScope.launch(Dispatchers.Default) { s.resetRx() }
    }

    /**
     * Routes an encoded burst out: digitally into our own receiver when the
     * debug loopback is on, otherwise through the speaker.
     */
    private suspend fun transmit(s: AetrSession, burst: List<Float>) {
        if (loopback) {
            withContext(Dispatchers.Default) { s.pushRx(burst) }
        } else {
            audio.play(burst.toFloatArray())
        }
    }

    /**
     * Estimated on-air seconds for a voice payload of `samples48k` samples in
     * the currently selected mode, including the configured TX key-up delay.
     * Falls back to NaN if the native library cannot answer (never expected
     * on device).
     */
    fun estimateVoiceAirtime(samples48k: ULong): Double = runCatching {
        uniffi.aetr_core.estimateAirtimeSecs(
            mode, uniffi.aetr_core.PayloadKind.VOICE, samples48k, txDelayMs()
        )
    }.getOrDefault(Double.NaN)

    override fun onCleared() {
        disconnect()
        router.unregisterDeviceCallback()
        audio.shutdown()
    }
}
