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
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.material3.Slider
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
import me.faulk.aetr.AudioLevels
import me.faulk.aetr.AudioRouter
import me.faulk.aetr.LogEntry
import uniffi.aetr_core.ModemMode
import uniffi.aetr_core.RxCounters
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
        contentWindowInsets = WindowInsets.safeDrawing,
        topBar = { HeaderBar(vm) },
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
                .imePadding()
        ) {
            if (!vm.connected) {
                // Settings scroll; Connect stays pinned so it cannot collapse
                // into a line under a tall form + empty message log.
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    SettingsForm(vm)
                }
                ConnectBar(vm)
            } else {
                ConnectedControls(vm)
                MessageLog(vm, Modifier.weight(1f))
                ComposeBar(vm)
            }
        }
    }
}

/** Top bar: app name, RX badge, and a full-size Disconnect when connected. */
@Composable
private fun HeaderBar(vm: AetrViewModel) {
    // Custom topBar does not get status-bar insets from Scaffold automatically;
    // without this, "Aetr" sits under the system status / menu bar.
    Surface(
        tonalElevation = 3.dp,
        modifier = Modifier
            .fillMaxWidth()
            .statusBarsPadding(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Aetr", style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.weight(1f))
            Column(horizontalAlignment = Alignment.End) {
                if (vm.transmitting) {
                    Text(
                        "TX",
                        style = MaterialTheme.typography.labelLarge,
                        color = Color(0xFFF44336),
                    )
                } else {
                    RxBadge(vm.rxState, vm.connected)
                }
                if (vm.connected && vm.rejectedFrames > 0uL) {
                    Text(
                        "${vm.rejectedFrames} frame(s) rejected",
                        style = MaterialTheme.typography.labelSmall,
                        color = Color(0xFFFF9800),
                    )
                }
                if (vm.connected) {
                    RxStageSummary(vm.rxCounters, vm.lastSnrDb)
                }
            }
            if (vm.connected) {
                Button(
                    onClick = { vm.disconnect() },
                    modifier = Modifier.heightIn(min = 48.dp),
                ) {
                    Text("Configure")
                }
            }
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
 * Disconnected settings form (scrollable parent owns Connect). Passphrase,
 * mode, Input/Output device pickers, VOX/ARQ toggles, and cable TX channel.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SettingsForm(vm: AetrViewModel) {
    // Devices can appear/disappear while this form is showing (BT pairing).
    LaunchedEffect(Unit) { vm.refreshDevices() }

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
        label = "Input (radio mic / APRS cable)",
        devices = vm.inputDevices,
        selected = vm.selectedInput,
        onSelect = { vm.selectInput(it) },
    )
    DevicePicker(
        label = "Output (radio TX / APRS cable)",
        devices = vm.outputDevices,
        selected = vm.selectedOutput,
        onSelect = { vm.selectOutput(it) },
    )
    Text(
        "Pick the same cable for Input and Output. Try Left/Right TX channel if " +
            "Android→Mac stays silent.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )

    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        OutlinedTextField(
            value = vm.voiceCapText,
            onValueChange = { vm.voiceCapText = it.filter(Char::isDigit).take(3) },
            label = { Text("Voice clip cap (s)") },
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
            "VOX primer (auto on for cable). Needed so key-up delay opens TX.",
            style = MaterialTheme.typography.bodySmall,
        )
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Switch(checked = vm.autoArq, onCheckedChange = { vm.autoArq = it })
        Spacer(Modifier.width(8.dp))
        Text(
            "Auto ARQ: request/send repairs without a second PTT. Keep VOX primer on.",
            style = MaterialTheme.typography.bodySmall,
        )
    }
    Text("Cable TX channel", style = MaterialTheme.typography.labelLarge)
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        val channels = listOf(
            AudioEngine.CableTxChannel.BOTH to "L+R",
            AudioEngine.CableTxChannel.LEFT to "Left",
            AudioEngine.CableTxChannel.RIGHT to "Right",
        )
        channels.forEachIndexed { i, (ch, label) ->
            SegmentedButton(
                selected = vm.cableTxChannel == ch,
                onClick = { vm.selectCableTxChannel(ch) },
                shape = SegmentedButtonDefaults.itemShape(i, channels.size),
            ) { Text(label) }
        }
    }
    VoiceCapEstimate(vm)
}

/**
 * Sticky Connect control under the scrollable settings. Always at least 52 dp
 * tall so it cannot compress into a hairline on short screens.
 */
@Composable
private fun ConnectBar(vm: AetrViewModel) {
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

    Surface(tonalElevation = 4.dp, shadowElevation = 4.dp) {
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
            // Empty passphrase is allowed: it makes a plaintext-only session.
            enabled = !vm.connecting,
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp)
                .heightIn(min = 52.dp),
        ) {
            if (vm.connecting) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(8.dp))
                Text("Deriving key…", style = MaterialTheme.typography.titleMedium)
            } else {
                Text("Connect", style = MaterialTheme.typography.titleMedium)
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
 * Compact connected strip: session summary, route status, input meter, TX
 * test / cable channel, loopback. Disconnect lives in the top bar as Configure.
 */
@Composable
private fun ConnectedControls(vm: AetrViewModel) {
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "Mode ${vm.mode.name.removePrefix("B")} B · ${vm.txDelayMs()} ms" +
                    if (vm.voxPrimer) " · VOX" else "",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.weight(1f),
            )
            OutlinedButton(
                onClick = { vm.resetRx() },
                modifier = Modifier.heightIn(min = 40.dp),
            ) {
                Text("Reset RX")
            }
        }
        vm.routeStatus?.let { status ->
            val warn = status.startsWith("Bluetooth routing failed") ||
                status.contains("never decode denser") ||
                status.contains("Gemini/Assistant")
            Text(
                status,
                style = MaterialTheme.typography.bodySmall,
                maxLines = 3,
                color = if (warn) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
            )
        }
        InputLevelMeter(level = vm.inputLevel, transmitting = vm.transmitting)
        // RMS as well as peak: the receiver's sync floor is an RMS threshold
        // near -57 dBFS, and below it a quiet signal is indistinguishable
        // from no signal.
        Text(
            buildString {
                append("In RMS ")
                append(
                    if (vm.inputRmsDbfs.isFinite()) "%.0f dBFS".format(vm.inputRmsDbfs)
                    else "−∞ dBFS"
                )
                append("  (sync floor ≈ −57)")
                if (vm.txPeak > 0f) append("   ·   last TX peak %.2f".format(vm.txPeak))
            },
            style = MaterialTheme.typography.labelSmall,
            color = if (vm.inputRmsDbfs.isFinite() && vm.inputRmsDbfs < -57f) {
                Color(0xFFFF9800)
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
        )
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("TX level", style = MaterialTheme.typography.labelMedium)
            Slider(
                value = vm.txLevel,
                onValueChange = { vm.selectTxLevel(it) },
                valueRange = 0.1f..1.0f,
                modifier = Modifier.weight(1f),
            )
            Text(
                "%.2f".format(vm.txLevel),
                style = MaterialTheme.typography.labelSmall,
                modifier = Modifier.width(40.dp),
            )
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            OutlinedButton(
                onClick = { vm.playTestTone() },
                enabled = !vm.transmitting,
                modifier = Modifier.heightIn(min = 40.dp),
            ) {
                Text("TX test")
            }
            SingleChoiceSegmentedButtonRow(Modifier.weight(1f)) {
                listOf(
                    AudioEngine.CableTxChannel.BOTH to "L+R",
                    AudioEngine.CableTxChannel.LEFT to "L",
                    AudioEngine.CableTxChannel.RIGHT to "R",
                ).forEachIndexed { i, (ch, label) ->
                    SegmentedButton(
                        selected = vm.cableTxChannel == ch,
                        onClick = { vm.selectCableTxChannel(ch) },
                        shape = SegmentedButtonDefaults.itemShape(i, 3),
                    ) { Text(label) }
                }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Switch(checked = vm.loopback, onCheckedChange = { vm.loopback = it })
            Spacer(Modifier.width(8.dp))
            Text(
                if (vm.loopback) {
                    "Loopback ON (local only, not the radio)"
                } else {
                    "Digital loopback (debug)"
                },
                style = MaterialTheme.typography.bodySmall,
                color = if (vm.loopback) MaterialTheme.colorScheme.error
                else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Switch(checked = vm.captureTx, onCheckedChange = { vm.enableTxCapture(it) })
            Spacer(Modifier.width(8.dp))
            Text(
                // Pair this with the receiving end's capture: same burst, two
                // recordings, and the difference is what the analog path did.
                vm.lastTxCapturePath?.takeIf { vm.captureTx }
                    ?: "Save each TX burst as a WAV (debug)",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * One-line summary of where the receiver is losing bursts.
 *
 * The stages run in order, so read it left to right and stop at the first
 * count that does not carry through. Nothing shown at all means no burst has
 * reached the modem yet, which points at routing or level rather than decode.
 */
@Composable
private fun RxStageSummary(counters: RxCounters, snrDb: Float) {
    if (counters.syncCandidates == 0uL && counters.framesDecoded == 0uL) return
    val parts = mutableListOf("sync ${counters.syncCandidates}")
    if (counters.preambleFailed > 0uL) parts += "pre✗ ${counters.preambleFailed}"
    if (counters.payloadFailed > 0uL) parts += "pay✗ ${counters.payloadFailed}"
    parts += "ok ${counters.framesDecoded}"
    if (snrDb > -99f) parts += "%.0f dB".format(snrDb)
    Text(
        parts.joinToString(" · "),
        style = MaterialTheme.typography.labelSmall,
        color = if (counters.framesDecoded > 0uL) Color(0xFF4CAF50) else Color(0xFFFF9800),
    )
}

/**
 * Live RX input peak meter. Flatline while connected means the selected
 * input device is silent (wrong device, muted radio, or cable not routed).
 */
@Composable
private fun InputLevelMeter(level: Float, transmitting: Boolean = false) {
    val color = when {
        transmitting -> Color(0xFFF44336)
        level < 0.02f -> Color(0xFF9E9E9E)
        level < 0.7f -> Color(0xFF4CAF50)
        level < 0.9f -> Color(0xFFFF9800)
        else -> Color(0xFFF44336)
    }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(
            if (transmitting) "TX" else "In",
            style = MaterialTheme.typography.labelMedium,
        )
        Spacer(Modifier.width(8.dp))
        Box(
            Modifier
                .weight(1f)
                .height(14.dp)
                .background(Color(0x22000000), CircleShape)
        ) {
            Box(
                Modifier
                    .fillMaxHeight()
                    .fillMaxWidth(level.coerceIn(0f, 1f))
                    .background(color, CircleShape)
            )
        }
        Spacer(Modifier.width(8.dp))
        Text(
            AudioLevels.dbFsLabel(level),
            style = MaterialTheme.typography.labelSmall,
            modifier = Modifier.width(72.dp),
        )
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
                    is LogEntry.SentText -> {
                        Text(entry.text)
                        if (!entry.encrypted) ClearBadge()
                    }

                    is LogEntry.SentVoice -> {
                        Label("Voice · ${clipSecs(entry.pcm)} (~%.1f s airtime)".format(entry.airtimeSecs))
                        TextButton(onClick = { vm.playClip(entry.pcm) }) { Text("Play") }
                    }

                    is LogEntry.ReceivedText -> {
                        Text(entry.text)
                        if (!entry.encrypted) ClearBadge()
                    }

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

/** Badge on a text bubble that was sent or received in the clear. */
@Composable
private fun ClearBadge() {
    Text(
        "🔓 CLEAR",
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.Bold,
        color = MaterialTheme.colorScheme.error,
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
                // Encrypt-on-send toggle. Disabled (forced clear) with no passphrase.
                TextButton(
                    onClick = { vm.sendEncrypted = !vm.sendEncrypted },
                    enabled = vm.passphrase.isNotEmpty(),
                ) {
                    Text(if (vm.sendEncrypted) "🔒" else "🔓")
                }
                OutlinedTextField(
                    value = vm.draft,
                    onValueChange = { vm.draft = it },
                    placeholder = { Text("Message") },
                    modifier = Modifier.weight(1f),
                    maxLines = 3,
                )
                Spacer(Modifier.width(8.dp))
                Button(
                    onClick = { vm.sendText() },
                    enabled = vm.draft.isNotBlank(),
                    modifier = Modifier.heightIn(min = 48.dp),
                ) {
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
