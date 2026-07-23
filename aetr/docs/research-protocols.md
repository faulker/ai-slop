# Step 1 Research: Data-over-Audio Protocols for Analog FM Radio

Use case: macOS + Android apps feed audio into analog FM transceivers on privately
licensed frequencies (~300-3000 Hz voice passband, push-to-talk). Payloads are
encrypted text and compressed voice clips. Core implemented in Rust.

## Protocol/modem survey

| Protocol | Modulation | Throughput (3 kHz FM channel) | Robust to FM artifacts? | License | Rust? |
|---|---|---|---|---|---|
| **aicodix modem / Rattlegram / Ribbit** | COFDM (256 carriers, QPSK, polar/BCH FEC) | ~1360-2800 bps raw (170 bytes/~1 s default; up to 8.6 kbps at QAM4096) | Yes, purpose-built for VHF/UHF FM push-to-talk; field-tested on GMRS/FRS | 0BSD (C++) | No crate; small portable C++ lib, easy to FFI-bind or port |
| **ggwave** | Multi-tone FSK + Reed-Solomon | 8-16 bytes/sec | Built for speaker-to-mic, not radio impairments | MIT | `ggwave-rs` bindings exist |
| **AX.25/AFSK1200 (Direwolf)** | AFSK 1200 baud | ~100-130 bytes/sec effective | Very proven over analog FM (APRS) | GPL | No |
| **PSK31/MFSK/JS8/FT8 family** | PSK/multi-tone FSK | Very low; built for -20 dB SNR HF weak-signal | Overkill FEC for a strong local FM link | varies | No |
| **FreeDV (1600/700D)** | OFDM/COHPSK | 700-1600 bps | Built for HF SSB, not narrowband FM | LGPL 2.1 (C) | No |
| **M17** | Direct RF 4FSK 9.6 kbps | 3200 bps voice | Disqualified: modulates RF deviation directly, needs digital-capable radio, not a 300-3000 Hz audio path | GPL | `m17rt` crates (wrong physical layer for us) |
| **quiet-modem/libquiet** | Configurable OFDM/FSK | ggwave-range | Less field-proven on FM radio | Apache-ish | No |

## Voice codecs

- **Codec2** (drowe67/codec2, C, LGPL 2.1): purpose-built for digital radio,
  700-3200 bps. The `codec2` crate on crates.io is a pure-Rust port covering
  3200/2400/1600/1400/1300/1200 bps but **not 700/700C**; for 700 bps use FFI
  to the C library (see `yuvadm/codec2.rs` or a minimal bindgen wrapper).
- **Opus**: great Rust support (`audiopus`/`opus`) but its quality floor is far
  above sub-1 kbps; Codec2 sounds much better at these rates. Not chosen.

## Closest existing solution

**Rattlegram/Ribbit** is the closest match: short text over analog FM handhelds
via mic/speaker coupling, exactly our transport. 2023 ARRL Technical Innovation
Award; confirmed working on GMRS/FRS in community reports.

- Reusable: aicodix `modem`/`code`/`dsp` (0BSD) — OFDM PHY with FEC, handles
  arbitrary binary datagrams, tuned for the mic → PTT → FM → speaker → squelch
  pipeline.
- Missing (we build): encryption layer, text-vs-voice payload framing, Rust
  bindings, voice clip support.
- Reticulum/LXMF: well-designed encrypted messaging stack but Python-first, no
  first-class Rust implementation; poor fit for our Rust shared-lib design.

## Throughput expectations

Default Rattlegram-like config (~1360-2800 bps in ~1.6 kHz), plus ~1-2 s
sync/settle overhead per transmission:

- Encrypted text (~100-200 bytes): **~2-3 s total per message**.
- 5 s voice clip @ Codec2 700 bps (~437 bytes): **~4-5 s total**.
- 5 s clip @ Codec2 1200 bps (~750 bytes): **~6 s total**.
- Higher-order modes (e.g. QAM256, ~5.4 kbps) on a strong local link cut the
  700 bps clip to **~1-2 s** — good adaptive option.

## Recommendation

1. **PHY**: build on **aicodix `modem`/`code`/`dsp`** (C++, 0BSD). FFI-bind via
   `cxx`/`extern "C"` shim into the Rust core, or port the compact DSP code to
   Rust. Configurable 0.7-8.6+ kbps.
2. **Voice codec**: **Codec2** via FFI to the canonical C library (LGPL 2.1) to
   get 700/700C modes.
3. **Encryption**: ChaCha20-Poly1305 with Argon2 passphrase-derived key, applied
   to compressed payload before modulation (modem treats payloads as opaque
   bytes).
4. Fallback/prototype option: ggwave (`ggwave-rs`), but 8-16 B/s only.

## Sources

- https://github.com/aicodix/rattlegram
- https://github.com/aicodix/modem
- https://github.com/aicodix
- https://alfaexploit.com/en/posts/ribbit_rattlegram/
- https://forums.mygmrs.com/topic/6178-ribbitrattlegram-on-gmrs/
- https://github.com/OpenResearchInstitute/ribbit
- https://www.gars.org/presentations/2023-03-14%20-%20Ribbit%20-%20A%20new%20digital%20texting%20mode%20for%20VHF%20&%20UHF%20-%20Pierre%20Deliou%20W4CKX.pdf
- https://github.com/ggerganov/ggwave
- https://crates.io/crates/ggwave-rs
- https://github.com/drowe67/codec2
- https://crates.io/crates/codec2
- https://www.rowetel.com/wordpress/?page_id=452
- https://freedv.org/
- https://github.com/drowe67/codec2/blob/main/README_freedv.md
- https://en.wikipedia.org/wiki/M17_(amateur_radio)
- https://m17-protocol-specification.readthedocs.io/en/latest/physical_layer.html
- https://github.com/thombles/m17rt/
- https://reticulum.network/manual/hardware.html
- https://github.com/wb2osz/direwolf
- https://github.com/quiet/quiet
- https://js8call.com/
