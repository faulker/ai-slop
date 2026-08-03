package me.faulk.aetr

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Enumerates candidate audio devices and owns the platform-level route
 * switch needed for Bluetooth radios that carry TX/RX audio over SCO.
 *
 * Per-stream pinning (AudioRecord/AudioTrack.setPreferredDevice) lives in
 * [AudioEngine]; this class handles the part setPreferredDevice cannot: the
 * SCO link itself, which only comes up asynchronously after a request. On
 * API 31+ that request is AudioManager.setCommunicationDevice; on API 29-30
 * it is the legacy startBluetoothSco() + MODE_IN_COMMUNICATION dance.
 */
class AudioRouter(context: Context) {

    companion object {
        /** How long to wait for an asynchronous Bluetooth SCO route to come up. */
        private const val ROUTE_TIMEOUT_MS = 4_000L

        /** True for device types carried over a Bluetooth link. */
        fun isBluetooth(device: AudioDeviceInfo): Boolean =
            device.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO ||
                device.type == AudioDeviceInfo.TYPE_BLUETOOTH_A2DP ||
                device.type == AudioDeviceInfo.TYPE_BLE_HEADSET

        /** Human-readable transport label for an AudioDeviceInfo type constant. */
        fun typeLabel(type: Int): String = when (type) {
            AudioDeviceInfo.TYPE_BUILTIN_MIC -> "Built-in mic"
            AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "Speaker"
            AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> "Earpiece"
            AudioDeviceInfo.TYPE_WIRED_HEADSET -> "Wired headset"
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES -> "Wired headphones"
            AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "Bluetooth (SCO)"
            AudioDeviceInfo.TYPE_BLUETOOTH_A2DP -> "Bluetooth (A2DP)"
            AudioDeviceInfo.TYPE_BLE_HEADSET -> "Bluetooth (LE)"
            AudioDeviceInfo.TYPE_USB_DEVICE -> "USB"
            AudioDeviceInfo.TYPE_USB_HEADSET -> "USB headset"
            AudioDeviceInfo.TYPE_DOCK -> "Dock"
            else -> "Type $type"
        }

        /** Picker label: product name plus transport type. */
        fun label(device: AudioDeviceInfo): String {
            val name = device.productName?.toString()?.takeIf { it.isNotBlank() }
                ?: "Unknown device"
            return "$name · ${typeLabel(device.type)}"
        }

        /** Built-in speaker/earpiece — wrong TX sink when an APRS cable is the mic. */
        fun isBuiltinSink(device: AudioDeviceInfo?): Boolean =
            device != null && isBuiltinSinkType(device.type)

        fun isBuiltinSinkType(type: Int): Boolean =
            type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER ||
                type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE

        /** Built-in mic — wrong RX source when an APRS cable should feed the modem. */
        fun isBuiltinMic(device: AudioDeviceInfo?): Boolean =
            device?.type == AudioDeviceInfo.TYPE_BUILTIN_MIC

        /**
         * Finds the playback half of a cable/USB interface chosen as capture.
         * Android enumerates USB input and output as separate device ids; leaving
         * output on the speaker is the usual reason Mac→Android works but
         * Android→Mac does not.
         */
        fun matchOutputForInput(
            input: AudioDeviceInfo,
            outputs: List<AudioDeviceInfo>,
        ): AudioDeviceInfo? {
            val cableOuts = outputs.filter { AudioEngine.isCableInterface(it) }
            if (cableOuts.isEmpty()) return null
            val inName = input.productName?.toString()
            val inAddr = input.address
            return cableOuts.firstOrNull { it.id == input.id }
                ?: cableOuts.firstOrNull { inAddr.isNotEmpty() && it.address == inAddr }
                ?: cableOuts.firstOrNull { inName != null && it.productName?.toString() == inName }
                ?: cableOuts.singleOrNull()
        }
    }

    private val appContext = context.applicationContext
    private val audioManager =
        appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager

    private var scoReceiver: BroadcastReceiver? = null
    private var deviceCallback: AudioDeviceCallback? = null
    private var routed = false
    /** True while a temporary cable/USB TX communication route is held. */
    private var cableTxRouted = false
    private var modeBeforeCableTx = AudioManager.MODE_NORMAL

    /** Capture-capable devices worth offering in the input picker. */
    fun inputDevices(): List<AudioDeviceInfo> =
        audioManager.getDevices(AudioManager.GET_DEVICES_INPUTS)
            .filter {
                it.type != AudioDeviceInfo.TYPE_TELEPHONY &&
                    it.type != AudioDeviceInfo.TYPE_REMOTE_SUBMIX
            }
            .sortedBy { it.type }

    /** Playback-capable devices worth offering in the output picker. */
    fun outputDevices(): List<AudioDeviceInfo> =
        audioManager.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
            .filter {
                it.type != AudioDeviceInfo.TYPE_TELEPHONY &&
                    it.type != AudioDeviceInfo.TYPE_REMOTE_SUBMIX
            }
            .sortedBy { it.type }

    /**
     * Notifies `onChanged` (on the main thread) whenever audio devices are
     * plugged or unplugged, so pickers can stay current.
     */
    fun registerDeviceCallback(onChanged: () -> Unit) {
        if (deviceCallback != null) return
        val cb = object : AudioDeviceCallback() {
            override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = onChanged()
            override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = onChanged()
        }
        deviceCallback = cb
        audioManager.registerAudioDeviceCallback(cb, Handler(Looper.getMainLooper()))
    }

    /** Unregisters the device-change callback registered above. */
    fun unregisterDeviceCallback() {
        deviceCallback?.let { audioManager.unregisterAudioDeviceCallback(it) }
        deviceCallback = null
    }

    /**
     * Applies the platform route for the chosen devices and suspends until
     * it is active or [ROUTE_TIMEOUT_MS] passes. Only Bluetooth selections
     * need anything here; wired/built-in devices route purely through
     * setPreferredDevice. Returns null on success, or a short failure
     * description for the UI status line.
     */
    suspend fun activate(input: AudioDeviceInfo?, output: AudioDeviceInfo?): String? {
        val wantsBluetooth = (input?.let(::isBluetooth) == true) ||
            (output?.let(::isBluetooth) == true)
        if (!wantsBluetooth) return null
        return if (Build.VERSION.SDK_INT >= 31) {
            activateCommunicationDevice(output ?: input!!)
        } else {
            activateLegacySco()
        }
    }

    /**
     * API 31+ path: pick the matching entry from
     * availableCommunicationDevices (the only devices setCommunicationDevice
     * accepts), request it, and poll until the platform reports it active.
     */
    private suspend fun activateCommunicationDevice(selected: AudioDeviceInfo): String? {
        val candidates = audioManager.availableCommunicationDevices
        val target = candidates.firstOrNull { it.id == selected.id }
            ?: candidates.firstOrNull { isBluetooth(it) && it.address == selected.address }
            ?: candidates.firstOrNull { it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO }
            ?: return "no SCO-capable communication device available"
        audioManager.mode = AudioManager.MODE_IN_COMMUNICATION
        if (!audioManager.setCommunicationDevice(target)) {
            audioManager.mode = AudioManager.MODE_NORMAL
            return "platform rejected ${label(target)}"
        }
        routed = true
        // The route change is asynchronous; wait until the getter reflects it.
        val active = withTimeoutOrNull(ROUTE_TIMEOUT_MS) {
            while (audioManager.communicationDevice?.id != target.id) delay(100)
            true
        } ?: false
        return if (active) null else "Bluetooth route did not become active in time"
    }

    /**
     * API 29-30 path: legacy SCO. startBluetoothSco() is asynchronous, so a
     * receiver on ACTION_SCO_AUDIO_STATE_UPDATED gates streaming until the
     * link reports connected (or the timeout passes).
     */
    @Suppress("DEPRECATION")
    private suspend fun activateLegacySco(): String? {
        if (!audioManager.isBluetoothScoAvailableOffCall) {
            return "Bluetooth SCO not available off-call on this device"
        }
        val connected = CompletableDeferred<Boolean>()
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context?, intent: Intent?) {
                val state = intent?.getIntExtra(
                    AudioManager.EXTRA_SCO_AUDIO_STATE,
                    AudioManager.SCO_AUDIO_STATE_DISCONNECTED
                )
                if (state == AudioManager.SCO_AUDIO_STATE_CONNECTED) connected.complete(true)
            }
        }
        appContext.registerReceiver(
            receiver, IntentFilter(AudioManager.ACTION_SCO_AUDIO_STATE_UPDATED)
        )
        scoReceiver = receiver
        audioManager.mode = AudioManager.MODE_IN_COMMUNICATION
        audioManager.startBluetoothSco()
        audioManager.isBluetoothScoOn = true
        routed = true
        val active = withTimeoutOrNull(ROUTE_TIMEOUT_MS) { connected.await() }
            ?: audioManager.isBluetoothScoOn
        return if (active) null else "Bluetooth SCO did not connect in time"
    }

    /**
     * Forces cable/USB playback onto the platform communication route for one
     * TX. [AudioTrack.setPreferredDevice] alone is widely ignored for USB-C
     * / TRRS interfaces, so TX stays on the speaker while RX (setPreferredDevice
     * on AudioRecord) still works — exactly Mac→Android OK / Android→Mac dead.
     *
     * Call only around a TX burst; [releaseCableTx] must run afterward so
     * MODE_IN_COMMUNICATION does not keep AEC on for RX.
     * Returns null on success, or a short warning (TX may still be attempted).
     */
    fun activateCableTx(output: AudioDeviceInfo): String? {
        if (!AudioEngine.isCableInterface(output)) return null
        if (Build.VERSION.SDK_INT >= 31) {
            val candidates = audioManager.availableCommunicationDevices
            val target = candidates.firstOrNull { it.id == output.id }
                ?: candidates.firstOrNull {
                    output.address.isNotEmpty() && it.address == output.address &&
                        AudioEngine.isCableInterface(it)
                }
                ?: candidates.firstOrNull { AudioEngine.isCableInterface(it) }
                ?: return "cable/USB not in communication devices — TX may stay on speaker"
            modeBeforeCableTx = audioManager.mode
            audioManager.mode = AudioManager.MODE_IN_COMMUNICATION
            @Suppress("DEPRECATION")
            audioManager.isSpeakerphoneOn = false
            if (!audioManager.setCommunicationDevice(target)) {
                audioManager.mode = modeBeforeCableTx
                return "platform rejected cable TX route ${label(target)}"
            }
            cableTxRouted = true
            // Brief settle; callers reopen AudioTrack after this returns.
            return null
        }
        // API 29-30: no setCommunicationDevice; at least kill speakerphone.
        @Suppress("DEPRECATION")
        audioManager.isSpeakerphoneOn = false
        return "pre-Android 12: forced speakerphone off; verify TX is not on the earpiece"
    }

    /**
     * Clears a temporary [activateCableTx] route. No-op when Bluetooth SCO
     * owns the communication device ([routed]).
     */
    @Suppress("DEPRECATION")
    fun releaseCableTx() {
        if (!cableTxRouted) {
            audioManager.isSpeakerphoneOn = false
            return
        }
        cableTxRouted = false
        if (routed) {
            // BT session still owns the communication route.
            return
        }
        if (Build.VERSION.SDK_INT >= 31) {
            runCatching { audioManager.clearCommunicationDevice() }
        }
        audioManager.mode = modeBeforeCableTx
        audioManager.isSpeakerphoneOn = false
    }

    /**
     * Undoes any Bluetooth routing and receiver registration. Safe to call
     * when no route is active or repeatedly.
     */
    @Suppress("DEPRECATION")
    fun release() {
        releaseCableTx()
        scoReceiver?.let { runCatching { appContext.unregisterReceiver(it) } }
        scoReceiver = null
        if (!routed) return
        routed = false
        if (Build.VERSION.SDK_INT >= 31) {
            runCatching { audioManager.clearCommunicationDevice() }
        } else {
            runCatching { audioManager.stopBluetoothSco() }
            audioManager.isBluetoothScoOn = false
        }
        audioManager.mode = AudioManager.MODE_NORMAL
    }
}
