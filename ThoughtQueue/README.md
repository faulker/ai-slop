# ThoughtQueue

A macOS menu bar app that lets you capture text from any application and send it to [Claude Desktop](https://claude.ai/download) as a new conversation. Save ideas, code snippets, quotes, or anything else as you work -- then explore them with Claude when you're ready.

## Why

You're reading something interesting, debugging code, or researching a topic and you come across text you want to ask Claude about -- but not right now. ThoughtQueue lives in your menu bar and lets you grab that text with a hotkey, organize it into categories, and send it to Claude as a fresh conversation whenever you want.

## Features

- **Global hotkeys** -- capture selected text from any app without switching windows
- **Quick capture** -- one shortcut saves instantly, no interruption
- **Detailed capture** -- a second shortcut opens an overlay to edit the text and pick a category before saving
- **Categories** -- organize entries however you want; create, rename, move between, or delete categories
- **Claude Desktop integration** -- opens a new Claude chat and pastes the text automatically
- **Completion tracking** -- entries get a checkmark after being sent to Claude; bulk-clear completed entries
- **Local storage** -- everything stays on your machine in a SQLite database
- **Customizable hotkeys** -- change shortcuts in Preferences

## Requirements

- macOS 13.0+
- Xcode 16.0+ and [XcodeGen](https://github.com/yonaskolb/XcodeGen) (for building from source)
- [Claude Desktop](https://claude.ai/download) (for the "Open in Claude" feature)

## Install

```bash
brew install xcodegen  # if you don't have it

git clone <repo-url>
cd txtmem
./build.sh release
open build/Build/Products/Release/ThoughtQueue.app
```

Or open the project in Xcode:

```bash
xcodegen generate
open txtmem.xcodeproj
# Build and run with Cmd+R
```

To keep ThoughtQueue available, drag `ThoughtQueue.app` to your Applications folder.

## Setup

On first launch, ThoughtQueue appears in your menu bar with a `"` icon. macOS will prompt you to grant **Accessibility permission** (System Settings > Privacy & Security > Accessibility). This is required for global hotkeys and text capture to work.

## Usage

### Capture text

| Action | Default Shortcut | What happens |
|---|---|---|
| Quick capture | `Cmd+Shift+B` | Saves selected text instantly to "Uncategorized" |
| Detailed capture | `Cmd+Shift+Option+B` | Opens an overlay to edit text and choose a category |

Select text in any app, hit the shortcut, and keep working. A toast confirms the capture.

### Manage your queue

- **Left-click** the menu bar icon to open a popover with collapsible categories and quick actions (Open, Move, Delete) on each entry
- **Right-click** the menu bar icon for the full management window, preferences, or to quit

### Send to Claude

Click **Open** on any entry. ThoughtQueue will activate Claude Desktop, open a new chat, paste the text, and mark the entry as completed.

### Organize with categories

Create categories from the sidebar in the full management window or from the right-click context menu. Move entries between categories with the **Move** button. Deleting a category gives you the option to move its entries to Uncategorized or delete them.

### Clear completed entries

In the full management window, click **Clear Completed** to remove all entries that have already been sent to Claude.

### Change hotkeys

Right-click the menu bar icon > **Preferences**. Click a shortcut field and press your desired key combination.

## How it works

ThoughtQueue uses macOS Accessibility APIs (`CGEventTap`) to listen for global hotkeys and simulate keyboard input. Text capture works by simulating Cmd+C, reading the pasteboard, then restoring it. Claude integration triggers Claude Desktop's native shortcuts via keyboard simulation -- no API keys or network calls needed.

Entries are stored locally in a SQLite database at:

```
~/Library/Application Support/ThoughtQueue/thoughtqueue.db
```

## Running tests

```bash
./build.sh        # builds Debug by default
xcodebuild -project txtmem.xcodeproj -scheme txtmemTests test
```

## Tech stack

- Swift 5.9, AppKit (no SwiftUI)
- SQLite3 (C API, no ORM)
- CGEvent for hotkeys and keyboard simulation
- XcodeGen for project generation
- No external dependencies

## License

MIT
