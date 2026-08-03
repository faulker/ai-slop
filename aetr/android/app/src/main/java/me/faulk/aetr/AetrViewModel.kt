package me.faulk.aetr

import android.app.Application
import android.media.AudioDeviceInfo
import android.os.SystemClock
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
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import uniffi.aetr_core.AetrException
import uniffi.aetr_core.AetrSession
import uniffi.aetr_core.ModemMode
import uniffi.aetr_core.RxCounters
import uniffi.aetr_core.RxEvent
import uniffi.aetr_core.RxState
import uniffi.aetr_core.SessionConfig
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlin.coroutines.resume

/**
 * One row in the message log. Each variant carries a locally unique `uid` so
 * Compose can key rows and the ViewModel can replace in-flight entries when
 * their final event (Text/Voice/Failed) arrives.
 */
sealed interface LogEntry {
    val uid: Long

    /** A text message we transmitted. `encrypted` false = sent in the clear. */
    data class SentText(
        override val uid: Long,
        val text: String,
        val encrypted: Boolean,
    ) : LogEntry

    /** A voice clip we transmitted, kept for local replay. */
    data class SentVoice(
        override val uid: Long,
        val pcm: FloatArray,
        val airtimeSecs: Double,
    ) : LogEntry

    /** A fully received text message. `encrypted` false = arrived in the clear. */
    data class ReceivedText(
        override val uid: Long,
        val messageId: ULong,
        val text: String,
        val encrypted: Boolean,
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

    private val router = AudioRouter(app)
    private val audio = AudioEngine(app, router)
    private var session: AetrSession? = null
    private var pollJob: Job? = null
    private val nextUid = AtomicLong(1)

    // --- Connection settings ---
    var passphrase by mutableStateOf("")
    /** B85 is the reliable default over APRS cables / Bluetooth. */
    var mode by mutableStateOf(ModemMode.B85)
    var voiceCapText by mutableStateOf("30")
    var txDelayText by mutableStateOf("1000")
    var voxPrimer by mutableStateOf(false)

    /**
     * When on (default), incomplete receives auto-request repair after a
     * short stall, and repair responses auto-TX. Needed for VOX-only radios
     * that cannot manually PTT a second repair burst.
     */
    var autoArq by mutableStateOf(true)

    /**
     * Which stereo channel carries TX into the radio mic on cable/USB.
     * Try LEFT or RIGHT if BOTH does not key the radio.
     */
    var cableTxChannel by mutableStateOf(AudioEngine.CableTxChannel.BOTH)
        private set

    /** Updates the cable TX channel mapping (applies to the next burst). */
    fun selectCableTxChannel(channel: AudioEngine.CableTxChannel) {
        cableTxChannel = channel
        audio.cableTxChannel = channel
    }

    /** Send text encrypted (default). Off = transmit in the clear; receivers
     * read it without the passphrase. Forced off with an empty passphrase. */
    var sendEncrypted by mutableStateOf(true)

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

    /**
     * Picks the capture device and, for cable/USB mics, auto-selects the
     * matching playback half so TX does not stay on the built-in speaker.
     */
    fun selectInput(device: AudioDeviceInfo?) {
        selectedInput = device
        if (device != null && AudioEngine.isCableInterface(device)) {
            AudioRouter.matchOutputForInput(device, outputDevices)?.let {
                selectedOutput = it
            }
        }
    }

    /**
     * Picks the playback device. For cable/USB TX, also auto-selects a matching
     * input so RX is not left on the built-in mic.
     */
    fun selectOutput(device: AudioDeviceInfo?) {
        selectedOutput = device
        if (device != null && AudioEngine.isCableInterface(device) &&
            !AudioEngine.isCableInterface(selectedInput)
        ) {
            val match = inputDevices.firstOrNull { inDev ->
                AudioEngine.isCableInterface(inDev) && (
                    inDev.id == device.id ||
                        (inDev.address.isNotEmpty() && inDev.address == device.address) ||
                        inDev.productName?.toString() == device.productName?.toString()
                    )
            } ?: inputDevices.firstOrNull { AudioEngine.isCableInterface(it) }
            if (match != null) selectedInput = match
        }
    }

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

    /**
     * Modem frames that synced but failed auth/checksum. Non-zero with a
     * Syncing badge and no chat row usually means wrong passphrase, mode
     * mismatch damage, or a corrupted analog path.
     */
    var rejectedFrames by mutableStateOf(0uL)
        private set

    /**
     * Where the receiver is losing bursts, stage by stage. The stages run in
     * order, so the first non-zero counter with a zero successor is the point
     * of failure; all zero means no burst energy is arriving at all, which is
     * a routing or level problem rather than a decode one.
     */
    var rxCounters by mutableStateOf(RxCounters(0uL, 0uL, 0uL, 0uL))
        private set

    /** Constellation SNR in dB from the last demodulated symbol (-99 = none). */
    var lastSnrDb by mutableStateOf(-99f)
        private set

    /**
     * Smoothed input peak 0..1 for the RX level meter. Updated on the capture
     * thread into [inputPeakRaw], sampled onto the UI at the poll rate.
     */
    var inputLevel by mutableStateOf(0f)
        private set

    /**
     * Input RMS in dBFS. The peak meter alone cannot tell you whether you are
     * near the receiver's sync floor, which sits around -57 dBFS RMS: below it
     * nothing syncs and every counter reads zero, exactly like a dead route.
     */
    var inputRmsDbfs by mutableStateOf(Float.NEGATIVE_INFINITY)
        private set

    /** Peak of the last transmitted burst, 0..1. */
    var txPeak by mutableStateOf(0f)
        private set

    /**
     * Peak the cable TX burst is normalized to. The core's own burst peaks
     * near 0.49, so values above that are the only ones that add gain.
     */
    var txLevel by mutableStateOf(AudioEngine.CABLE_TX_PEAK)
        private set

    /** Sets the cable TX peak target (applies to the next burst). */
    fun selectTxLevel(level: Float) {
        txLevel = level.coerceIn(0.1f, 1.0f)
        audio.txPeakTarget = txLevel
    }

    /**
     * When on, every transmitted burst is also written to a WAV under the
     * app's external files dir, so it can be compared against what the far
     * end actually recorded.
     */
    var captureTx by mutableStateOf(false)
        private set

    /** Most recent TX capture path, shown so the file can be pulled off. */
    var lastTxCapturePath by mutableStateOf<String?>(null)
        private set

    /** Turns TX capture on or off. */
    fun enableTxCapture(on: Boolean) {
        captureTx = on
        audio.txCaptureDir =
            if (on) getApplication<Application>().getExternalFilesDir(null) else null
    }

    /** True while a TX burst is on the wire (RX is muted for half-duplex). */
    var transmitting by mutableStateOf(false)
        private set

    private val inputPeakRaw = AtomicReference(0f)
    private val inputRmsRaw = AtomicReference(0f)

    /** messageId → (last progress elapsedRealtime, received, total). */
    private val progressWatch = mutableMapOf<ULong, Triple<Long, UInt, UInt>>()
    /** messageIds we already auto-requested repair for (one shot). */
    private val autoRepairRequestedIds = mutableSetOf<ULong>()

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
        // An empty passphrase makes a plaintext-only session, so force clear
        // sending (encryption needs a key).
        if (passphrase.isEmpty()) sendEncrypted = false
        connecting = true
        errorMessage = null
        viewModelScope.launch {
            try {
                ensureCableTxRoute()
                prepareCableTxDefaults()
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
                val rxStatus = startCapture(s)
                routeStatus = listOfNotNull(routeStatus, rxStatus).joinToString("\n")
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
        audio.closePlayback()
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
     * When a cable/USB mic is in use, forces a cable/USB output. Android
     * often keeps the speaker as the default sink while the cable is only
     * used for capture — that is exactly Mac→Android works / Android→Mac fails.
     */
    private fun ensureCableTxRoute() {
        val input = selectedInput
        val output = selectedOutput
        val cableIn = AudioEngine.isCableInterface(input)
        val cableOut = AudioEngine.isCableInterface(output)
        if (!cableIn && !cableOut) return

        if (!cableOut) {
            val match = input?.let { AudioRouter.matchOutputForInput(it, outputDevices) }
                ?: outputDevices.firstOrNull { AudioEngine.isCableInterface(it) }
            if (match != null) {
                selectedOutput = match
            } else {
                error(
                    "APRS cable/USB is selected for input but no cable/USB output " +
                        "is available. Plug the interface in and pick it as " +
                        "Output (radio TX) — otherwise Android plays TX on the speaker."
                )
            }
        }
        if (!AudioEngine.isCableInterface(selectedInput)) {
            selectOutput(selectedOutput) // also auto-fills matching cable input
            if (!AudioEngine.isCableInterface(selectedInput)) {
                error(
                    "Pick the APRS cable/USB as Input (radio mic) as well as Output. " +
                        "Otherwise Android listens on the built-in mic and never hears the radio."
                )
            }
        }
        if (AudioRouter.isBuiltinSink(selectedOutput)) {
            error(
                "Output is ${selectedOutput?.let(AudioRouter::label)}. " +
                    "For Android→radio TX you must select the APRS cable/USB device " +
                    "as Output (radio TX), not the built-in speaker."
            )
        }
    }

    /**
     * Cable/USB TX is quiet and often VOX-keyed. Turn on the primer and a
     * usable key-up delay so the radio opens before the modem burst.
     */
    private fun prepareCableTxDefaults() {
        val cable = AudioEngine.isCableInterface(selectedInput) ||
            AudioEngine.isCableInterface(selectedOutput)
        if (!cable) return
        if (!voxPrimer) voxPrimer = true
        if (txDelayMs() < 1000u) txDelayText = "1500"
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
        audio.cableTxChannel = cableTxChannel
        val parts = mutableListOf<String>()
        if (AudioEngine.isCableInterface(selectedOutput)) {
            parts += "Cable TX: VOX primer on, channel ${cableTxChannel.name}, " +
                "RX muted while sending. Hearing your own send on this phone " +
                "was cable crosstalk — only Mac receiving counts. Try L/R if needed."
        }
        if (bt) {
            val failure = router.activate(selectedInput, selectedOutput)
            if (failure != null) {
                audio.preferredInput = null
                audio.preferredOutput = null
                audio.communicationRoute = false
                router.release()
                parts += "Bluetooth routing failed: $failure. Using the default route."
            } else if (mode != ModemMode.B85) {
                parts += "Bluetooth SCO active with mode ${mode.name}. Hands-free " +
                    "codecs (CVSD/mSBC) often sync but never decode denser " +
                    "modes — switch both ends to B85 and raise TX delay."
            } else {
                parts += "Bluetooth SCO active (B85). Keep TX delay ≥ radio key-up latency."
            }
        }
        // Persistent TX track (off main thread: may sleep briefly while USB settles).
        withContext(Dispatchers.IO) { audio.openPlayback() }?.let { parts += it }
        routeStatus = parts.takeIf { it.isNotEmpty() }?.joinToString("\n")
    }

    /** Tears down audio, polling, routing, and the session (keeps settings). */
    fun disconnect() {
        pollJob?.cancel()
        pollJob = null
        audio.stopCapture()
        audio.closePlayback()
        audio.clearHalfDuplex()
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
        rejectedFrames = 0uL
        rxCounters = RxCounters(0uL, 0uL, 0uL, 0uL)
        lastSnrDb = -99f
        inputLevel = 0f
        inputPeakRaw.set(0f)
        inputRmsRaw.set(0f)
        inputRmsDbfs = Float.NEGATIVE_INFINITY
        txPeak = 0f
        transmitting = false
        progressWatch.clear()
        autoRepairRequestedIds.clear()
    }

    /**
     * Mic capture: every block feeds the modem receiver; while the user holds
     * the record button the same blocks also accumulate into the voice clip,
     * hard-capped at the configured length.
     */
    /** Starts mic capture and returns the RX route/format status line. */
    private fun startCapture(s: AetrSession): String =
        audio.startCapture { block ->
            val peak = AudioLevels.peakAbs(block)
            inputPeakRaw.updateAndGet { cur -> AudioLevels.decay(cur, peak) }
            // RMS as well as peak: the sync floor is an RMS threshold, and a
            // burst that is merely quiet looks identical to no burst at all.
            val rms = AudioLevels.rms(block)
            inputRmsRaw.updateAndGet { cur -> AudioLevels.decay(cur, rms) }
            // Half-duplex: do not decode while we are transmitting. Cable
            // TX→RX crosstalk otherwise looks like a successful self-receive.
            if (!audio.rxMuted) {
                s.pushRx(block.toList())
            }
            if (recording && !audio.rxMuted) {
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

    /**
     * Plays a 1 kHz tone out the TX path. On Mac, the In meter should jump if
     * the radio/cable path is live; if only Android's speaker clicks, Output
     * is still wrong.
     */
    fun playTestTone() {
        if (!connected) {
            errorMessage = "Connect first, then play the TX test tone"
            return
        }
        if (loopback) {
            errorMessage = "Turn off digital loopback to test the real TX cable path"
            return
        }
        audio.playTestTone()
    }

    /** Polls the core ~10 Hz for events and the RX badge state. */
    private fun startPolling(s: AetrSession) {
        pollJob = viewModelScope.launch {
            while (isActive) {
                val events = withContext(Dispatchers.Default) { s.pollEvents() }
                events.forEach(::handleEvent)
                rxState = withContext(Dispatchers.Default) { s.rxState() }
                rejectedFrames = withContext(Dispatchers.Default) { s.rejectedFrames() }
                rxCounters = withContext(Dispatchers.Default) { s.rxCounters() }
                lastSnrDb = withContext(Dispatchers.Default) { s.lastSnrDb() }
                // Sample the capture-thread envelopes onto Compose state ~10 Hz.
                inputLevel = inputPeakRaw.get()
                inputRmsDbfs = AudioLevels.dbFs(inputRmsRaw.get())
                txPeak = audio.lastTxPeak
                lastTxCapturePath = audio.lastTxCapturePath
                transmitting = audio.transmitting
                if (autoArq) maybeAutoRequestRepair()
                delay(100)
            }
        }
    }

    /**
     * If an in-flight text/voice assembly stalled (usually while Receiving),
     * fire one automatic repair request (VOX can key it without a PTT press).
     */
    private fun maybeAutoRequestRepair() {
        val now = SystemClock.elapsedRealtime()
        val stalled = progressWatch.entries.firstOrNull { (id, watch) ->
            val (at, received, total) = watch
            AutoArq.shouldAutoRequest(
                autoArqEnabled = autoArq,
                transmitting = transmitting,
                rxMuted = audio.rxMuted,
                alreadyRequested = id in autoRepairRequestedIds,
                received = received,
                total = total,
                msSinceProgress = now - at,
            )
        } ?: return
        autoRepairRequestedIds.add(stalled.key)
        requestRepair(stalled.key)
    }

    /** Folds one core event into the log. */
    private fun handleEvent(event: RxEvent) {
        when (event) {
            is RxEvent.Progress -> {
                progressWatch[event.messageId] = Triple(
                    SystemClock.elapsedRealtime(), event.received, event.total
                )
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
                clearProgressWatch(event.messageId)
                removeInProgress(event.messageId)
                log.add(
                    LogEntry.ReceivedText(
                        nextUid.getAndIncrement(), event.messageId, event.text, event.encrypted
                    )
                )
            }

            is RxEvent.Voice -> {
                clearProgressWatch(event.messageId)
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
                clearProgressWatch(event.messageId)
                removeInProgress(event.messageId)
                log.add(
                    LogEntry.Failed(
                        nextUid.getAndIncrement(), event.messageId, event.reason
                    )
                )
            }

            is RxEvent.RepairRequested -> {
                val pcm = event.pcmResponse.toFloatArray()
                if (autoArq) {
                    val uid = nextUid.getAndIncrement()
                    log.add(LogEntry.RepairRequest(uid, event.messageId, pcm))
                    // Auto-TX so VOX-only radios key without a manual PTT.
                    viewModelScope.launch {
                        delay(AutoArq.RESPONSE_DELAY_MS)
                        val s = session ?: return@launch
                        val entry = log.filterIsInstance<LogEntry.RepairRequest>()
                            .firstOrNull { it.uid == uid } ?: return@launch
                        transmit(s, entry.pcm.toList())
                        log.removeAll { it.uid == uid }
                    }
                } else {
                    log.add(
                        LogEntry.RepairRequest(
                            nextUid.getAndIncrement(),
                            event.messageId,
                            pcm,
                        )
                    )
                }
            }
        }
    }

    /** Drops the in-flight progress row for a message that just finalized. */
    private fun removeInProgress(messageId: ULong) {
        log.removeAll { it is LogEntry.InProgress && it.messageId == messageId }
    }

    private fun clearProgressWatch(messageId: ULong) {
        progressWatch.remove(messageId)
        autoRepairRequestedIds.remove(messageId)
    }

    /** Encodes and transmits the drafted text message. */
    fun sendText() {
        val text = draft.trim()
        val s = session ?: return
        if (text.isEmpty()) return
        draft = ""
        val encrypted = sendEncrypted
        viewModelScope.launch {
            try {
                val burst = withContext(Dispatchers.Default) { s.encodeText(text, encrypted) }
                log.add(LogEntry.SentText(nextUid.getAndIncrement(), text, encrypted))
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
        viewModelScope.launch {
            withContext(Dispatchers.Default) { s.resetRx() }
            rejectedFrames = 0uL
        }
    }

    /**
     * Routes an encoded burst out: digitally into our own receiver when the
     * debug loopback is on, otherwise through the speaker/cable. After a real
     * TX, flush modem sync only (keep reassembler + queued events) then unmute.
     */
    private suspend fun transmit(s: AetrSession, burst: List<Float>) {
        if (loopback) {
            withContext(Dispatchers.Default) { s.pushRx(burst) }
            return
        }
        try {
            suspendCancellableCoroutine { cont ->
                audio.play(burst.toFloatArray(), halfDuplex = true) {
                    if (cont.isActive) cont.resume(Unit)
                }
            }
            // Match macOS: brief hang so squelch/cable tail is not decoded.
            delay(400)
            audio.lastTxRouteNote?.let { note ->
                routeStatus = listOfNotNull(routeStatus, "Last TX: $note").joinToString("\n")
            }
            withContext(Dispatchers.Default) {
                s.flushRxModem()
            }
        } finally {
            audio.clearHalfDuplex()
            transmitting = false
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
