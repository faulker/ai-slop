//! High-level QYT KT-5000 image transfer: read the whole image, write the
//! writable part of it back.
//!
//! The KT-5000 memory map is flat, so unlike the AnyTone there is no region
//! table, no bitmap, and no sector read-modify-write. A read walks `0x0000` to
//! `0x31FF` in `0x40` steps; a write walks `0x0000` to `0x30FF` in `0x10`
//! steps. Both are preceded by the `0x3DF0` prologue probe.
//!
//! # The radio reboots after every transfer
//!
//! There is no exit command, and the radio resets itself once the port closes.
//! That means **no in-session read-back verification of a write is possible**,
//! and none is attempted below. The vendor CPS does not attempt one either.
//! Verifying a write means opening a fresh session after the radio comes back
//! and diffing the image. Do not add a read-back loop here; it will time out.

use crate::error::{Error, Result};
use crate::qyt::protocol::{
    end_read_session, read_block, session_prologue, write_block, READ_LEN, WRITE_LEN,
};
use crate::transport::Transport;

/// Size of the full image the radio exposes: `0x0000` through `0x31FF`.
pub const IMAGE_SIZE: usize = 0x3200;

/// First address never written. `0x3100`-`0x31FF` is read into the image so
/// backups stay faithful and diffable, but the vendor CPS never writes it and
/// neither do we. Corroborated twice over: CHIRP's `UPLOAD_MEM_SIZE` is
/// `0x3100`, and the capture's 784 writes at `0x10` each land exactly here.
pub const WRITE_LIMIT: u16 = 0x3100;

/// Number of read commands a full image read issues. One more than
/// `IMAGE_SIZE / READ_LEN` because the vendor CPS reads `0x0000` twice; see
/// [`read_image`].
pub const READ_BLOCKS: usize = IMAGE_SIZE / READ_LEN as usize + 1;

/// Number of write commands a full image write issues.
pub const WRITE_BLOCKS: usize = WRITE_LIMIT as usize / WRITE_LEN as usize;

/// Read the radio's full `0x3200`-byte image. `progress` is called after each
/// block with `(blocks_done, total_blocks)`.
///
/// The duplicated `0x0000` read is deliberate. The vendor CPS issues it, and
/// reproducing its host stream byte for byte is the cheapest correctness
/// argument available for hardware we cannot unit-test against. A second
/// 64-byte read costs nothing.
pub fn read_image(
    t: &mut dyn Transport,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<u8>> {
    session_prologue(t)?;

    let step = READ_LEN as usize;
    let mut image = vec![0u8; IMAGE_SIZE];
    let mut buf = [0u8; READ_LEN as usize];
    let mut done = 0usize;

    read_block(t, 0, READ_LEN, &mut buf)?;
    image[..step].copy_from_slice(&buf);
    done += 1;
    progress(done, READ_BLOCKS);

    for addr in (0..IMAGE_SIZE).step_by(step) {
        read_block(t, addr as u16, READ_LEN, &mut buf)?;
        image[addr..addr + step].copy_from_slice(&buf);
        done += 1;
        progress(done, READ_BLOCKS);
    }

    end_read_session(t)?;
    Ok(image)
}

/// Write `data` back to the radio, covering `0x0000` through `0x30FF` only.
/// `progress` is called after each acknowledged block.
///
/// `data` must be a full [`IMAGE_SIZE`] image even though the tail is not sent,
/// so that callers always hand over the same shape they got from
/// [`read_image`].
///
/// One deliberate divergence from the vendor CPS: at `0x2000`-`0x23EF` (950
/// bytes) the CPS uploads what is plainly an uninitialised buffer, into a
/// region the radio reads back as `0xFF` regardless. We send our own image's
/// bytes there instead, which for any image originating from [`read_image`]
/// means `0xFF`. This is the only range where our host stream differs from the
/// capture, and `core/tests/qyt_capture.rs` asserts the difference is confined
/// to exactly it.
pub fn write_image(
    t: &mut dyn Transport,
    data: &[u8],
    progress: &mut dyn FnMut(usize, usize),
) -> Result<()> {
    if data.len() != IMAGE_SIZE {
        return Err(Error::InvalidArgument(format!(
            "QYT image is {} bytes, expected {IMAGE_SIZE}",
            data.len()
        )));
    }

    session_prologue(t)?;

    let step = WRITE_LEN as usize;
    let mut done = 0usize;
    for addr in (0..WRITE_LIMIT as usize).step_by(step) {
        write_block(t, addr as u16, &data[addr..addr + step])?;
        done += 1;
        progress(done, WRITE_BLOCKS);
    }

    // No read-back verification here, by design. See the module docs.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qyt::mock::QytMock;
    use crate::qyt::protocol::{handshake, PROLOGUE_ADDR};

    /// Drive a full read against a mock seeded with a recognisable pattern.
    fn seeded_mock() -> QytMock {
        let mut m = QytMock::new();
        let pattern: Vec<u8> = (0..IMAGE_SIZE).map(|i| (i % 251) as u8).collect();
        m.seed(0, &pattern);
        m
    }

    #[test]
    fn block_counts_match_the_vendor_capture() {
        assert_eq!(READ_BLOCKS, 201);
        assert_eq!(WRITE_BLOCKS, 784);
    }

    #[test]
    fn read_image_issues_201_reads_plus_the_prologue_and_trailing_stx() {
        let mut m = seeded_mock();
        let image = read_image(&mut m, &mut |_, _| {}).unwrap();

        assert_eq!(image.len(), IMAGE_SIZE);
        assert_eq!(&image[..], &m.memory()[..IMAGE_SIZE]);

        // Prologue plus 201 data reads.
        assert_eq!(m.reads.len(), READ_BLOCKS + 1);
        assert_eq!(m.reads[0], (PROLOGUE_ADDR, 0x10));
        let data_reads: Vec<u16> = m.reads[1..].iter().map(|&(a, _)| a).collect();
        assert!(m.reads[1..].iter().all(|&(_, len)| len == READ_LEN));

        // The duplicated 0x0000, then a clean 0x40 stride to 0x31C0.
        assert_eq!(data_reads[0], 0x0000);
        assert_eq!(data_reads[1], 0x0000);
        let mut expected = vec![0x0000u16];
        expected.extend((0..IMAGE_SIZE).step_by(0x40).map(|a| a as u16));
        assert_eq!(data_reads, expected);
        assert_eq!(*data_reads.last().unwrap(), 0x31C0);

        // The session closes with one bare 0x06.
        assert_eq!(*m.written.last().unwrap(), 0x06);
    }

    #[test]
    fn read_image_reports_progress_for_every_block() {
        let mut m = seeded_mock();
        let mut seen = Vec::new();
        read_image(&mut m, &mut |done, total| seen.push((done, total))).unwrap();
        assert_eq!(seen.len(), READ_BLOCKS);
        assert_eq!(seen.first(), Some(&(1, READ_BLOCKS)));
        assert_eq!(seen.last(), Some(&(READ_BLOCKS, READ_BLOCKS)));
    }

    #[test]
    fn write_image_issues_784_writes_at_a_0x10_stride() {
        let mut m = QytMock::new();
        let data = vec![0x42u8; IMAGE_SIZE];
        write_image(&mut m, &data, &mut |_, _| {}).unwrap();

        assert_eq!(m.writes.len(), WRITE_BLOCKS);
        assert!(m.writes.iter().all(|&(_, len)| len == WRITE_LEN as usize));
        let addrs: Vec<u16> = m.writes.iter().map(|&(a, _)| a).collect();
        let expected: Vec<u16> = (0..WRITE_LIMIT).step_by(WRITE_LEN as usize).collect();
        assert_eq!(addrs, expected);
        assert_eq!(*addrs.last().unwrap(), 0x30F0);

        // The prologue is the only read a write session performs.
        assert_eq!(m.reads, vec![(PROLOGUE_ADDR, 0x10)]);
    }

    #[test]
    fn qyt_write_never_touches_0x3100_or_above() {
        let mut m = QytMock::new();
        // A sentinel above the write limit that a stray write would clobber.
        m.seed(0x3100, &[0xAAu8; 0x100]);
        let data = vec![0x00u8; IMAGE_SIZE];
        write_image(&mut m, &data, &mut |_, _| {}).unwrap();

        assert!(m.writes.iter().all(|&(addr, _)| addr < WRITE_LIMIT));
        assert_eq!(&m.memory()[0x3100..0x3200], &[0xAAu8; 0x100]);
        assert!(m.memory()[..0x3100].iter().all(|&b| b == 0x00));
    }

    #[test]
    fn write_image_rejects_a_wrongly_sized_image() {
        let mut m = QytMock::new();
        let err = write_image(&mut m, &[0u8; 16], &mut |_, _| {}).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "got {err:?}");
        assert!(m.written.is_empty());
    }

    #[test]
    fn qyt_write_then_read_roundtrips_an_image() {
        let mut m = QytMock::new();
        let mut data: Vec<u8> = (0..IMAGE_SIZE).map(|i| (i % 241) as u8).collect();
        // The tail is never written, so an honest round trip has to expect back
        // whatever the radio already holds there.
        for b in &mut data[WRITE_LIMIT as usize..] {
            *b = 0xFF;
        }

        write_image(&mut m, &data, &mut |_, _| {}).unwrap();
        let read_back = read_image(&mut m, &mut |_, _| {}).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn a_full_session_starts_with_the_handshake() {
        let mut m = seeded_mock();
        let id = handshake(&mut m).unwrap();
        assert!(id.is_kt5000());
        read_image(&mut m, &mut |_, _| {}).unwrap();
        assert_eq!(
            &m.written[..8],
            &[0x55, 0x20, 0x21, 0x01, 0x23, 0xFF, 0xDC, 0x02]
        );
        assert_eq!(&m.written[8..13], &[0x06, 0x53, 0x3D, 0xF0, 0x10]);
    }
}
