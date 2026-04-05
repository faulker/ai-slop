# ThoughtQueue

A macOS menu bar app for capturing text snippets and sending them to Claude Desktop as new conversations. Built for people who want to bookmark ideas mid-conversation and explore them later.

## Requirements

- macOS 13.0+
- Xcode 16.0+
- [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- [Claude Desktop](https://claude.ai/download) (for the "Open in Claude" feature)

## Build

```bash
git clone <repo-url>
cd txtmem
xcodegen generate
xcodebuild -project txtmem.xcodeproj -scheme txtmem -configuration Release build
```

Or open `txtmem.xcodeproj` in Xcode and hit Cmd+R.

## Setup

On first launch, ThoughtQueue will appear in your menu bar with a `"` icon. You'll be prompted to grant **Accessibility permission** in System Settings > Privacy & Security > Accessibility. This is required for global hotkeys to work.

## Usage

### Capturing text

1. **Quick capture** — Select text in any app, press **Cmd+Shift+B**. The text is saved instantly to your queue and a "Captured!" toast appears.

2. **Detailed capture** — Select text, press **Cmd+Shift+Option+B**. An overlay appears where you can edit the text and assign a category before saving.

### Managing your queue

- **Left-click** the menu bar icon to open the popover. Categories are expandable — click one to see its entries. Each entry has **Open**, **Move**, and **Delete** buttons.

- **Right-click** the menu bar icon and select **Open ThoughtQueue** for the full management window with a category sidebar and entries table.

### Sending to Claude

Click **Open** on any entry. ThoughtQueue will:

1. Copy the text to your clipboard
2. Activate Claude Desktop
3. Open a new chat (Cmd+Shift+O)
4. Paste the text (Cmd+V)
5. Clear the clipboard
6. Mark the entry with a checkmark

### Categories

- Create categories from the full management window (sidebar "+" button) or from the right-click context menu
- Move entries between categories using the **Move** button
- Deleting a category prompts you to move its entries to Uncategorized or delete them all

### Clearing completed entries

In the full management window, click **Clear Completed** to remove all entries that have been sent to Claude.

### Customizing hotkeys

Right-click the menu bar icon > **Preferences**. Click a shortcut field and press your desired key combination.

## Running tests

```bash
xcodebuild -project txtmem.xcodeproj -scheme txtmemTests test
```

## How it works

ThoughtQueue uses macOS Accessibility APIs (`CGEventTap`) to listen for global hotkeys and simulate keyboard input. Text capture works by simulating Cmd+C, reading the clipboard, then clearing it. Claude integration uses keyboard simulation to trigger Claude Desktop's native "New Chat" shortcut.

The app stores entries in a local SQLite database at `~/Library/Application Support/ThoughtQueue/thoughtqueue.db`.
