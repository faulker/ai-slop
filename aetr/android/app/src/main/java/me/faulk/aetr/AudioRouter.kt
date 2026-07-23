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
    }

    private val appContext = context.applicationContext
    private val audioManager =
        appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager

    private var scoReceiver: BroadcastReceiver? = null
    private var deviceCallback: AudioDeviceCallback? = null
    private var routed = false

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
     * Undoes any Bluetooth routing and receiver registration. Safe to call
     * when no route is active or repeatedly.
     */
    @Suppress("DEPRECATION")
    fun release() {
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
