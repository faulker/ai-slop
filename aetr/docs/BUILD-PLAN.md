# aetr Implementation Plan

Encrypted text + voice over analog FM radio audio. macOS (SwiftUI) + Android (Kotlin/Compose) apps sharing a Rust core that does all crypto, framing, FEC, voice coding, and modem DSP. Platform apps only move PCM and pixels.

Builds on `docs/research-protocols.md` (Step 1) and `docs/research-error-correction.md` (Step 2).

## Key Decisions

### D1: Modem = FFI-bind aicodix C++ (vendored), not a Rust port, not a v1 4FSK

Pick: vendor the aicodix `modem`/`code`/`dsp` headers (0BSD, header-only C++17, zero external deps) into the core crate, write a small C++ shim that instantiates the templates for our fixed configs and exposes `extern "C"` encode/decode, compiled by the `cc` crate in `build.rs`.

Why:
- The modem is the highest-risk component, and the hard 80% is sync (Schmidl-Cox), carrier-frequency offset, and timing recovery over real FM radios with squelch tails and AGC. aicodix solved this and is field-proven (Rattlegram on GMRS/FRS). A "simpler" Rust 4FSK v1 still has to solve sync and CFO to work on real radios, so it isn't actually simpler, and it would be throwaway.
- A Rust port of polar-coded OFDM with SCL decoding is weeks of high-risk DSP work for zero functional gain.
- Cross-compile risk is bounded and proven: the Rattlegram Android app compiles these exact headers under the NDK. `cc` picks up the NDK clang++ automatically when driven by `cargo-ndk`. Cost is linking libc++ (`-lc++` on macOS; on Android, the shared `c++_shared` with `libc++_shared.so` bundled into jniLibs), which is well-trodden.
- Hedge: the core wraps the shim behind a Rust `Modem` trait, so if the shim stalls, a Rust FSK fallback can slot in without touching framing/crypto/apps. That is the insurance policy, not the plan.

### D2: Voice codec = pure-Rust `codec2` crate (1200 bps default), no C FFI

Why: we already carry one C++ build for the modem; the codec2 C library adds a CMake cross-compile for both platforms just to gain the 700/700C modes. At 1200 bps a chunk's ~143-byte payload carries ~0.95 s of voice, so airtime is roughly 1:1 with clip length in the 170-byte modem mode, which is acceptable. Losing 700 bps costs ~40% airtime on voice; buy it back later via higher-order modem modes (QAM on strong links) or add C FFI in v2 if field use demands it. Both the crate and the C lib are LGPL-2.1, so licensing is a wash.

### D3: FFI surface = UniFFI (proc-macro), not hand-written C ABI + JNI

Why: two target languages. One Rust definition generates Swift and Kotlin bindings with correct memory management; hand-written JNI is the single most error-prone artifact we could add. spell-i's hand-written FFI made sense for Swift-only; it doesn't here. Buffer copies at the UniFFI boundary are irrelevant at our data rates (48 kHz mono f32 is ~192 KB/s worst case, pushed in chunks).

Everything else follows the research docs as written: XChaCha20-Poly1305 per-chunk AEAD with the 11-byte header as AAD and derived nonces (`chacha20poly1305` crate), Argon2id KDF (`argon2` crate), `reed-solomon-erasure` for multi-chunk text, no RS on voice (graceful per-chunk degradation), broadcast-first with no ARQ in v1.

## Repo Layout

```
aetr/
  Cargo.toml                  # workspace: members = ["core"]
  PLAN.md  docs/              # existing
  README.md                   # written in M5
  core/                       # aetr-core crate (staticlib + cdylib + rlib)
    Cargo.toml
    build.rs                  # cc compiles cpp/shim.cc + vendored aicodix
    cpp/
      aicodix/                # vendored modem/code/dsp headers, VENDORED.md pins commit
      shim.hh  shim.cc        # extern "C" modem_encode/modem_decode_feed etc.
    src/
      lib.rs
      api.rs                  # UniFFI surface (uniffi proc-macros)
      crypto.rs               # Argon2id KDF, per-chunk AEAD
      frame.rs                # header, chunking, nonce derivation, reassembly
      fec.rs                  # RS erasure encode/recover, interleaving
      voice.rs                # codec2 spans, 48k<->8k resampling, silence fill
      modem.rs                # Modem trait + safe wrapper over shim
      channel.rs              # (cfg(test)) simulated channel: AWGN, drops, skew
    uniffi-bindgen.rs         # bin for binding generation
  scripts/
    gen-bindings.sh           # uniffi-bindgen -> macos/Generated + android kotlin dir
  macos/
    project.yml  build.sh  build-rust.sh
    Aetr/                     # SwiftUI sources + BridgingHeader/modulemap
    Generated/                # UniFFI Swift output (gitignored ok)
  android/
    settings.gradle.kts  gradle/wrapper/  gradlew
    local.properties          # sdk.dir (gitignored)
    app/
      build.gradle.kts        # + cargo-ndk invocation task
      src/main/{AndroidManifest.xml, java/me/faulk/aetr/, jniLibs/}
  testdata/
    vectors.json              # passphrase->key, message->chunk bytes, header cases
    *.wav                     # golden modem bursts for cross-platform decode checks
```

## Protocol (recap, normative for implementation)

- Sample rate everywhere at the FFI boundary: 48 kHz mono f32. The shim instantiates aicodix at 48000; voice is decimated to 8 kHz inside `voice.rs` for codec2 and upsampled back on play. Platform layers never resample.
- Chunk = one modem transmission. Payload modes 85/128/170 bytes; default 170.
- Header (11 B, AEAD associated data): `message_id(8, random) | chunk_index(1) | chunk_count(1) | flags(1)` with flags bit0 is_repair, bit1 payload_type (0=text, 1=voice), bit2 last_chunk. Ciphertext + 16 B tag follow. Usable payload in 170-mode: 143 B.
- Nonce = `message_id || chunk_index || zero-pad` to 24 B. Never transmitted.
- KDF: Argon2id, m=19456 KiB, t=2, p=1 (OWASP baseline, sized for low-end Android), salt = fixed app constant `"aetr-v1"` since a shared passphrase is the only common secret. Documented tradeoff.
- Text > 143 B: split into K chunks, add R = ceil(0.25 K) RS repair chunks, interleave source/repair in TX order. Single-chunk text: no RS.
- Voice: codec2 1200 bps, each chunk a self-contained span (~0.95 s), no RS, missing spans play as silence placed by chunk_index.
- Voice clip cap: default 30 s, user-configurable in settings. The UI must show the
  consequence as the cap/clip grows: estimated airtime (~1.1 s per second of voice
  at 170-byte mode, longer in robust modes) and a warning that longer bursts have
  a higher chance of mid-transmission loss.
- ARQ (v1, user decision 2026-07-22): at encode time the sender generates a parity
  pool upfront (encode RS at N = 2K, transmit only the first R parity shards, cache
  the rest per message for the session). A receiver with an incomplete message can
  send a control frame: flags bit3 = control, payload = target message_id (8 B) +
  received-shard bitmask. Sender responds with enough cached parity shards (text)
  or exact missing chunks (voice, no RS) to close the gap. One round; still
  broadcast-safe since ARQ is optional and receiver-initiated.

## FFI Surface (UniFFI)

```rust
#[derive(uniffi::Enum)]  pub enum ModemMode { B85, B128, B170 }
#[derive(uniffi::Enum)]  pub enum RxState { Idle, Syncing, Receiving }
#[derive(uniffi::Record)] pub struct SessionConfig { passphrase: String, mode: ModemMode }

#[derive(uniffi::Enum)]
pub enum RxEvent {
  Text     { message_id: u64, text: String },
  Voice    { message_id: u64, pcm48k: Vec<f32>, missing_spans: Vec<u32> },
  Progress { message_id: u64, received: u32, total: u32, is_voice: bool },
  Failed   { message_id: u64, reason: String },   // auth fail / timeout
}

#[derive(uniffi::Object)] pub struct AetrSession;
impl AetrSession {
  #[uniffi::constructor] fn new(config: SessionConfig) -> Result<Arc<Self>, AetrError>; // runs KDF (blocking ~100ms)
  fn encode_text(&self, text: String) -> Result<Vec<f32>, AetrError>;   // full 48k PCM burst
  fn encode_voice(&self, pcm48k: Vec<f32>) -> Result<Vec<f32>, AetrError>;
  fn push_rx(&self, pcm48k: Vec<f32>);      // called from audio thread, any block size
  fn poll_events(&self) -> Vec<RxEvent>;    // UI polls ~10 Hz
  fn rx_state(&self) -> RxState;
  fn reset_rx(&self);
}
```

Session state behind a Mutex; `push_rx` is cheap (buffers + occasional frame decode). Poll model avoids UniFFI callback threading pain in v1.

## Milestones

### M0: Scaffold
Deliverables: workspace, `aetr-core` with vendored aicodix, `build.rs` compiling a trivial shim function on host, deps declared (`chacha20poly1305`, `argon2`, `reed-solomon-erasure`, `codec2`, `uniffi`, `cc`).
Verify: `cargo build -p aetr-core && cargo test -p aetr-core` (one FFI smoke test calling into the shim).

### M1: Core loopback over simulated noisy channel
Deliverables: `crypto.rs`, `frame.rs`, `fec.rs`, `voice.rs`, `modem.rs` + real shim (encode + streaming decode), `channel.rs` sim (AWGN at 5-20 dB SNR, whole-frame drops, +/-200 ppm sample-rate skew, level scaling, DC offset).
Tests (the acceptance bar for this milestone):
- text roundtrip single-chunk and 10-chunk, clean channel
- 10-chunk text with 2 dropped chunks recovered via RS
- voice clip with 1 dropped chunk yields silence gap at the right offset
- wrong passphrase: every chunk fails auth, no plaintext
- header/nonce/KDF known-answer tests written to `testdata/vectors.json`
Verify: `cargo test -p aetr-core --release` (DSP tests in release; debug run also passes but slow).

### M2: Bindings
Deliverables: `api.rs`, `scripts/gen-bindings.sh` producing Swift into `macos/Generated/` and Kotlin into `android/app/src/main/java/`.
Verify: script exits 0; `xcrun swiftc -parse macos/Generated/*.swift` clean; `cargo test` still green.

### M3: macOS app
Deliverables: `macos/` with XcodeGen `project.yml` + `build.sh` + `build-rust.sh` mirroring spell-i (universal static lib, `OTHER_LDFLAGS: [-laetr_core, -lc++]`, `NSMicrophoneUsageDescription`). SwiftUI: passphrase SecureField, send log + text field, hold-to-record voice with playback of received clips, input/output device pickers (CoreAudio device list, set via `kAudioOutputUnitProperty_CurrentDevice`), TX/RX state badge. AVAudioEngine taps at 48 kHz feeding `push_rx`; TX schedules the returned burst on the output. A debug "digital loopback" toggle pipes encode output straight to `push_rx`.
Verify: `cd macos && ./build.sh` produces `build/Debug/Aetr.app`; digital loopback delivers a message in-app; acoustic speaker-to-mic test on this machine.

### M4: Android app
One-time setup on this machine (SDK/NDK are installed at `~/Library/Android/sdk`, NDK 28.2.13676358, platforms 34-36; missing pieces below):
```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"  # no system JDK
export ANDROID_HOME="$HOME/Library/Android/sdk"
brew install gradle    # only to run `gradle wrapper` once; builds use ./gradlew after
```
Deliverables: Gradle project (Kotlin DSL, minSdk 29, target 35), Compose UI mirroring M3, `AudioRecord`/`AudioTrack` at 48 kHz mono float (plain APIs, not Oboe: no real-time duplex requirement, and Oboe adds NDK C++ glue for nothing), RECORD_AUDIO runtime permission, Gradle task running `cargo ndk -t arm64-v8a -t x86_64 -o app/src/main/jniLibs build --release`, UniFFI Kotlin bindings + JNA dependency.
Verify: `cd android && JAVA_HOME=... ./gradlew :app:assembleDebug` produces `app/build/outputs/apk/debug/app-debug.apk`. That is the "compiles for Android" bar on this machine. Optional device check: `~/Library/Android/sdk/platform-tools/adb install`.

### M5: Cross-platform validation + docs
Deliverables: golden `testdata/*.wav` bursts generated by a core test, decoded in a macOS unit test and an Android instrumented (or JVM+JNA desktop) test to prove byte-identical behavior; manual over-the-air checklist (two devices, speaker-to-mic, then real radios); `aetr/README.md` (setup, build, test for all three parts); add aetr to `ai-slop/CLAUDE.md` subproject list.
Verify: all of `cargo test --release`, `macos/build.sh`, `gradlew assembleDebug` green from a clean checkout following only the README.

## Environment Notes (checked on this machine, 2026-07-22)

Present: Xcode 26.6, xcodegen, cargo + apple targets, Android Studio, SDK + NDK 28.2 + build-tools 33-36.1.
Missing: system Java (use Android Studio's JBR), gradle CLI (bootstrap wrapper once), cargo-ndk, Android rust targets, `ANDROID_HOME`/`adb` on PATH. All covered by the M4 setup block; put it in the README verbatim.

## Risks and Open Questions

Risks:
- aicodix shim under NDK: mitigated by Rattlegram precedent and the `Modem` trait fallback seam.
- `codec2` crate fidelity vs C reference is less battle-tested; validate audibly in M3 and keep C-FFI as a v2 option.
- Real-radio performance (AGC, clipping, squelch tails) is unknowable until OTA testing; keep TX level configurable and test early in M5.
- LGPL (codec2) statically linked: fine for a personal tool, note relink obligation in README if ever distributed.
- Shared static passphrase means fixed KDF salt, no forward secrecy, and anyone with the phrase decrypts everything. Document the threat model; out of scope to fix in v1.

Open questions — resolved by user 2026-07-22:
1. Android minSdk 29: **yes**.
2. Application id `me.faulk.aetr`: **yes**.
3. Voice clip cap: **30 s default, configurable**, UI must show airtime impact when increased.
4. ARQ: **include in v1** (receiver-initiated single repair round; see Protocol section).
5. Modem mode picker: **yes**, 3-way picker in both UIs.
