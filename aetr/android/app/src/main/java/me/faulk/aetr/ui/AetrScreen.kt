package me.faulk.aetr.ui

import android.Manifest
import android.content.pm.PackageManager
import android.media.AudioDeviceInfo
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuAnchorType
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import me.faulk.aetr.AetrViewModel
import me.faulk.aetr.AudioEngine
import me.faulk.aetr.AudioRouter
import me.faulk.aetr.LogEntry
import uniffi.aetr_core.ModemMode
import uniffi.aetr_core.RxState

/**
 * The whole app on one screen: connection settings when disconnected, and
 * the message log + compose bar + hold-to-talk when connected. Mirrors the
 * macOS Aetr window.
 */
@Composable
fun AetrScreen(vm: AetrViewModel) {
    val snackbar = remember { SnackbarHostState() }

    // Surface core/audio errors as transient snackbars.
    LaunchedEffect(vm.errorMessage) {
        vm.errorMessage?.let {
            snackbar.showSnackbar(it)
            vm.errorMessage = null
        }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = { HeaderBar(vm) },
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
                .imePadding()
        ) {
            if (!vm.connected) {
                SettingsCard(vm)
            } else {
                ConnectedControls(vm)
            }
            MessageLog(vm, Modifier.weight(1f))
            if (vm.connected) {
                ComposeBar(vm)
            }
        }
    }
}

/** Top bar: app name plus the live RX state badge. */
@Composable
private fun HeaderBar(vm: AetrViewModel) {
    Surface(tonalElevation = 3.dp) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Aetr", style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.weight(1f))
            RxBadge(vm.rxState, vm.connected)
        }
    }
}

/** Colored pill showing the receiver state (Idle / Syncing / Receiving). */
@Composable
private fun RxBadge(state: RxState, connected: Boolean) {
    val (label, color) = when {
        !connected -> "Disconnected" to Color(0xFF9E9E9E)
        state == RxState.IDLE -> "Idle" to Color(0xFF4CAF50)
        state == RxState.SYNCING -> "Syncing" to Color(0xFFFF9800)
        else -> "Receiving" to Color(0xFF2196F3)
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            Modifier
                .size(10.dp)
                .background(color, CircleShape)
        )
        Spacer(Modifier.width(6.dp))
        Text(label, style = MaterialTheme.typography.labelLarge)
    }
}

/**
 * Disconnected state: passphrase, modem mode picker, audio device pickers,
 * voice cap and TX key-up delay with a live airtime estimate, and the
 * Connect button (which first ensures the RECORD_AUDIO runtime permission,
 * plus BLUETOOTH_CONNECT on API 31+ when a Bluetooth device is selected).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SettingsCard(vm: AetrViewModel) {
    val context = LocalContext.current
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { grants ->
        val micOk = grants[Manifest.permission.RECORD_AUDIO] ?: false
        val btOk = grants[Manifest.permission.BLUETOOTH_CONNECT] ?: true
        when {
            !micOk -> vm.errorMessage = "Microphone permission is required to receive"
            !btOk -> vm.errorMessage =
                "Bluetooth permission is required for the selected Bluetooth device"
            else -> vm.connect()
        }
    }

    // Devices can appear/disappear while this card is showing (BT pairing).
    LaunchedEffect(Unit) { vm.refreshDevices() }

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(16.dp),
        colors = CardDefaults.cardColors(),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(
                value = vm.passphrase,
                onValueChange = { vm.passphrase = it },
                label = { Text("Shared passphrase") },
                visualTransformation = PasswordVisualTransformation(),
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )

            Text("Modem mode", style = MaterialTheme.typography.labelLarge)
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                val modes = listOf(
                    ModemMode.B85 to "85 B (robust)",
                    ModemMode.B128 to "128 B",
                    ModemMode.B170 to "170 B (fast)",
                )
                modes.forEachIndexed { i, (m, label) ->
                    SegmentedButton(
                        selected = vm.mode == m,
                        onClick = { vm.mode = m },
                        shape = SegmentedButtonDefaults.itemShape(i, modes.size),
                    ) { Text(label, maxLines = 1) }
                }
            }

            Text("Audio devices", style = MaterialTheme.typography.labelLarge)
            DevicePicker(
                label = "Input (radio mic)",
                devices = vm.inputDevices,
                selected = vm.selectedInput,
                onSelect = { vm.selectedInput = it },
            )
            DevicePicker(
                label = "Output (radio TX)",
                devices = vm.outputDevices,
                selected = vm.selectedOutput,
                onSelect = { vm.selectedOutput = it },
            )

            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(
                    value = vm.voiceCapText,
                    onValueChange = { vm.voiceCapText = it.filter(Char::isDigit).take(3) },
                    label = { Text("Voice clip cap (seconds)") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                OutlinedTextField(
                    value = vm.txDelayText,
                    onValueChange = { vm.txDelayText = it.filter(Char::isDigit).take(4) },
                    label = { Text("TX delay (ms)") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = vm.voxPrimer, onCheckedChange = { vm.voxPrimer = it })
                Spacer(Modifier.width(8.dp))
                Text(
                    "VOX primer tone (fills the key-up delay with a quiet tone " +
                        "so VOX-keyed radios open before the data starts)",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            VoiceCapEstimate(vm)

            Button(
                onClick = {
                    // BLUETOOTH_CONNECT is only a runtime permission on 31+,
                    // and only needed when a Bluetooth device is selected.
                    val needed = buildList {
                        add(Manifest.permission.RECORD_AUDIO)
                        if (Build.VERSION.SDK_INT >= 31 && vm.bluetoothSelected()) {
                            add(Manifest.permission.BLUETOOTH_CONNECT)
                        }
                    }
                    val allGranted = needed.all {
                        ContextCompat.checkSelfPermission(context, it) ==
                            PackageManager.PERMISSION_GRANTED
                    }
                    if (allGranted) vm.connect()
                    else permissionLauncher.launch(needed.toTypedArray())
                },
                enabled = !vm.connecting && vm.passphrase.isNotEmpty(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (vm.connecting) {
                    CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(8.dp))
                    Text("Deriving key…")
                } else {
                    Text("Connect")
                }
            }
        }
    }
}

/**
 * Read-only dropdown over the candidate audio devices for one direction.
 * `null` selection means the system default route; device rows show the
 * product name plus a transport label (e.g. "Bluetooth (SCO)").
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DevicePicker(
    label: String,
    devices: List<AudioDeviceInfo>,
    selected: AudioDeviceInfo?,
    onSelect: (AudioDeviceInfo?) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    ExposedDropdownMenuBox(expanded = expanded, onExpandedChange = { expanded = it }) {
        OutlinedTextField(
            value = selected?.let(AudioRouter::label) ?: "System default",
            onValueChange = {},
            readOnly = true,
            singleLine = true,
            label = { Text(label) },
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded) },
            modifier = Modifier
                .fillMaxWidth()
                .menuAnchor(MenuAnchorType.PrimaryNotEditable),
        )
        ExposedDropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            DropdownMenuItem(
                text = { Text("System default") },
                onClick = {
                    onSelect(null)
                    expanded = false
                },
            )
            devices.forEach { device ->
                DropdownMenuItem(
                    text = { Text(AudioRouter.label(device)) },
                    onClick = {
                        onSelect(device)
                        expanded = false
                    },
                )
            }
        }
    }
}

/**
 * Shows the estimated airtime for a max-length clip at the configured cap
 * (including the TX key-up delay), plus the longer-burst risk warning the
 * protocol doc requires.
 */
@Composable
private fun VoiceCapEstimate(vm: AetrViewModel) {
    val cap = vm.voiceCapSecs()
    val airtime = vm.estimateVoiceAirtime(cap.toULong() * AudioEngine.SAMPLE_RATE.toULong())
    val estimate = if (airtime.isNaN()) {
        ""
    } else {
        " Estimated airtime for a full clip: %.1f s (includes %d ms key-up delay)."
            .format(airtime, vm.txDelayMs().toInt())
    }
    Text(
        "A $cap s clip occupies the channel for the whole transmission.$estimate " +
            "Longer bursts have a higher chance of mid-transmission loss.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/**
 * Connected state strip: session summary (mode, caps, TX delay), the audio
 * route status line when Bluetooth is involved, loopback toggle, RX reset,
 * and disconnect. Settings themselves stay locked while connected because
 * the SettingsCard is replaced by this strip.
 */
@Composable
private fun ConnectedControls(vm: AetrViewModel) {
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                "Mode ${vm.mode.name.removePrefix("B")} B · voice cap ${vm.voiceCapSecs()} s" +
                    " · TX delay ${vm.txDelayMs()} ms" +
                    if (vm.voxPrimer) " · VOX primer" else "",
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(Modifier.weight(1f))
            TextButton(onClick = { vm.resetRx() }) { Text("Reset RX") }
            OutlinedButton(onClick = { vm.disconnect() }) { Text("Disconnect") }
        }
        vm.routeStatus?.let { status ->
            Text(
                status,
                style = MaterialTheme.typography.bodySmall,
                color = if (status.startsWith("Bluetooth routing failed")) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Switch(checked = vm.loopback, onCheckedChange = { vm.loopback = it })
            Spacer(Modifier.width(8.dp))
            Text(
                "Digital loopback (debug: feed TX straight into RX)",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

/** Scrolling message log; auto-follows the newest entry. */
@Composable
private fun MessageLog(vm: AetrViewModel, modifier: Modifier = Modifier) {
    val listState = rememberLazyListState()
    LaunchedEffect(vm.log.size) {
        if (vm.log.isNotEmpty()) listState.animateScrollToItem(vm.log.size - 1)
    }
    LazyColumn(
        state = listState,
        modifier = modifier.fillMaxWidth(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(vm.log, key = { it.uid }) { entry ->
            LogRow(vm, entry)
        }
    }
}

/** Renders one log entry, aligned by direction (sent right, received left). */
@Composable
private fun LogRow(vm: AetrViewModel, entry: LogEntry) {
    val sent = entry is LogEntry.SentText || entry is LogEntry.SentVoice
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = if (sent) Arrangement.End else Arrangement.Start,
    ) {
        Card(
            modifier = Modifier.widthIn(max = 320.dp),
            shape = RoundedCornerShape(12.dp),
            colors = CardDefaults.cardColors(
                containerColor = if (sent) MaterialTheme.colorScheme.primaryContainer
                else MaterialTheme.colorScheme.surfaceVariant
            ),
        ) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                when (entry) {
                    is LogEntry.SentText -> Text(entry.text)

                    is LogEntry.SentVoice -> {
                        Label("Voice · ${clipSecs(entry.pcm)} (~%.1f s airtime)".format(entry.airtimeSecs))
                        TextButton(onClick = { vm.playClip(entry.pcm) }) { Text("Play") }
                    }

                    is LogEntry.ReceivedText -> Text(entry.text)

                    is LogEntry.ReceivedVoice -> {
                        Label("Voice · ${clipSecs(entry.pcm)}")
                        if (entry.missingSpans.isNotEmpty()) {
                            Text(
                                "Missing spans: ${entry.missingSpans.joinToString(", ")}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.error,
                            )
                            TextButton(onClick = { vm.requestRepair(entry.messageId) }) {
                                Text("Request repair")
                            }
                        }
                        TextButton(onClick = { vm.playClip(entry.pcm) }) { Text("Play") }
                    }

                    is LogEntry.InProgress -> {
                        Label(if (entry.isVoice) "Receiving voice…" else "Receiving text…")
                        LinearProgressIndicator(
                            progress = {
                                if (entry.total == 0u) 0f
                                else entry.received.toFloat() / entry.total.toFloat()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        )
                        Text(
                            "${entry.received}/${entry.total} chunks",
                            style = MaterialTheme.typography.bodySmall,
                        )
                        TextButton(onClick = { vm.requestRepair(entry.messageId) }) {
                            Text("Request repair")
                        }
                    }

                    is LogEntry.Failed -> {
                        Label("Failed")
                        Text(
                            entry.reason,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                        if (entry.messageId != 0uL) {
                            TextButton(onClick = { vm.requestRepair(entry.messageId) }) {
                                Text("Request repair")
                            }
                        }
                    }

                    is LogEntry.RepairRequest -> {
                        Label("Peer requested repair")
                        Text(
                            "A receiver is missing parts of one of your messages.",
                            style = MaterialTheme.typography.bodySmall,
                        )
                        Button(onClick = { vm.sendRepair(entry) }) { Text("Send repair") }
                    }
                }
            }
        }
    }
}

/** Small bold caption used inside log bubbles. */
@Composable
private fun Label(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        fontWeight = FontWeight.SemiBold,
    )
}

/** Formats a 48 kHz clip length as "N.N s". */
private fun clipSecs(pcm: FloatArray): String =
    "%.1f s".format(pcm.size / AudioEngine.SAMPLE_RATE.toFloat())

/** Bottom bar: text draft + Send, and the press-and-hold voice button. */
@Composable
private fun ComposeBar(vm: AetrViewModel) {
    Surface(tonalElevation = 3.dp) {
        Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
            if (vm.recording) {
                Text(
                    "Recording… %.1f s / %d s".format(vm.recordedSecs, vm.voiceCapSecs().toInt()),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(bottom = 4.dp),
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    value = vm.draft,
                    onValueChange = { vm.draft = it },
                    placeholder = { Text("Message") },
                    modifier = Modifier.weight(1f),
                    maxLines = 3,
                )
                Spacer(Modifier.width(8.dp))
                Button(onClick = { vm.sendText() }, enabled = vm.draft.isNotBlank()) {
                    Text("Send")
                }
                Spacer(Modifier.width(8.dp))
                HoldToTalkButton(vm)
            }
        }
    }
}

/**
 * Press-and-hold voice recorder: recording starts on press and the clip is
 * encoded + transmitted on release (or when the cap auto-stops it).
 */
@Composable
private fun HoldToTalkButton(vm: AetrViewModel) {
    val active = vm.recording
    Surface(
        shape = CircleShape,
        color = if (active) MaterialTheme.colorScheme.error
        else MaterialTheme.colorScheme.secondaryContainer,
        modifier = Modifier
            .size(48.dp)
            .pointerInput(Unit) {
                detectTapGestures(
                    onPress = {
                        vm.startRecording()
                        try {
                            awaitRelease()
                        } finally {
                            vm.stopRecording()
                        }
                    }
                )
            },
    ) {
        Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
            Text(if (active) "REC" else "PTT", style = MaterialTheme.typography.labelMedium)
        }
    }
}
