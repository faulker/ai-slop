//! Golden-capture tests for the QYT KT-5000 wire protocol.
//!
//! These replay the raw vendor CPS captures in `docs/reference/QYT/` through
//! the real protocol code and assert that the byte stream we put on the wire is
//! identical to the one the Windows CPS puts on the wire. That is the strongest
//! correctness argument available for a radio we cannot attach to CI, and it is
//! stronger than any mock: the radio's half of the conversation is the real
//! recorded bytes, not something we invented.
//!
//! The capture directory is gitignored (the dumps carry personal data), so
//! every test here skips gracefully with a printed note when the files are
//! absent rather than failing.

use anytone_core::error::Result;
use anytone_core::qyt::codeplug::{QytCodeplug, Tone};
use anytone_core::qyt::device::{read_image, write_image, IMAGE_SIZE};
use anytone_core::qyt::protocol::handshake;
use anytone_core::transport::Transport;

/// Directory holding the eight raw capture files.
fn capture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/QYT")
}

/// Load one capture, or `None` when the gitignored directory is not present.
fn capture(name: &str) -> Option<Vec<u8>> {
    let path = capture_dir().join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            println!(
                "skipping: capture {} is not present (gitignored)",
                path.display()
            );
            None
        }
    }
}

/// A transport that replays a recorded radio-to-host stream and records the
/// host-to-radio stream we generate, so the two can be diffed against the
/// vendor's own capture pair.
struct CaptureTransport {
    /// The recorded `*_read` stream, i.e. everything the radio said.
    to_host: Vec<u8>,
    /// How much of `to_host` has been consumed.
    pos: usize,
    /// Everything the host said, for byte-exact comparison with `*_write`.
    written: Vec<u8>,
}

impl CaptureTransport {
    /// Wrap a recorded radio-to-host byte stream.
    fn new(to_host: Vec<u8>) -> Self {
        Self {
            to_host,
            pos: 0,
            written: Vec::new(),
        }
    }
}

impl Transport for CaptureTransport {
    /// Record what the host sends; the replay never reacts to it.
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.written.extend_from_slice(buf);
        Ok(())
    }

    /// Hand back the next slice of the recorded stream, or time out at its end.
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let end = self.pos + buf.len();
        if end > self.to_host.len() {
            return Err(anytone_core::error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "capture exhausted: wanted {} bytes at offset {}, only {} remain",
                    buf.len(),
                    self.pos,
                    self.to_host.len() - self.pos
                ),
            )));
        }
        buf.copy_from_slice(&self.to_host[self.pos..end]);
        self.pos = end;
        Ok(())
    }
}

/// Reconstruct the radio image from the recorded read session, the same way
/// `read_image` does but driven straight off the capture.
fn image_from_read_capture() -> Option<Vec<u8>> {
    let replies = capture("read-radio_read")?;
    let mut t = CaptureTransport::new(replies);
    handshake(&mut t).expect("handshake against the capture");
    Some(read_image(&mut t, &mut |_, _| {}).expect("read_image against the capture"))
}

#[test]
fn read_session_host_stream_is_byte_identical_to_the_vendor_capture() {
    let (Some(replies), Some(expected)) = (capture("read-radio_read"), capture("read-radio_write"))
    else {
        return;
    };

    let mut t = CaptureTransport::new(replies);
    handshake(&mut t).unwrap();
    let image = read_image(&mut t, &mut |_, _| {}).unwrap();

    assert_eq!(image.len(), IMAGE_SIZE);
    assert_eq!(
        expected.len(),
        1019,
        "capture read-radio_write changed size unexpectedly"
    );
    assert_eq!(
        t.written.len(),
        expected.len(),
        "host stream is {} bytes, vendor sent {}",
        t.written.len(),
        expected.len()
    );
    assert_eq!(
        t.written, expected,
        "host read stream diverges from the vendor CPS"
    );
}

#[test]
fn write_session_host_stream_matches_the_vendor_capture_outside_the_garbage_window() {
    let (Some(image), Some(replies), Some(expected)) = (
        image_from_read_capture(),
        capture("write-radio_read"),
        capture("write-radio_write"),
    ) else {
        return;
    };

    let mut t = CaptureTransport::new(replies);
    handshake(&mut t).unwrap();
    let mut acks = 0usize;
    write_image(&mut t, &image, &mut |_, _| acks += 1).unwrap();

    assert_eq!(acks, 784, "expected 784 acknowledged write blocks");
    assert_eq!(
        expected.len(),
        16477,
        "capture write-radio_write changed size unexpectedly"
    );
    assert_eq!(
        t.written.len(),
        expected.len(),
        "host stream is {} bytes, vendor sent {}",
        t.written.len(),
        expected.len()
    );

    // Map every differing stream byte back to the image address it carries.
    // Stream layout after the 8-byte handshake and the 5-byte prologue is a run
    // of 21-byte frames: 5 header bytes then 16 data bytes.
    const PREFIX: usize = 8 + 5;
    const FRAME: usize = 5 + 16;
    let mut differing_addrs = Vec::new();
    for (i, (&ours, &theirs)) in t.written.iter().zip(expected.iter()).enumerate() {
        if ours == theirs {
            continue;
        }
        assert!(i >= PREFIX, "handshake or prologue differs at offset {i}");
        let frame = (i - PREFIX) / FRAME;
        let within = (i - PREFIX) % FRAME;
        assert!(
            within >= 5,
            "a frame header differs at offset {i} (frame {frame}, byte {within})"
        );
        differing_addrs.push(frame * 16 + (within - 5));
    }

    // The only place we deliberately diverge: the vendor CPS uploads 950 bytes
    // of uninitialised buffer at 0x2000-0x23EF into a region the radio reads
    // back as 0xFF. We send our image's own bytes there instead.
    assert!(
        !differing_addrs.is_empty(),
        "expected to differ inside the 0x2000-0x23EF garbage window"
    );
    let lo = *differing_addrs.iter().min().unwrap();
    let hi = *differing_addrs.iter().max().unwrap();
    assert!(
        lo >= 0x2000 && hi <= 0x23EF,
        "differences escape the garbage window: 0x{lo:04X}..=0x{hi:04X}"
    );
    // Everything we send in that window is the erased 0xFF the radio reports.
    assert!(
        differing_addrs.iter().all(|&a| image[a] == 0xFF),
        "the garbage window should differ only where our image holds 0xFF"
    );
}

#[test]
fn read_capture_parses_to_an_image_with_the_expected_landmarks() {
    let Some(image) = image_from_read_capture() else {
        return;
    };

    assert_eq!(image.len(), 12_800);

    // Channel 0: 462.5625 MHz simplex, no tones, flag byte 0x24.
    assert_eq!(
        &image[0x0000..0x0010],
        &[
            0x50, 0x62, 0x25, 0x46, 0x50, 0x62, 0x25, 0x46, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x24
        ]
    );

    // The model tag the file-format detector will key off.
    assert_eq!(&image[0x0FE0..0x0FF0], b"Model:KT-5000   ");

    // Unused channel-name space and the read-only tail are erased.
    assert!(image[0x0A60..0x0E00].iter().all(|&b| b == 0xFF));
    assert!(image[0x3100..0x3200].iter().all(|&b| b == 0xFF));
}

#[test]
fn write_capture_replies_are_784_acks() {
    let Some(replies) = capture("write-radio_read") else {
        return;
    };

    // Identity (61) + prologue reply (20), then one bare 0x06 per write block,
    // then the 117-byte port-close banner the radio emits as it reboots.
    const HEADER: usize = 61 + 20;
    const TRAILER: usize = 117;
    assert_eq!(replies.len(), 982);
    let acks = &replies[HEADER..replies.len() - TRAILER];
    assert_eq!(acks.len(), 784);
    assert!(acks.iter().all(|&b| b == 0x06));
}

#[test]
fn the_real_capture_parses_into_the_expected_codeplug() {
    let Some(image) = image_from_read_capture() else {
        return;
    };
    let cp = QytCodeplug::parse(&image).expect("the captured image parses");

    // 159 programmed slots out of 200, the highest at index 165.
    assert_eq!(cp.channels().count(), 159);
    assert_eq!(cp.channels().map(|c| c.index).max(), Some(165));
    assert_eq!(cp.model_tag(), "Model:KT-5000");

    let ch0 = cp.channels().find(|c| c.index == 0).unwrap();
    assert_eq!(ch0.name, "FRS-01");
    assert_eq!(ch0.rx_frequency_hz, 462_562_500);
    assert_eq!(ch0.tx_frequency_hz, Some(462_562_500));
    assert_eq!(ch0.rx_tone, Tone::None);
    // The two bits CHIRP leaves unnamed, set on this channel alone.
    assert_eq!(ch0.flags_raw, [0, 0, 0, 0x24]);

    let ch64 = cp.channels().find(|c| c.index == 64).unwrap();
    assert_eq!(ch64.name, "N7FYZ");
    assert_eq!(ch64.rx_frequency_hz, 147_320_000);
    assert_eq!(ch64.tx_frequency_hz, Some(147_920_000));
    assert_eq!(ch64.rx_tone, Tone::None);
    assert_eq!(ch64.tx_tone, Tone::Ctcss { hz_tenths: 1000 });

    // Every tone word in the capture decodes to something real, never to the
    // "unmappable" fallback on a non-zero word.
    for ch in cp.channels() {
        for tone in [ch.rx_tone, ch.tx_tone] {
            if let Tone::Ctcss { hz_tenths } = tone {
                assert!((670..=2503).contains(&hz_tenths), "channel {}", ch.index);
            }
        }
    }
}

#[test]
fn the_real_capture_round_trips_byte_for_byte() {
    let Some(image) = image_from_read_capture() else {
        return;
    };
    let cp = QytCodeplug::parse(&image).expect("the captured image parses");
    assert_eq!(cp.serialize(), image);
}

#[test]
fn an_edit_to_the_real_capture_disturbs_nothing_else() {
    let Some(image) = image_from_read_capture() else {
        return;
    };
    let mut cp = QytCodeplug::parse(&image).expect("the captured image parses");
    cp.channel_mut(64).unwrap().set_name("EDITED");
    let out = cp.serialize();

    let touched: Vec<usize> = (0..image.len()).filter(|&i| image[i] != out[i]).collect();
    assert!(touched.iter().all(|&i| (0x1000 + 64 * 16..0x1000 + 64 * 16 + 7).contains(&i)));
    // Channel 1's unmodelled 0x05 at name byte 7 is still there.
    assert_eq!(out[0x1000 + 16 + 7], 0x05);
    // So is channel 0's unnamed tail.
    assert_eq!(out[15], 0x24);
}
