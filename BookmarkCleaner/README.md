# Bookmark Cleaner

A robust, terminal-based utility written in Rust to scan, clean, and upgrade your browser bookmarks.

## Overview

Bookmark Cleaner takes an exported HTML bookmark file (Netscape format), scans every link for validity, and provides an interactive Terminal User Interface (TUI) to review and clean up dead links. It also intelligently upgrades `http://` links to `https://` if the secure version is available.

## Features

-   **Interactive TUI**: Review dead links before deleting them.
-   **Smart HTTPS Upgrade**: Automatically upgrades `http` links to `https` if the `http` version is dead but `https` works.
-   **Dead Link Detection**: Identifies 404s, 410s, DNS errors, timeouts, and more.
-   **Robust Scanning**: Handles rate-limiting, retries with backoff, and custom user agents to minimize false positives.
-   **Selective Exclusion**:
    -   Ignore specific folders (e.g., "Archive", "Work").
    -   Ignore local/private network addresses (localhost, 192.168.x.x, etc.).
-   **Batch Selection**: Quickly mark all dead links to **Keep** or **Delete**.
-   **Safe**: Creates a new output file, leaving your original backup untouched.

## Installation

Ensure you have [Rust and Cargo installed](https://rustup.rs/).

```bash
git clone <repository-url>
cd BookmarkCleaner
cargo build --release
```

The binary will be located in `target/release/bookmark-cleaner`.

## Usage

1.  **Export your bookmarks** from your browser (Chrome, Firefox, Edge, etc.) to an HTML file (e.g., `bookmarks.html`).
2.  **Run the tool**:

```bash
cargo run --release -- --input-file bookmarks.html --output-file cleaned_bookmarks.html
```

### Command Line Arguments

| Argument | Description | Default |
| :--- | :--- | :--- |
| `-i, --input-file <PATH>` | Path to the source bookmark HTML file. | **Required** |
| `-o, --output-file <PATH>` | Path to save the cleaned/upgraded file. | Optional |
| `--ignore-local` | Ignore localhost and private IP addresses. | `false` |
| `--exclude-folder <NAME>` | Exclude bookmarks in specific folders. Can be used multiple times. | None |
| `--concurrent-requests <NUM>` | Number of concurrent requests. **Higher values may cause false positives.** | `1` |
| `--timeout <SECONDS>` | Request timeout in seconds. | `60` |
| `--retries <NUM>` | Number of retries for failed requests. | `3` |
| `--redirect-limit <NUM>` | Maximum number of redirects to follow. | `10` |
| `--ignore-ssl` | Ignore SSL certificate errors. | `false` |

### Examples

**Basic Scan (Safe Mode):**
```bash
cargo run -- --input-file bookmarks.html --output-file cleaned.html
```

**Faster Scan (May trigger rate limits):**
```bash
cargo run -- --input-file bookmarks.html --output-file cleaned.html --concurrent-requests 10
```

**Ignore Local Dev Links and Specific Folders:**
```bash
cargo run -- \
  --input-file bookmarks.html \
  --output-file cleaned.html \
  --ignore-local \
  --exclude-folder "Work Stuff" \
  --exclude-folder "Old Archives"
```

## Interactive Controls

Once the scan is complete (or while it's running), use the following keys in the TUI:

-   `Up` / `Down`: Navigate the list of dead links.
-   `Space`: Toggle selection (Keep / Delete).
-   `k`: Mark **All** dead links to **Keep**.
-   `d`: Mark **All** dead links to **Delete** (Default state).
-   `Enter`: Confirm changes. This will save the new file with selected links removed and upgraded links updated.
-   `q`: Quit without saving.

## How it Works

1.  **Parsing**: Reads the Netscape HTML format, preserving the folder structure context.
2.  **Scanning**: Checks links concurrently (limit: 1) with a custom user agent.
    -   If a link returns 200 OK -> Kept (Hidden from list).
    -   If a link fails (404/410/DNS/Timeout) -> Marked as **Dead**.
    -   **Smart Upgrade**: If an `http://` link fails, it tries `https://`. If that works, the link is automatically upgraded in the output.
3.  **Review**:
    -   The app presents a list of **Dead Links**.
    -   By default, all dead links are marked for **Deletion** (`[DEL ]`).
    -   You can toggle specific links to **Keep** (`[KEEP]`) if you believe they are false positives.
4.  **Output**: Generates a clean HTML file compatible with browser import.

## License

MIT