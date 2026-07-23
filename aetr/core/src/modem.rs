//! Safe wrapper over the C++ COFDM modem shim.
//!
//! The shim (core/cpp/shim.cc) implements the COFDMTV burst format at a
//! fixed 48 kHz mono f32 configuration; this module exposes it as the
//! `Modem` trait plus a streaming receiver, so a Rust-native fallback modem
//! could slot in without touching framing/crypto.

use crate::AetrError;
use std::os::raw::c_void;

/// Modem payload size selector. One frame = one burst = one transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ModemMode {
    /// 85-byte frames (most robust polar code).
    B85,
    /// 128-byte frames.
    B128,
    /// 170-byte frames (default).
    B170,
}

impl ModemMode {
    /// Index understood by the C shim (0/1/2).
    fn index(self) -> i32 {
        match self {
            ModemMode::B85 => 0,
            ModemMode::B128 => 1,
            ModemMode::B170 => 2,
        }
    }

    /// Total bytes carried by one modem frame in this mode.
    pub fn frame_bytes(self) -> usize {
        match self {
            ModemMode::B85 => 85,
            ModemMode::B128 => 128,
            ModemMode::B170 => 170,
        }
    }

    /// Bytes left for chunk plaintext after the 11-byte header and 16-byte tag.
    pub fn chunk_payload_bytes(self) -> usize {
        self.frame_bytes() - crate::frame::FRAME_OVERHEAD
    }

    /// Recovers a mode from a frame's byte length.
    pub fn from_frame_bytes(len: usize) -> Option<ModemMode> {
        match len {
            85 => Some(ModemMode::B85),
            128 => Some(ModemMode::B128),
            170 => Some(ModemMode::B170),
            _ => None,
        }
    }
}

extern "C" {
    fn aetr_modem_payload_bytes(mode: i32) -> i32;
    fn aetr_modem_burst_samples() -> i32;
    fn aetr_modem_encode(
        mode: i32,
        payload: *const u8,
        payload_len: i32,
        out_pcm: *mut f32,
        out_capacity: i32,
    ) -> i32;
    fn aetr_modem_rx_new() -> *mut c_void;
    fn aetr_modem_rx_feed(handle: *mut c_void, pcm: *const f32, len: i32) -> i32;
    fn aetr_modem_rx_fetch(handle: *mut c_void, out_payload: *mut u8) -> i32;
    fn aetr_modem_rx_free(handle: *mut c_void);
}

/// Feed status reported by the receiver, mirroring the shim's AETR_RX_* codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxStatus {
    /// Nothing new in this buffer.
    Idle,
    /// A preamble was decoded; payload symbols are being collected.
    Synced,
    /// A sync candidate appeared but preamble decoding failed.
    Failed,
}

/// Number of 48 kHz samples in one full burst (constant across modes).
pub fn burst_samples() -> usize {
    // Safety: pure constant getter with no state.
    (unsafe { aetr_modem_burst_samples() }) as usize
}

/// Transmit-side modem interface. Implementations turn one frame's bytes
/// into a complete 48 kHz mono f32 burst including preamble and sync.
pub trait Modem {
    /// Encodes exactly `mode.frame_bytes()` bytes into a PCM burst.
    fn encode_frame(&self, mode: ModemMode, frame: &[u8]) -> Result<Vec<f32>, AetrError>;
}

/// The aicodix COFDM modem (via the C++ shim).
pub struct OfdmModem;

impl Modem for OfdmModem {
    /// Encodes one frame into a burst. The frame must be exactly the mode's
    /// frame size; framing pads chunks up before calling this.
    fn encode_frame(&self, mode: ModemMode, frame: &[u8]) -> Result<Vec<f32>, AetrError> {
        if frame.len() != mode.frame_bytes() {
            return Err(AetrError::Malformed(format!(
                "frame is {} bytes, mode carries {}",
                frame.len(),
                mode.frame_bytes()
            )));
        }
        let cap = burst_samples();
        let mut pcm = vec![0f32; cap];
        // Safety: payload pointer/length describe a live slice, out buffer
        // has the capacity we pass, and the shim never retains pointers.
        let written = unsafe {
            aetr_modem_encode(
                mode.index(),
                frame.as_ptr(),
                frame.len() as i32,
                pcm.as_mut_ptr(),
                cap as i32,
            )
        };
        if written < 0 {
            return Err(AetrError::Modem(format!("encode failed with status {written}")));
        }
        pcm.truncate(written as usize);
        Ok(pcm)
    }
}

/// Streaming COFDM receiver. Feed arbitrary 48 kHz f32 buffers; completed
/// frames come back as byte vectors (85/128/170 bytes, mode auto-detected).
pub struct OfdmRx {
    handle: *mut c_void,
    last_status: RxStatus,
}

// Safety: the shim handle is a plain heap object touched only through the
// methods below; OfdmRx is not Sync, and moving it between threads is fine.
unsafe impl Send for OfdmRx {}

impl OfdmRx {
    /// Creates a streaming receiver.
    pub fn new() -> Result<Self, AetrError> {
        // Safety: constructor returns an owned handle or null.
        let handle = unsafe { aetr_modem_rx_new() };
        if handle.is_null() {
            return Err(AetrError::Modem("receiver allocation failed".into()));
        }
        Ok(OfdmRx { handle, last_status: RxStatus::Idle })
    }

    /// Feeds PCM and returns any frames completed inside this buffer. The
    /// buffer is internally processed in sub-block steps so multiple bursts
    /// per call are all recovered.
    pub fn feed(&mut self, pcm: &[f32]) -> Result<Vec<Vec<u8>>, AetrError> {
        let mut frames = Vec::new();
        // One decoder block is 8640 samples; stepping at that size means at
        // most one completed payload per step, so none can be missed.
        const STEP: usize = 8640;
        for chunk in pcm.chunks(STEP.max(1)) {
            // Safety: pointer/length describe the live chunk slice.
            let status = unsafe {
                aetr_modem_rx_feed(self.handle, chunk.as_ptr(), chunk.len() as i32)
            };
            match status {
                s if s < 0 => {
                    return Err(AetrError::Modem(format!("rx feed failed with status {s}")))
                }
                1 => self.last_status = RxStatus::Synced,
                2 => {
                    let mut payload = [0u8; 170];
                    // Safety: fetch writes at most 170 bytes.
                    let n = unsafe { aetr_modem_rx_fetch(self.handle, payload.as_mut_ptr()) };
                    if n > 0 {
                        frames.push(payload[..n as usize].to_vec());
                    }
                    self.last_status = RxStatus::Idle;
                }
                3 => self.last_status = RxStatus::Failed,
                _ => {}
            }
        }
        Ok(frames)
    }

    /// Most recent notable receiver status (for UI badges).
    pub fn status(&self) -> RxStatus {
        self.last_status
    }
}

impl Drop for OfdmRx {
    /// Releases the shim decoder.
    fn drop(&mut self) {
        // Safety: handle came from aetr_modem_rx_new and is dropped once.
        unsafe { aetr_modem_rx_free(self.handle) };
    }
}

/// Sanity check that the Rust and C sides agree on frame sizes.
pub fn shim_payload_bytes(mode: ModemMode) -> usize {
    // Safety: pure constant getter.
    (unsafe { aetr_modem_payload_bytes(mode.index()) }) as usize
}
