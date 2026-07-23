# Step 2 Research: Link/Application-Layer Error Checking & Loss Recovery

Builds on `research-protocols.md` (aicodix OFDM modem + Codec2 + passphrase-derived
symmetric encryption). Question: above the modem's PHY FEC, how do we frame,
verify, and recover messages so decryption survives lost frames, squelch drops,
and disconnection?

## Channel model: erasure, not bit errors

The aicodix modem uses CRC32C-aided SCL polar decoding — a delivered frame has
already passed CRC as part of PHY decode. Frames either decode cleanly or not at
all. Narrowband modes carry ~85/128/170 bytes per ~1 s transmission. There is no
multi-frame reassembly layer in the modem; that's the app's job. So: no
redundant per-chunk CRC needed; design for **whole-chunk erasure**.

## 1. Framing: per-chunk AEAD

Per-chunk AEAD with the chunk header as associated data; nonce **derived** from
`message_id || chunk_index`, never transmitted.

- Each chunk is independently verifiable/decryptable on arrival — required when
  any subset of frames may be missing and more may never come.
- Whole-message-then-chunk MAC would block using any data until all chunks
  arrive; conflicts with partial-loss/partial-play requirement.
- Precedent: LXMF/Reticulum encrypts per-packet for small messages; M17 packet
  mode's single CRC-16 over a chunked superframe is a poor fit here.
- Cost: 16-byte Poly1305 tag + header per chunk (~27 B fixed, 16-32% overhead
  depending on modem mode). Accepted as the price of independence; minimized by
  deriving nonces instead of sending them.

## 2. Erasure coding across chunks

Default: **`reed-solomon-erasure`** (MIT, mature). MDS: any K of K+R shards
recovers deterministically — better than fountain codes' probabilistic decode
for small K (2-50). `raptorq` (Apache-2.0) is a good alternative where rateless
repair generation helps (multi-round ARQ). `reed-solomon-simd` only matters at
hundreds of shards.

**Asymmetry**: erasure-code text (atomic recovery is what you want), do NOT
erasure-code voice by default — voice degrades gracefully per-chunk, and RS is
all-or-nothing below the K threshold.

## 3. ARQ over PTT half-duplex

Per-frame stop-and-wait would ~double airtime (frames are ~1 s). Instead,
message-level selective repeat (VARA/ARDOP principle: retransmit only what's
missing, short ACK frames):

1. Sender transmits full burst (K source + R repair chunks), no waiting.
2. If a return channel exists, receiver sends ONE short status frame:
   `message_id + received_count` (or bitmask).
3. MDS means the sender only needs *how many* are missing — it sends that many
   fresh repair shards.
4. ARQ is strictly optional; baseline must work with zero return channel
   (one-to-many broadcast).

## 4. Integrity, replay, nonces

- Per-chunk Poly1305 tag covers authenticity (and neutralizes rare polar-decode
  false-accepts). No separate whole-message hash needed.
- Truncation detection: header carries `chunk_count` / `last_chunk` flag.
- **XChaCha20-Poly1305** (192-bit nonce) so uncoordinated senders sharing a
  static passphrase key need no persisted counter state. Nonce = `message_id
  (8B) || chunk_index (1B) || zero-pad (15B)`. 8-byte random message_id gives a
  ~2^32-message birthday bound — ample for hand-PTT use.

## 5. Recommended design

**Header (11 bytes, AEAD associated data):**

```
message_id   : 8 bytes   random, unique per message
chunk_index  : 1 byte    0..254
chunk_count  : 1 byte    total incl. repair; 0 = unknown/streaming
flags        : 1 byte    bit0 = is_repair_chunk
                         bit1 = payload_type (0=text, 1=voice)
                         bit2 = last_chunk
                         bits3-7 reserved
```

Followed by ciphertext + 16-byte tag. Fixed overhead 27 B/chunk; prefer the
larger modem payload mode (170 B) where the link allows.

- **Chunk size**: one chunk = one modem transmission. No sub-chunking.
- **Text >1 chunk**: RS erasure coding, R ≈ 20-30% of K. Single-chunk text: none.
- **Voice clips**: split Codec2 stream into self-contained spans, encrypt each
  independently, play what arrived with silence/comfort-noise for missing spans
  (placed via chunk_index). Optional small R per-message if completeness is
  preferred for short clips.
- **Broadcast**: choose R generously upfront (no second chance); for voice this
  further favors graceful degradation over RS thresholds.
- **Interleaving**: when RS is used, round-robin source and repair chunks in
  transmission order so a cut-short reception window can still reach K of N.

## Sources

- https://www.aicodix.de/cofdmtv/rattlegram/
- https://github.com/aicodix/modem
- https://github.com/markqvist/LXMF and https://unsigned.io/lxmf/
- https://m17-protocol-specification.readthedocs.io/en/latest/data_link_layer.html
- https://en.wikipedia.org/wiki/FX.25_Forward_Error_Correction
- https://sbcara.org/wp-content/uploads/2025/08/VARA-Specification.pdf
- https://winlink.org/content/ardop_overview
- https://github.com/cberner/raptorq
- https://www.cberner.com/2020/10/12/building-fastest-raptorq-rfc6330-codec-rust/
- https://github.com/rust-rse/reed-solomon-erasure
- https://github.com/AndersTrier/reed-solomon-simd
- https://doc.libsodium.org/secret-key_cryptography/aead/chacha20-poly1305/xchacha20-poly1305_construction
- https://soatok.blog/2021/03/12/understanding-extended-nonce-constructions/
