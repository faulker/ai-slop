package me.faulk.anytone.ui.channels

import androidx.compose.foundation.layout.RowScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material3.DropdownMenuItem
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
import me.faulk.anytone.model.DumpQytChannel
import me.faulk.anytone.model.formatMhz
import me.faulk.anytone.ui.common.FilterRow
import me.faulk.anytone.ui.common.RecordListScreen
import me.faulk.anytone.ui.common.SortOption

/**
 * The QYT KT-5000 channel list.
 *
 * Ported from `QytChannelsPane` (macOS `QytChannelsPane.swift`). There is no
 * mode / color-code / slot filter bar here, because this radio is analog-only
 * and has none of those fields. What it does have is per-channel scan
 * membership, and a list of a few hundred channels is unreadable when you are
 * trying to see which ones the scanner will skip, so that is the one filter.
 *
 * Selection and every callback are addressed by SLOT INDEX (`DumpQytChannel.index`),
 * never list position, so sorting or searching this screen cannot send an edit
 * or delete to the wrong record.
 */
@Composable
fun QytChannelsScreen(
    channels: List<DumpQytChannel>,
    isUnsaved: (Int) -> Boolean,
    selection: Set<Int>,
    onSelectionChange: (Set<Int>) -> Unit,
    onOpen: (DumpQytChannel) -> Unit,
    onAdd: () -> Unit,
    onBulkEdit: (Set<Int>) -> Unit,
    onDelete: (Set<Int>) -> Unit,
    onBack: (() -> Unit)?,
    modifier: Modifier = Modifier,
    actions: @Composable RowScope.() -> Unit = {},
) {
    var scanFilter by remember { mutableStateOf(ScanFilter.All) }

    val filtered = remember(channels, scanFilter) {
        channels.filter { scanFilter.matches(it) }
    }

    RecordListScreen(
        title = "Channels",
        items = filtered,
        slotOf = DumpQytChannel::index,
        headline = ::qytChannelHeadline,
        supporting = ::qytChannelSummary,
        isUnsaved = { isUnsaved(it.index) },
        searchMatch = { c, q -> qytChannelMatchesFilter(c, q) },
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
            SortOption("TX Frequency", compareBy { it.txFrequencyHz ?: 0L }),
            SortOption("Power", compareBy { it.power }),
        ),
        onAdd = onAdd,
        emptyMessage = "No channels yet.",
        filterBar = {
            QytChannelFilterBar(scanFilter) { scanFilter = it }
        },
        selectionActions = { sel ->
            IconButton(onClick = { onBulkEdit(sel) }, enabled = sel.size >= 2) {
                Icon(Icons.Default.Edit, contentDescription = "Bulk Edit")
            }
            IconButton(onClick = { onDelete(sel) }) {
                Icon(Icons.Default.Delete, contentDescription = "Delete")
            }
        },
    )
}

/**
 * Show every channel, only the scanned ones, or only the skipped ones.
 *
 * Three explicit states rather than a "hide skipped" toggle, because the
 * question is asked both ways round: "what will the scanner actually stop on"
 * and "which channels did I skip, so I can unskip one".
 */
enum class ScanFilter(val label: String) {
    All("All channels"),
    Scanned("Scanned only"),
    Skipped("Skipped only");

    /** Whether a channel belongs in this filter's list. */
    fun matches(channel: DumpQytChannel): Boolean = when (this) {
        All -> true
        Scanned -> channel.scanAdd
        Skipped -> !channel.scanAdd
    }
}

/** The scan-membership filter, plus a Clear action once it is narrowed. */
@Composable
private fun QytChannelFilterBar(selected: ScanFilter, onChange: (ScanFilter) -> Unit) {
    FilterRow {
        FilterMenu(label = selected.label) { close ->
            ScanFilter.entries.forEach { option ->
                DropdownMenuItem(
                    text = { Text(option.label) },
                    onClick = { onChange(option); close() },
                )
            }
        }
        if (selected != ScanFilter.All) {
            TextButton(onClick = { onChange(ScanFilter.All) }) { Text("Clear") }
        }
    }
}

/** The row's first line: the name, then both frequencies in parentheses. */
fun qytChannelHeadline(c: DumpQytChannel): String =
    "${c.name} (Rx ${formatMhz(c.rxFrequencyHz)} MHz / Tx ${formatOptionalMhz(c.txFrequencyHz)} MHz)"

/**
 * The row's second line: scan membership first, then both tones as values (not
 * table indices, since a QYT channel stores a tone directly), power and
 * bandwidth.
 *
 * Scan membership is spelled out in both directions rather than only marking
 * the exceptional "Skip", because the list has a filter for it now and a row
 * that says nothing is ambiguous once the list has been narrowed.
 */
fun qytChannelSummary(c: DumpQytChannel): String {
    val scan = if (c.scanAdd) "Scan: Yes" else "Scan: Skip"
    val tones = "${toneLabel(c.rxTone)}/${toneLabel(c.txTone)}"
    return "$scan  •  $tones  •  ${c.power}  •  ${c.bandwidth}"
}
