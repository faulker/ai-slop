---
status: done
task: "Phone/tablet↔phone/tablet shows Tx and Syncing/Rx but no messages; computer↔phone works"
updated: "2026-08-01 18:21:38 MDT"
---

# Fix mobile↔mobile decode with visible Rx

## Goal
Find why two Android devices sync (Rx badge moves) but never surface a message, while computer↔phone works, then fix the audio path and/or make silent decode failures visible.

## Approach
Treat this as an **Android analog/RX-path** problem first, not a core protocol bug. Core is bit-identical across platforms (golden wavs / UniFFI); computer↔phone already proves encode/decode can succeed. Auth failures and malformed frames are **silently dropped** in `AetrSession::push_rx`, so Syncing without a chat row is exactly what bad SNR / AEC / Bluetooth SCO / wrong passphrase look like.

Rejected: rewriting the modem. Sync already fires; the gap is post-sync frame survival and observability.

**Assumptions (stated defaults):** both ends are Android (phone/tablet); path may be acoustic speaker→mic and/or radios (possibly Bluetooth). Diagnostic steps below cover both before any code change.

## Steps
1. **Reproduce with a fixed checklist** (two Androids, short clear-text "hello", same mode, same passphrase or both clear/empty):
   - Note Rx badge: Idle → Syncing only, or also Progress / Failed rows.
   - Digital loopback on each device (encode → `pushRx` in-app): must succeed; proves session/UI.
   - Acoustic: built-in speaker → other built-in mic, ~30 cm, ~50% volume, mode **B85**, then B170.
   - If using radios: confirm TX delay / VOX primer; if Bluetooth, force **B85** and confirm SCO route status line is active on both.
2. **Config parity check** in UI on both devices: modem mode, passphrase (character-exact), encrypt vs clear lock, TX delay. Mode or key mismatch → Syncing possible, message never appears.
3. **If acoustic Android↔Android fails but Mac↔Android works:** treat phone TX speaker + Android capture processing as the culprit. In `android/.../AudioEngine.kt`, try a cleaner capture path for non-BT (prefer `MediaRecorder.AudioSource.UNPROCESSED` with fallback to `VOICE_RECOGNITION`; avoid anything that enables AEC/NS on built-in mic). Keep `VOICE_COMMUNICATION` only when `communicationRoute` is true.
4. **If Bluetooth radio on both Androids fails:** document/enforce B85 + adequate `tx_delay_ms`; consider UX that blocks denser modes or surfaces a stronger warning when both sides look like SCO. Optionally record a short RX PCM snip on Failed/timeout for offline analysis.
5. **Observability (small core + UI change):** stop being blind to silent drops. In `core/src/api.rs` `push_rx`, count or emit a lightweight debug/stat event (or promote selective `Failed` / status) when frames sync but `AuthFailed`/`Malformed` dominate; surface a short Android/macOS status like "synced, N frames rejected" so wrong passphrase vs bad audio is distinguishable. Keep radio-channel semantics (no spam of every bad frame).
6. **Verify** with the Test plan below; update `README.md` / `docs/OTA-TEST-CHECKLIST.md` with a "two Android devices" note (levels, B85 first, BT SCO caveats).

## Critical files
- `android/app/src/main/java/me/faulk/aetr/AudioEngine.kt` (AudioSource / attributes)
- `android/app/src/main/java/me/faulk/aetr/AudioRouter.kt` (SCO / MODE_IN_COMMUNICATION)
- `android/app/src/main/java/me/faulk/aetr/AetrViewModel.kt` (loopback, event → log)
- `android/app/src/main/java/me/faulk/aetr/ui/AetrScreen.kt` (Rx badge / status)
- `core/src/api.rs` (`push_rx` silent drop, `rx_state`)
- `docs/OTA-TEST-CHECKLIST.md`, `README.md`

## Test plan
- `cargo test -p aetr-core --release`
- Android JVM: `./gradlew :app:testDebugUnitTest` (loopback + golden wav still pass)
- Manual: each Android digital loopback → message appears
- Manual: Android A → Android B acoustic short clear text at B85 → message appears
- Manual: same at B170; note if only denser mode fails
- Manual: Mac ↔ Android still works (no regression)
- If radios/BT in use: B85 + TX delay checklist item from `docs/OTA-TEST-CHECKLIST.md`
- Confirm new reject/status indicator appears for deliberate wrong-passphrase while Syncing

## Risks / open questions
- Exact path unknown (acoustic vs FM vs Bluetooth SCO). Step 1 resolves this; code changes in steps 3–5 stay valid either way.
- `UNPROCESSED` is not available on every device; must fall back cleanly.
