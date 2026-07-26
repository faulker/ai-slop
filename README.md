# AI Slop
This is a AI Slop catch all repo that I created as a place to put all the random tools I have AI create for one reason or another.

If I feel a tool is production ready or I find I'm using more then just a one off I will probably move it into it's own repo.

## Apps

- **[aetr](aetr/README.md)** — Encrypted text and voice sent as COFDM audio bursts through analog FM radios. Rust core with Swift macOS and Kotlin Android apps; shared crypto/FEC/modem work via UniFFI.

- **[ai-hardware-eval](ai-hardware-eval/README.md)** — Hardware evaluation experiment.

- **[AudioMerge](AudioMerge/README.md)** — Rust CLI that recursively merges MP3 files from subdirectories into consolidated audio files, useful for audiobooks and fragmented collections.

- **[BookmarkCleaner](BookmarkCleaner/README.md)** — Rust TUI tool that scans exported browser bookmark files for dead links, lets you review and remove them, and auto-upgrades HTTP links to HTTPS.

- **[claude-usage](claude-usage/README.md)** — Rust CLI that pulls Claude Code usage data from the Anthropic API via the OAuth token stored in the macOS Keychain.

- **[obd2-writer](obd2-writer/README.md)** — Rust CLI and TUI for reading/writing to a 2023 Toyota Tacoma's ECUs over Bluetooth using an OBDLink MX+ scanner, with live dashboards, DID scanning, and backup/restore.

- **[orcha-ai](orcha-ai/README.md)** — TypeScript CLI that orchestrates multiple Claude Code agents from a markdown spec, parsing tasks into a DAG and running them in parallel.

- **[osx-anytone](osx-anytone/README.md)** — Rust tool for reading and writing codeplug settings to Anytone digital radios via USB, with backup/restore and editing capabilities.

- **[Spell-i](spell-i/README.md)** — macOS menu bar app that provides system-wide spell and grammar checking across all apps using the Accessibility API and the Harper engine via Rust FFI.

- **[stash-mgr](stash-mgr/README.md)** — Rust TUI for managing git stashes with live diff previews, selective file stashing, and vim keybindings.
