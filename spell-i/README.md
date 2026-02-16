# Spell-i

A system-wide spell checker and grammar corrector for macOS. Spell-i works across every application — text editors, browsers, terminals, Slack, IDEs — by reading text through the Accessibility API and drawing squiggly underlines on a transparent overlay. Click an underline to see suggestions and apply corrections.

Powered by [Harper](https://github.com/Automattic/harper), an open-source grammar checking engine written in Rust. Everything runs locally — no cloud, no accounts, no data leaves your machine.

## How It Works

Spell-i runs as a menu bar app (no Dock icon). Under the hood:

1. **Keystroke detection** — A `CGEventTap` listens for keyDown events system-wide (listen-only, never blocks input).
2. **Debouncing** — After you stop typing for 400ms, a lint pass is triggered.
3. **Text reading** — The Accessibility API reads the full text content of the focused text field.
4. **Linting** — Text is sent to the Harper engine (via Rust FFI) which returns spelling and grammar errors with suggestions.
5. **Overlay rendering** — A transparent borderless window draws squiggly underlines at the screen positions of each error (red for spelling, blue for grammar).
6. **Click-to-correct** — Clicking an underline shows a popup with suggestions. Select one to replace the word in-place via the Accessibility API.

### Architecture

```
┌─────────────┐     ┌──────────────┐     ┌───────────────────┐
│ CGEventTap  │────▸│   Debouncer  │────▸│  AX Text Reader   │
│ (keystrokes)│     │   (400ms)    │     │  (focused element) │
└─────────────┘     └──────────────┘     └────────┬──────────┘
                                                   │ text
                                              ┌────▼────┐
                                              │  Harper  │
                                              │  (Rust)  │
                                              └────┬────┘
                                                   │ lint results
                    ┌──────────────┐     ┌─────────▼──────────┐
                    │  Correction  │◂────│  Overlay Window     │
                    │  Popup       │     │  (squiggly lines)   │
                    └──────────────┘     └─────────────────────┘
```

The Rust engine (`spell-i-engine/`) is compiled as a static library and linked via [swift-bridge](https://github.com/niccolocchelli/swift-bridge) FFI. The bridge is generated at build time.

## Requirements

- macOS 13.0 (Ventura) or later
- Apple Silicon (aarch64) — the build script targets `aarch64-apple-darwin`
- Xcode 15+
- Rust toolchain (`rustup` with the `aarch64-apple-darwin` target)

## Building

### 1. Install Rust (if needed)

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add aarch64-apple-darwin
```

### 2. Generate the Xcode project

The project uses [XcodeGen](https://github.com/yonaskolb/XcodeGen) to generate the `.xcodeproj` from `project.yml`. Install it if needed, then generate:

```sh
brew install xcodegen
xcodegen generate
```

### 3. Build and run

**From the command line:**

```sh
# Build (Debug)
xcodebuild -project Spell-i.xcodeproj -scheme Spell-i -configuration Debug build SYMROOT="$(pwd)/build"

# Launch
open build/Debug/Spell-i.app

# Run Swift tests
xcodebuild -project Spell-i.xcodeproj -scheme Spell-i -configuration Debug test

# Run Rust tests
cd spell-i-engine && cargo test
```

**From Xcode:**

Open `Spell-i.xcodeproj` and press Cmd+R. The Rust library is built automatically via a pre-build script phase that invokes `build-rust.sh` before compiling Swift sources.

The build phase handles compiling the Rust crate, copying `libspell_i_engine.a`, and generating the Swift/C bridge files into `Generated/`.

## Usage

1. **Launch Spell-i** — a menu bar icon (Aa) appears. No Dock icon.
2. **Grant Accessibility permission** — on first launch, an onboarding window guides you to System Settings > Privacy & Security > Accessibility. The app polls for permission and proceeds automatically once granted.
3. **Type in any app** — after a brief pause, misspelled words get red squiggly underlines and grammar issues get blue ones.
4. **Click an underline** — a popup appears with suggestions. Click a suggestion to replace the word.
5. **Add to Dictionary** — in the popup, click "Add to Dictionary" to teach Spell-i a new word. It persists across sessions in `~/Library/Application Support/Spell-i/dictionary.txt`.
6. **Ignore** — click "Ignore" to dismiss an underline for the current session only.
7. **Toggle on/off** — click the menu bar icon and toggle "Enable Spell-i".
8. **Quit** — menu bar icon > Quit Spell-i.

## Project Structure

```
spell-i/
├── spell-i-engine/          # Rust crate (Harper FFI wrapper)
│   ├── Cargo.toml
│   ├── build.rs             # swift-bridge codegen
│   └── src/
│       ├── lib.rs           # SpellEngine + LintResults FFI
│       └── user_dict.rs     # Plain-text user dictionary I/O
├── Generated/               # Auto-generated bridge files
├── build-rust.sh            # Xcode pre-build script
├── project.yml              # XcodeGen project spec
├── Spell-i/
│   ├── App/                 # AppDelegate, StatusBarController
│   ├── TextMonitoring/      # EventTap, AX reader, debouncer, coordinator
│   ├── Overlay/             # Window, squiggly view, popup, text replacer
│   ├── Permissions/         # AX permission checker, onboarding
│   ├── Utilities/           # Constants, Logger
│   └── BridgingHeader.h
├── Spell-iTests/            # Unit tests
└── Spell-i.xcodeproj/       # Generated by xcodegen
```

## Privacy & Security

- **No network access** — all processing is local via Harper.
- **No data collection** — text is read from the focused element, linted in-process, and discarded.
- **Accessibility API** — required to read text from other apps and insert corrections. The app sandbox is disabled to allow this.
- **CGEventTap** — listen-only (`.listenOnly`), never modifies or blocks keyboard events.

## License

This project uses [Harper](https://github.com/Automattic/harper) (Apache-2.0) for grammar and spell checking.
