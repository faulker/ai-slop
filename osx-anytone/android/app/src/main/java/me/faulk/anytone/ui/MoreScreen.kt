package me.faulk.anytone.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import me.faulk.anytone.state.CodeplugSection
import me.faulk.anytone.state.CodeplugState
import me.faulk.anytone.ui.common.EmptyState

/**
 * The record families that are not one of the app's three priorities.
 *
 * Which entries appear comes from the core's `capabilities.sections` for the
 * open codeplug, so a radio with no zones simply does not show zones and adding
 * a radio to the core needs no change in this file.
 */
@Composable
fun MoreScreen(
    state: CodeplugState,
    onOpenSection: (CodeplugSection) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (!state.isOpen) {
        EmptyState(
            "No codeplug open",
            "Open one from the Radio tab to edit it.",
            modifier = modifier,
        )
        return
    }

    // Channels and the config sections have their own destinations already.
    val sections = state.sections.filterNot { it.isChannels || it.isConfig }
    if (sections.isEmpty()) {
        EmptyState(
            "Nothing else",
            "This radio's channels and settings are all it has.",
            modifier = modifier,
        )
        return
    }

    SectionIndex(
        sections = sections,
        subtitle = { countLabel(state, it) },
        hasUnsaved = { state.hasUnsavedChanges(it) },
        onOpen = onOpenSection,
        modifier = modifier,
    )
}

/** How many records a section holds, for the row's second line. */
private fun countLabel(state: CodeplugState, section: CodeplugSection): String = when (section) {
    CodeplugSection.ZONES -> "${state.zones.size} zones"
    CodeplugSection.SCAN_LISTS -> "${state.scanLists.size} scan lists"
    // The AnyTone always reports its full 10000 contact slots, so counting the
    // named ones is the number that means something to a user.
    CodeplugSection.CONTACTS -> "${state.contacts.count { it.name.isNotBlank() }} contacts"
    CodeplugSection.GROUP_LISTS -> "${state.groupLists.size} group lists"
    CodeplugSection.RADIO_IDS -> "${state.radioIds.size} radio IDs"
    else -> ""
}
