# aetr

Encrypted text and voice over the audio of analog FM radios. Type a message or record a clip, aetr turns it into an encrypted COFDM audio burst, and any cheap FM radio (FRS/GMRS/ham) carries it. Receivers with the same passphrase and mode decode it; everyone else hears modem noise.

> ⚠️ **Legal warning.** Transmitting encrypted or otherwise obscured content over public radio services is illegal in many jurisdictions. In the United States, the FCC prohibits messages "encoded for the purpose of obscuring their meaning" on the Amateur service (47 CFR 97.113(a)(4)) and on FRS/GMRS and most other Part 95 personal radio services. Similar rules apply in Canada, the UK, the EU, and elsewhere. Using aetr on those bands can expose you to fines or license revocation. You are responsible for knowing and following your local regulations; only transmit on frequencies and services where encrypted traffic is actually permitted (for example, certain experimental, licensed, or private allocations).

Two apps, one core:

- **core/** (Rust): everything that matters. Argon2id key derivation, XChaCha20-Poly1305 per-chunk AEAD, framing/chunking, Reed-Solomon FEC, codec2 voice coding, and the COFDM modem (vendored [aicodix](https://github.com/aicodix/modem) C++ headers behind a small `extern "C"` shim). Exposed to both apps via UniFFI.
- **macos/** (Swift/SwiftUI): chat-style window, device pickers, hold-to-record voice, AVAudioEngine at 48 kHz.
- **android/** (Kotlin/Compose, `me.faulk.aetr`): same UI mirrored, AudioRecord/AudioTrack at 48 kHz, minSdk 29.

The platform apps only move PCM and pixels. All crypto and DSP is Rust/C++ inside the core, so both platforms are bit-identical by construction (and verified by golden vectors, below).

## Protocol summary

| Aspect | Choice |
|---|---|
| Modem | aicodix COFDM at 48 kHz; payload modes 85/128/170 bytes per burst (default 170) |
| KDF | Argon2id, m=19456 KiB, t=2, p=1, fixed salt `aetr-v1` |
| Encryption | XChaCha20-Poly1305 per chunk; 11-byte header as AAD |
| Header (11 B) | `message_id(8)` `chunk_index(1)` `chunk_count(1)` `flags(1)` (repair / voice / last / control bits) |
| Nonce | `message_id \|\| chunk_index`, zero-padded to 24 B, never transmitted |
| Text FEC | K source chunks + ceil(0.25 K) Reed-Solomon parity, interleaved; none for single-chunk |
| Voice | codec2 1200 bps, ~0.95 s per chunk, no FEC; lost chunks play as silence at the right offset |
| ARQ | Receiver-initiated, one round: control frame with a received-shard bitmask; sender answers from a cached parity pool |
| Usable payload | 143 B per chunk in 170-mode (header + 16 B tag overhead) |

Both ends must share the passphrase and the modem mode.

## Voice cap and airtime

Voice clips are capped at 30 s by default (configurable per session). Airtime is roughly 1.1 s per second of voice in 170-mode and grows in the robust modes, so the UIs show an airtime estimate (via `estimate_airtime_secs`) as you raise the cap or pick a smaller mode. Longer bursts also raise the odds of a mid-transmission hit; voice degrades gracefully (silence gaps), text falls back on RS parity and then the ARQ round.

## TX key-up delay

Radios take a moment to start transmitting after audio appears (Bluetooth radios often need around a second). Every transmission therefore starts with a configurable wait, "TX delay" in both UIs (default 1000 ms, 0-5000), before the first data burst, on top of the fixed 0.1 s lead-in. If your radio keys on audio energy (VOX) rather than on stream activity, enable the "VOX primer tone" toggle: the delay is then filled with a quiet 700 Hz tone so the radio opens the transmitter during the delay instead of clipping the start of the data. The airtime estimates include the delay. Both settings live in `SessionConfig` (`tx_delay_ms`, `vox_primer`) and apply to messages, repair requests, and repair responses alike.

## Bluetooth-connected radios

Radios that carry TX/RX audio over Bluetooth work as audio devices:

- **macOS**: pair the radio, then pick it in the app's input/output device pickers. The engine converts between the device rate and the core's 48 kHz automatically.
- **Android**: pick the radio in the in-app device pickers. The app routes audio to Bluetooth SCO (`setCommunicationDevice` on Android 12+, `startBluetoothSco` on 10/11) and needs the `BLUETOOTH_CONNECT` runtime permission on Android 12+.

Two caveats: Bluetooth hands-free links are narrowband and use lossy speech codecs (CVSD/mSBC), which can degrade the denser modem modes — prefer the robust 85-byte mode over Bluetooth and step up only if it holds. And set the TX delay to cover your radio's Bluetooth key-up latency (start at 1000 ms).

## Threat model, briefly

A single static passphrase is the only secret. That means: a fixed KDF salt (no per-message salt is possible), no forward secrecy, and anyone who ever learns the phrase can decrypt everything ever sent with it, past and future. Traffic is authenticated (tampered or wrong-passphrase frames drop silently) but transmissions are trivially observable and jammable; this is confidentiality on an open channel, not stealth. Also check your local regulations: encrypted content is not legal on all radio services.

Licensing note: the `codec2` crate is LGPL-2.1 and is statically linked. Fine for personal use; if you ever distribute binaries, the LGPL relink obligation applies. The vendored aicodix modem headers are 0BSD.

## Setup

Common: Rust (stable) via rustup, Xcode + `xcodegen` (`brew install xcodegen`) for macOS.

Android, one-time machine setup (SDK/NDK assumed at `~/Library/Android/sdk`, NDK 28.2.13676358, platforms 34-36; missing pieces below):

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"  # no system JDK
export ANDROID_HOME="$HOME/Library/Android/sdk"
brew install gradle    # only to run `gradle wrapper` once; builds use ./gradlew after
```

**JDK 17 is required for the Android build.** Gradle 8.14.2 and the Android toolchain cannot parse newer JDKs (e.g. JDK 26 fails with `IllegalArgumentException: 26.0.1`). Use Android Studio's bundled JBR (above) or install one with `brew install openjdk@17`. The top-level `build.sh` auto-detects a JDK 17 and pins `JAVA_HOME` for the Android step, so `./build.sh --android` works even when your default `java` is newer; `./gradlew` directly needs `JAVA_HOME` pointed at a JDK 17 yourself.

## Build

```sh
# Core (from aetr/)
cargo build -p aetr-core --release

# Regenerate UniFFI bindings after changing core/src/api.rs
scripts/gen-bindings.sh

# macOS app -> macos/build/Debug/Aetr.app
cd macos && ./build.sh          # or ./build.sh Release

# Android APK -> android/app/build/outputs/apk/debug/app-debug.apk
cd android && ./gradlew :app:assembleDebug
```

## Test

```sh
# Core: full suite (run in release; the DSP tests are slow in debug)
cargo test -p aetr-core --release

# macOS: headless digital loopback + golden wav decode
macos/build/Debug/Aetr.app/Contents/MacOS/Aetr --selftest

# macOS: decode a specific golden vector
macos/build/Debug/Aetr.app/Contents/MacOS/Aetr --decode-golden testdata/golden_text.wav

# Android: host-JVM tests through the real core via JNA
# (needs the host cdylib: run scripts/gen-bindings.sh first; tests skip if absent)
cd android && ./gradlew :app:testDebugUnitTest
```

Cross-platform golden vectors live in `testdata/`: `golden_text.wav` and `golden_voice.wav` are deterministic encrypted bursts (fixed passphrase `correct horse battery staple`, fixed message ids) written by the core tests. The core, the macOS selftest, and the Android `GoldenWavTest` all decode the same files, proving identical behavior on every platform. `vectors.json` holds the KDF/nonce/header known answers. If you intentionally change the encoder, delete the wavs and rerun the core tests to regenerate.

For manual radio testing, follow `docs/OTA-TEST-CHECKLIST.md`.

## Docs

- `docs/BUILD-PLAN.md`: design decisions and milestones
- `docs/research-protocols.md`, `docs/research-error-correction.md`: background research
- `docs/OTA-TEST-CHECKLIST.md`: manual over-the-air test procedure
