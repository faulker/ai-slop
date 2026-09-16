package me.faulk.anytone.ui.channels

import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.RowScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import me.faulk.anytone.model.Capabilities
import me.faulk.anytone.model.DumpChannel
import me.faulk.anytone.model.formatMhz
import me.faulk.anytone.ui.common.FilterRow
import me.faulk.anytone.ui.common.RecordListScreen
import me.faulk.anytone.ui.common.SortOption

/**
 * The AnyTone channel list.
 *
 * Ported from `ChannelsPane` (macOS `CodeplugView.swift`). The desktop's sortable
 * table becomes [RecordListScreen]'s two-line rows; the mode / color-code / slot
 * filter row survives as [ChannelFilterBar]. Bulk Edit leads the selection
 * action bar, matching the priority the toolbar gives it on macOS.
 *
 * Selection and every callback here are addressed by SLOT INDEX (`DumpChannel.index`),
 * never list position, so filtering or sorting this screen's rows can never send
 * an edit or delete to the wrong record.
 */
@Composable
fun ChannelsScreen(
    channels: List<DumpChannel>,
    capabilities: Capabilities?,
    isUnsaved: (Int) -> Boolean,
    selection: Set<Int>,
    onSelectionChange: (Set<Int>) -> Unit,
    onOpen: (DumpChannel) -> Unit,
    onAdd: () -> Unit,
    onBulkEdit: (Set<Int>) -> Unit,
    onDelete: (Set<Int>) -> Unit,
    onBack: (() -> Unit)?,
    modifier: Modifier = Modifier,
    actions: @Composable RowScope.() -> Unit = {},
) {
    var filterMode by remember { mutableStateOf<String?>(null) }
    var filterColorCode by remember { mutableStateOf<Int?>(null) }
    var filterSlot by remember { mutableStateOf<Int?>(null) }

    val filtered = remember(channels, filterMode, filterColorCode, filterSlot) {
        channels.filter { channelMatchesFilter(it, filterMode, filterColorCode, filterSlot) }
    }

    RecordListScreen(
        title = "Channels",
        items = filtered,
        slotOf = DumpChannel::index,
        headline = ::channelHeadline,
        supporting = ::channelSummary,
        isUnsaved = { isUnsaved(it.index) },
        searchMatch = { c, q -> c.name.contains(q, ignoreCase = true) },
        selection = selection,
        onSelectionChange = onSelectionChange,
        onOpen = onOpen,
        onBack = onBack,
        modifier = modifier,
        actions = actions,
        sorts = listOf(
            SortOption("Slot", compareBy { it.index }),
            SortOption("Name", compareBy { it.name }),
            SortOption("RX Frequency", compareBy { it.rxFrequencyHz }),
            SortOption("TX Frequency", compareBy { it.txFrequencyHz }),
            SortOption("Mode", compareBy { it.mode }),
        ),
        onAdd = onAdd,
        emptyMessage = "No channels yet.",
        filterBar = {
            ChannelFilterBar(
                mode = filterMode,
                onModeChange = { filterMode = it },
                colorCode = filterColorCode,
                onColorCodeChange = { filterColorCode = it },
                slot = filterSlot,
                onSlotChange = { filterSlot = it },
            )
        },
        selectionActions = { sel -> ChannelSelectionActions(sel, onBulkEdit, onDelete) },
    )
}

/** Selection action bar: Bulk Edit first, matching the priority the toolbar gives it on macOS. */
@Composable
private fun ChannelSelectionActions(
    selection: Set<Int>,
    onBulkEdit: (Set<Int>) -> Unit,
    onDelete: (Set<Int>) -> Unit,
) {
    IconButton(onClick = { onBulkEdit(selection) }, enabled = selection.size >= 2) {
        Icon(Icons.Default.Edit, contentDescription = "Bulk Edit")
    }
    IconButton(onClick = { onDelete(selection) }) {
        Icon(Icons.Default.Delete, contentDescription = "Delete")
    }
}

/** Mode / color-code / slot filters, plus a Clear action once any is set. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChannelFilterBar(
    mode: String?,
    onModeChange: (String?) -> Unit,
    colorCode: Int?,
    onColorCodeChange: (Int?) -> Unit,
    slot: Int?,
    onSlotChange: (Int?) -> Unit,
) {
    FilterRow {
        FilterMenu(
            label = mode?.let { pickerLabel(it, channelModeOptions) } ?: "All modes",
        ) { close ->
            DropdownMenuItem(text = { Text("All modes") }, onClick = { onModeChange(null); close() })
            channelModeOptions.forEach { opt ->
                DropdownMenuItem(
                    text = { Text(opt.label) },
                    onClick = { onModeChange(opt.tag); close() },
                )
            }
        }

        FilterMenu(label = colorCode?.let { "CC $it" } ?: "All color codes") { close ->
            DropdownMenuItem(
                text = { Text("All color codes") },
                onClick = { onColorCodeChange(null); close() },
            )
            (0..15).forEach { cc ->
                DropdownMenuItem(text = { Text("CC $cc") }, onClick = { onColorCodeChange(cc); close() })
            }
        }

        FilterMenu(label = slot?.let { "Slot $it" } ?: "All slots") { close ->
            DropdownMenuItem(text = { Text("All slots") }, onClick = { onSlotChange(null); close() })
            DropdownMenuItem(text = { Text("Slot 1") }, onClick = { onSlotChange(1); close() })
            DropdownMenuItem(text = { Text("Slot 2") }, onClick = { onSlotChange(2); close() })
        }

        if (mode != null || colorCode != null || slot != null) {
            TextButton(onClick = {
                onModeChange(null)
                onColorCodeChange(null)
                onSlotChange(null)
            }) { Text("Clear") }
        }
    }
}

/** One filter chip that opens a dropdown menu of choices when tapped. */
@Composable
internal fun FilterMenu(label: String, items: @Composable ColumnScope.(close: () -> Unit) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    FilterChip(selected = expanded, onClick = { expanded = true }, label = { Text(label) })
    DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
        items { expanded = false }
    }
}

/** The row's first line: the name, then both frequencies in parentheses. */
fun channelHeadline(c: DumpChannel): String =
    "${c.name} (Rx ${formatMhz(c.rxFrequencyHz)} MHz / Tx ${formatMhz(c.txFrequencyHz)} MHz)"

/**
 * The row's second line: mode, then the fields that only mean something for
 * that mode. A digital channel's colour code and time slot are what tell two
 * otherwise identical repeater channels apart, and an analog one has neither,
 * so printing them unconditionally would fill the line with noise.
 */
fun channelSummary(c: DumpChannel): String = buildList {
    add(pickerLabel(c.mode, channelModeOptions))
    if (modeHasDigital(c.mode)) {
        add("CC ${c.colorCode}")
        add("TS ${c.timeSlot}")
    }
    add(c.power)
    add(c.bandwidth)
    if (c.scanListIndex == LIST_REFERENCE_NONE) add("Scan: Skip")
    if (c.rxOnly) add("RX only")
}.joinToString("  •  ")
