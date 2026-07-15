//! C ABI for the AnyToneMac SwiftUI app (Phase 3).
//!
//! Conventions (mirrored by `core/include/anytone_core.h`):
//! - Strings cross the boundary as UTF-8 C strings. Every `char *` returned by
//!   this module is heap-allocated and must be released with
//!   [`anytone_string_free`]. Never free them any other way.
//! - Fallible functions either return a string (null on failure) or an `i32`
//!   status (`0` = success, `-1` = failure). On failure `err_out`, when
//!   non-null, receives a heap-allocated error message the caller must free
//!   with [`anytone_string_free`].
//! - No panic ever crosses the boundary: every entry point runs its body under
//!   `catch_unwind` and converts a panic into an error status.
//!
//! The device functions keep the same safety gates as the CLI: `restore`
//! requires an explicit `force` flag, checks the identify model string, and the
//! core verifies every written block by reading it back.

use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

use serde::Serialize;

use crate::codeplug::Codeplug;
use crate::device::{codeplug_size, is_supported_model, Radio};
use crate::ports::list_ports;
use crate::transport::SerialTransport;

/// Serial baud rate; largely ignored by the CDC-ACM driver but set anyway.
const BAUD: u32 = 115_200;

/// Per-read timeout for the serial link.
const TIMEOUT: Duration = Duration::from_secs(2);

/// C progress callback: `(blocks_done, blocks_total, user_context)`. Called
/// synchronously on the thread that invoked the FFI function.
pub type AnytoneProgress = extern "C" fn(done: usize, total: usize, user: *mut c_void);

/// One serial port, serialized to JSON for [`anytone_ports_json`].
#[derive(Serialize)]
struct PortJson {
    name: String,
    vid: Option<u16>,
    pid: Option<u16>,
    product: Option<String>,
    likely_radio: bool,
}

/// Convert a Rust string into a heap-allocated C string the caller frees with
/// [`anytone_string_free`]. Interior NULs (impossible in practice) are replaced
/// so this can never fail.
fn into_c_string(s: String) -> *mut c_char {
    let sanitized: String = s.chars().map(|c| if c == '\0' { ' ' } else { c }).collect();
    CString::new(sanitized)
        .expect("sanitized string has no interior NUL")
        .into_raw()
}

/// Store `msg` into `err_out` (when non-null) as a heap-allocated C string.
///
/// Safety: `err_out` must be null or a valid pointer to a `char *` slot.
unsafe fn set_err(err_out: *mut *mut c_char, msg: &str) {
    if !err_out.is_null() {
        *err_out = into_c_string(msg.to_string());
    }
}

/// Borrow a required UTF-8 C string argument, with a clear error on null or
/// invalid UTF-8.
///
/// Safety: `p` must be null or a valid NUL-terminated C string.
unsafe fn arg_str<'a>(p: *const c_char, name: &str) -> Result<&'a str, String> {
    if p.is_null() {
        return Err(format!("{name} must not be null"));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|_| format!("{name} is not valid UTF-8"))
}

/// Run a string-producing body under `catch_unwind`; on error set `err_out`
/// and return null.
///
/// Safety: `err_out` must be null or a valid pointer to a `char *` slot.
unsafe fn ffi_string(
    err_out: *mut *mut c_char,
    body: impl FnOnce() -> Result<String, String>,
) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(s)) => into_c_string(s),
        Ok(Err(e)) => {
            set_err(err_out, &e);
            std::ptr::null_mut()
        }
        Err(_) => {
            set_err(err_out, "internal error: panic in anytone-core");
            std::ptr::null_mut()
        }
    }
}

/// Run a status-producing body under `catch_unwind`; on error set `err_out`
/// and return -1, else 0.
///
/// Safety: `err_out` must be null or a valid pointer to a `char *` slot.
unsafe fn ffi_status(err_out: *mut *mut c_char, body: impl FnOnce() -> Result<(), String>) -> i32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => {
            set_err(err_out, &e);
            -1
        }
        Err(_) => {
            set_err(err_out, "internal error: panic in anytone-core");
            -1
        }
    }
}

/// Open the serial port and wrap it in a [`Radio`].
fn open_radio(port: &str) -> Result<Radio<SerialTransport>, String> {
    SerialTransport::open(port, BAUD, TIMEOUT)
        .map(Radio::new)
        .map_err(|e| format!("failed to open {port}: {e}"))
}

/// Build an `FnMut(done, total)` progress closure that forwards to the
/// optional C callback with its user context pointer.
fn progress_closure(
    progress: Option<AnytoneProgress>,
    user: *mut c_void,
) -> impl FnMut(usize, usize) {
    move |done, total| {
        if let Some(cb) = progress {
            cb(done, total, user);
        }
    }
}

/// Free a string previously returned by any function in this module. Passing
/// null is a no-op.
///
/// # Safety
/// `s` must be null or a pointer previously returned by this module and not
/// yet freed.
#[no_mangle]
pub unsafe extern "C" fn anytone_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Enumerate serial ports as a JSON array:
/// `[{"name","vid","pid","product","likely_radio"}, ...]`.
/// Returns null and sets `err_out` on failure.
///
/// # Safety
/// `err_out` must be null or a valid pointer to a `char *` slot.
#[no_mangle]
pub unsafe extern "C" fn anytone_ports_json(err_out: *mut *mut c_char) -> *mut c_char {
    ffi_string(err_out, || {
        let ports = list_ports().map_err(|e| e.to_string())?;
        let json: Vec<PortJson> = ports
            .into_iter()
            .map(|p| PortJson {
                name: p.name,
                vid: p.vid,
                pid: p.pid,
                product: p.product,
                likely_radio: p.likely_radio,
            })
            .collect();
        serde_json::to_string(&json).map_err(|e| format!("failed to encode ports JSON: {e}"))
    })
}

/// Enter program mode on `port`, identify the radio, exit, and return the
/// model/version string. Returns null and sets `err_out` on failure.
///
/// # Safety
/// `port` must be a valid C string; `err_out` null or a valid `char *` slot.
#[no_mangle]
pub unsafe extern "C" fn anytone_identify(
    port: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut c_char {
    ffi_string(err_out, || {
        let port = arg_str(port, "port")?;
        let mut radio = open_radio(port)?;
        radio.enter().map_err(|e| e.to_string())?;
        let model = radio.identify();
        // Always attempt a clean exit from program mode, even on failure.
        let exit = radio.exit();
        let model = model.map_err(|e| e.to_string())?;
        exit.map_err(|e| e.to_string())?;
        Ok(model)
    })
}

/// Read the full codeplug from the radio on `port` into the file `out_path`.
/// `progress` (nullable) is called after each block with `(done, total)`.
///
/// # Safety
/// `port`/`out_path` must be valid C strings; `err_out` null or a valid
/// `char *` slot; `user` is passed through to `progress` unchanged.
#[no_mangle]
pub unsafe extern "C" fn anytone_backup(
    port: *const c_char,
    out_path: *const c_char,
    progress: Option<AnytoneProgress>,
    user: *mut c_void,
    err_out: *mut *mut c_char,
) -> i32 {
    ffi_status(err_out, || {
        let port = arg_str(port, "port")?;
        let out_path = arg_str(out_path, "out_path")?;
        let mut radio = open_radio(port)?;
        radio.enter().map_err(|e| e.to_string())?;
        let mut cb = progress_closure(progress, user);
        let data = radio.read_codeplug(&mut cb);
        let exit = radio.exit();
        let data = data.map_err(|e| e.to_string())?;
        exit.map_err(|e| e.to_string())?;
        fs::write(out_path, &data).map_err(|e| format!("failed to write {out_path}: {e}"))?;
        Ok(())
    })
}

/// Write the codeplug file `in_path` back to the radio on `port`, with the
/// same safety gates as the CLI: `force` must be true, the file size must
/// match, the identify string must look like a D878UV, and every block is
/// read back and verified by the core.
///
/// # Safety
/// `port`/`in_path` must be valid C strings; `err_out` null or a valid
/// `char *` slot; `user` is passed through to `progress` unchanged.
#[no_mangle]
pub unsafe extern "C" fn anytone_restore(
    port: *const c_char,
    in_path: *const c_char,
    force: bool,
    progress: Option<AnytoneProgress>,
    user: *mut c_void,
    err_out: *mut *mut c_char,
) -> i32 {
    ffi_status(err_out, || {
        let port = arg_str(port, "port")?;
        let in_path = arg_str(in_path, "in_path")?;
        let data = fs::read(in_path).map_err(|e| format!("failed to read {in_path}: {e}"))?;
        let expected = codeplug_size();
        if data.len() != expected {
            return Err(format!(
                "codeplug file is {} bytes, expected {expected}",
                data.len()
            ));
        }
        if !force {
            return Err(
                "restore overwrites the radio's configuration; take a fresh backup \
                 and confirm (force) to proceed"
                    .to_string(),
            );
        }
        let mut radio = open_radio(port)?;
        radio.enter().map_err(|e| e.to_string())?;
        let model = radio.identify().map_err(|e| e.to_string())?;
        if !is_supported_model(&model) {
            let _ = radio.exit();
            return Err(format!(
                "refusing to write: identify string {model:?} is not a supported D878UV radio"
            ));
        }
        let mut cb = progress_closure(progress, user);
        let result = radio.write_codeplug(&data, &mut cb);
        // Always attempt a clean exit from program mode, even on failure.
        let exit = radio.exit();
        result.map_err(|e| e.to_string())?;
        exit.map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Parse the codeplug file `bin_path` offline and return its channels and
/// zones as JSON: `{"channels":[...],"zones":[...]}`. Returns null and sets
/// `err_out` on failure.
///
/// # Safety
/// `bin_path` must be a valid C string; `err_out` null or a valid `char *`
/// slot.
#[no_mangle]
pub unsafe extern "C" fn anytone_dump_json(
    bin_path: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut c_char {
    ffi_string(err_out, || {
        let bin_path = arg_str(bin_path, "bin_path")?;
        let data = fs::read(bin_path).map_err(|e| format!("failed to read {bin_path}: {e}"))?;
        let codeplug = Codeplug::parse(&data).map_err(|e| e.to_string())?;
        serde_json::to_string(&codeplug.to_json())
            .map_err(|e| format!("failed to encode codeplug JSON: {e}"))
    })
}

/// Apply a batch of channel/zone edits to the codeplug file `bin_in` and write
/// the result to `bin_out` (the two paths may be equal). `edits_json` is
/// `{"channels":[{"index",..optional "name","rx_frequency_hz",
/// "tx_frequency_hz"}],"zones":[{"index",optional "name"}]}`. Every edit
/// round-trips through the `Codeplug` model and is verified by re-parsing the
/// output, so only the intended bytes change.
///
/// # Safety
/// `bin_in`/`edits_json`/`bin_out` must be valid C strings; `err_out` null or
/// a valid `char *` slot.
#[no_mangle]
pub unsafe extern "C" fn anytone_apply_edits(
    bin_in: *const c_char,
    edits_json: *const c_char,
    bin_out: *const c_char,
    err_out: *mut *mut c_char,
) -> i32 {
    ffi_status(err_out, || {
        let bin_in = arg_str(bin_in, "bin_in")?;
        let edits_json = arg_str(edits_json, "edits_json")?;
        let bin_out = arg_str(bin_out, "bin_out")?;
        apply_edits_impl(bin_in, edits_json, bin_out)
    })
}

/// Load `bin_in`, apply the edits described by `edits_json` through the shared
/// [`crate::codeplug::apply_edits`], and write the verified result to `bin_out`.
fn apply_edits_impl(bin_in: &str, edits_json: &str, bin_out: &str) -> Result<(), String> {
    let data = fs::read(bin_in).map_err(|e| format!("failed to read {bin_in}: {e}"))?;
    let out = crate::codeplug::apply_edits(&data, edits_json)?;
    fs::write(bin_out, &out).map_err(|e| format!("failed to write {bin_out}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codeplug::{
        channel_addr, global_offset, set_bcd8_be, write_ascii, zone_channels_addr, zone_name_addr,
        CHANNEL_BITMAP, CONTACT_BITMAP, ZONE_BITMAP, ZONE_CHANNELS_SLOT,
    };
    use std::path::PathBuf;

    /// Build a minimal synthetic codeplug image: channel 0 (digital, 439 MHz
    /// simplex, CC1, TS2, "CALLING") and zone 0 ("Zone A", member [0]).
    fn synthetic_image() -> Vec<u8> {
        let mut raw = vec![0u8; codeplug_size()];

        // Inverted contact bitmap: all-set means no contacts active.
        let ctbm = global_offset(CONTACT_BITMAP).unwrap();
        for b in raw[ctbm..ctbm + 0x500].iter_mut() {
            *b = 0xff;
        }

        let cbm = global_offset(CHANNEL_BITMAP).unwrap();
        raw[cbm] |= 1;
        let rec = global_offset(channel_addr(0)).unwrap();
        set_bcd8_be(&mut raw[rec..rec + 4], 439_000_000 / 10);
        set_bcd8_be(&mut raw[rec + 4..rec + 8], 0);
        raw[rec + 8] = 0b0000_0001; // digital, simplex
        raw[rec + 0x20] = 1; // CC1
        raw[rec + 0x21] = 1; // TS2
        write_ascii(&mut raw[rec + 0x23..rec + 0x33], "CALLING", 0x00);

        let zbm = global_offset(ZONE_BITMAP).unwrap();
        raw[zbm] |= 1;
        let noff = global_offset(zone_name_addr(0)).unwrap();
        write_ascii(&mut raw[noff..noff + 16], "Zone A", 0x00);
        let coff = global_offset(zone_channels_addr(0)).unwrap();
        for b in raw[coff..coff + ZONE_CHANNELS_SLOT].iter_mut() {
            *b = 0xff;
        }
        raw[coff..coff + 2].copy_from_slice(&0u16.to_le_bytes());

        raw
    }

    /// Unique temp file path for this test process.
    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("anytone-ffi-{}-{name}", std::process::id()))
    }

    /// Convenience: build a CString from a str.
    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// Take ownership of an FFI string result, copy it, and free it.
    fn take(p: *mut c_char) -> String {
        assert!(!p.is_null());
        unsafe {
            let s = CStr::from_ptr(p).to_str().unwrap().to_string();
            anytone_string_free(p);
            s
        }
    }

    #[test]
    fn ports_json_returns_valid_json_array() {
        let mut err: *mut c_char = std::ptr::null_mut();
        let out = unsafe { anytone_ports_json(&mut err) };
        let json = take(out);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.is_array());
    }

    #[test]
    fn dump_json_reads_synthetic_codeplug() {
        let path = temp_path("dump.bin");
        fs::write(&path, synthetic_image()).unwrap();
        let cpath = c(path.to_str().unwrap());

        let mut err: *mut c_char = std::ptr::null_mut();
        let out = unsafe { anytone_dump_json(cpath.as_ptr(), &mut err) };
        let json = take(out);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["channels"][0]["name"], "CALLING");
        assert_eq!(v["channels"][0]["rx_frequency_hz"], 439_000_000);
        assert_eq!(v["channels"][0]["color_code"], 1);
        assert_eq!(v["channels"][0]["time_slot"], 2);
        assert_eq!(v["zones"][0]["name"], "Zone A");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn dump_json_missing_file_sets_error() {
        let cpath = c("/nonexistent/anytone-test.bin");
        let mut err: *mut c_char = std::ptr::null_mut();
        let out = unsafe { anytone_dump_json(cpath.as_ptr(), &mut err) };
        assert!(out.is_null());
        let msg = take(err);
        assert!(msg.contains("failed to read"));
    }

    #[test]
    fn apply_edits_roundtrips_name_rx_tx_and_zone() {
        let input = synthetic_image();
        let pin = temp_path("edit-in.bin");
        let pout = temp_path("edit-out.bin");
        fs::write(&pin, &input).unwrap();

        let edits = r#"{
            "channels": [{"index": 0, "name": "REPEATER",
                          "rx_frequency_hz": 446000000,
                          "tx_frequency_hz": 441000000}],
            "zones": [{"index": 0, "name": "Zone B"}]
        }"#;
        let (cin, cedits, cout) = (
            c(pin.to_str().unwrap()),
            c(edits),
            c(pout.to_str().unwrap()),
        );
        let mut err: *mut c_char = std::ptr::null_mut();
        let status =
            unsafe { anytone_apply_edits(cin.as_ptr(), cedits.as_ptr(), cout.as_ptr(), &mut err) };
        assert_eq!(status, 0, "apply_edits failed: {}", take(err));

        let out = fs::read(&pout).unwrap();
        let cp = Codeplug::parse(&out).unwrap();
        let ch = cp.channels().next().unwrap();
        assert_eq!(ch.name, "REPEATER");
        assert_eq!(ch.rx_frequency_hz, 446_000_000);
        assert_eq!(ch.tx_frequency_hz, 441_000_000); // negative 5 MHz offset
        assert_eq!(cp.zones().next().unwrap().name, "Zone B");

        // Only the channel-0 record and zone-0 name slot may differ.
        let rec = global_offset(channel_addr(0)).unwrap();
        let zn = global_offset(zone_name_addr(0)).unwrap();
        for (i, (a, b)) in input.iter().zip(out.iter()).enumerate() {
            if (rec..rec + 0x40).contains(&i) || (zn..zn + 0x20).contains(&i) {
                continue;
            }
            assert_eq!(a, b, "byte {i} changed unexpectedly");
        }

        fs::remove_file(&pin).ok();
        fs::remove_file(&pout).ok();
    }

    #[test]
    fn apply_edits_rejects_inactive_channel() {
        let pin = temp_path("inactive.bin");
        fs::write(&pin, synthetic_image()).unwrap();
        let (cin, cedits) = (
            c(pin.to_str().unwrap()),
            c(r#"{"channels":[{"index": 7, "name": "X"}]}"#),
        );
        let mut err: *mut c_char = std::ptr::null_mut();
        let status =
            unsafe { anytone_apply_edits(cin.as_ptr(), cedits.as_ptr(), cin.as_ptr(), &mut err) };
        assert_eq!(status, -1);
        assert!(take(err).contains("not active"));
        fs::remove_file(&pin).ok();
    }

    #[test]
    fn apply_edits_adds_channel_contact_group_and_radio_id() {
        let pin = temp_path("add-dmr-in.bin");
        let pout = temp_path("add-dmr-out.bin");
        fs::write(&pin, synthetic_image()).unwrap();

        // Add a digital channel wired to a new talk group + group list + radio
        // ID, all in one apply.
        let edits = r#"{
            "add_channels": [{"name": "TG9 TS1", "rx_frequency_hz": 439000000,
                              "mode": "digital", "color_code": 1, "time_slot": 1,
                              "contact_index": 0, "group_list_index": 0,
                              "radio_id_index": 0, "power": "high", "bandwidth": "narrow"}],
            "add_contacts": [{"name": "LOCAL 9", "number": 9, "call_type": "group"}],
            "add_group_lists": [{"name": "LOCAL", "members": [0]}],
            "add_radio_ids": [{"name": "HOME", "number": 3141592}]
        }"#;
        let (cin, ce, cout) = (c(pin.to_str().unwrap()), c(edits), c(pout.to_str().unwrap()));
        let mut err: *mut c_char = std::ptr::null_mut();
        let status =
            unsafe { anytone_apply_edits(cin.as_ptr(), ce.as_ptr(), cout.as_ptr(), &mut err) };
        assert_eq!(status, 0, "apply_edits failed: {}", take(err));

        // Re-dump and confirm every entity landed.
        let mut err2: *mut c_char = std::ptr::null_mut();
        let json = take(unsafe { anytone_dump_json(cout.as_ptr(), &mut err2) });
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["channels"].as_array().unwrap().len(), 2);
        assert_eq!(v["contacts"][0]["name"], "LOCAL 9");
        assert_eq!(v["contacts"][0]["number"], 9);
        assert_eq!(v["contacts"][0]["call_type"], "Group");
        assert_eq!(v["group_lists"][0]["name"], "LOCAL");
        assert_eq!(v["group_lists"][0]["members"][0], 0);
        assert_eq!(v["radio_ids"][0]["number"], 3141592);
        let added = v["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "TG9 TS1")
            .unwrap();
        assert_eq!(added["mode"], "Digital");
        assert_eq!(added["time_slot"], 1);

        fs::remove_file(&pin).ok();
        fs::remove_file(&pout).ok();
    }

    #[test]
    fn apply_edits_removes_contact_and_scrubs_group_list() {
        // Seed a codeplug with two contacts and a group list referencing both,
        // then remove contact 0 and confirm the group list drops it.
        let pin = temp_path("rm-contact.bin");
        fs::write(&pin, synthetic_image()).unwrap();
        let cin = c(pin.to_str().unwrap());

        let seed = c(r#"{
            "add_contacts": [{"name": "A", "number": 1, "call_type": "group"},
                             {"name": "B", "number": 2, "call_type": "group"}],
            "add_group_lists": [{"name": "GL", "members": [0, 1]}]
        }"#);
        let mut err: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe { anytone_apply_edits(cin.as_ptr(), seed.as_ptr(), cin.as_ptr(), &mut err) },
            0,
            "seed failed: {}",
            take(err)
        );

        let rm = c(r#"{"remove_contacts": [0]}"#);
        let mut err2: *mut c_char = std::ptr::null_mut();
        assert_eq!(
            unsafe { anytone_apply_edits(cin.as_ptr(), rm.as_ptr(), cin.as_ptr(), &mut err2) },
            0,
            "remove failed: {}",
            take(err2)
        );

        let mut err3: *mut c_char = std::ptr::null_mut();
        let json = take(unsafe { anytone_dump_json(cin.as_ptr(), &mut err3) });
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["contacts"].as_array().unwrap().len(), 1);
        assert_eq!(v["contacts"][0]["index"], 1);
        assert_eq!(v["group_lists"][0]["members"].as_array().unwrap(), &vec![serde_json::json!(1)]);

        fs::remove_file(&pin).ok();
    }

    #[test]
    fn apply_edits_rejects_bad_frequency_and_long_name() {
        let pin = temp_path("badfreq.bin");
        fs::write(&pin, synthetic_image()).unwrap();
        let cin = c(pin.to_str().unwrap());

        let cedits = c(r#"{"channels":[{"index": 0, "rx_frequency_hz": 146520005}]}"#);
        let mut err: *mut c_char = std::ptr::null_mut();
        let status =
            unsafe { anytone_apply_edits(cin.as_ptr(), cedits.as_ptr(), cin.as_ptr(), &mut err) };
        assert_eq!(status, -1);
        assert!(take(err).contains("multiple of 10"));

        let cedits = c(r#"{"channels":[{"index": 0, "name": "THIS NAME IS WAY TOO LONG"}]}"#);
        let mut err: *mut c_char = std::ptr::null_mut();
        let status =
            unsafe { anytone_apply_edits(cin.as_ptr(), cedits.as_ptr(), cin.as_ptr(), &mut err) };
        assert_eq!(status, -1);
        assert!(take(err).contains("longer than"));
        fs::remove_file(&pin).ok();
    }

    #[test]
    fn restore_requires_force_before_touching_the_port() {
        let p = temp_path("restore.bin");
        fs::write(&p, vec![0u8; codeplug_size()]).unwrap();
        let (cport, cpath) = (c("/dev/nonexistent-port"), c(p.to_str().unwrap()));
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe {
            anytone_restore(
                cport.as_ptr(),
                cpath.as_ptr(),
                false,
                None,
                std::ptr::null_mut(),
                &mut err,
            )
        };
        assert_eq!(status, -1);
        assert!(take(err).contains("backup"));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn identify_bad_port_sets_error() {
        let cport = c("/dev/nonexistent-anytone-port");
        let mut err: *mut c_char = std::ptr::null_mut();
        let out = unsafe { anytone_identify(cport.as_ptr(), &mut err) };
        assert!(out.is_null());
        assert!(take(err).contains("failed to open"));
    }

    #[test]
    fn null_arguments_are_rejected_not_crashed() {
        let mut err: *mut c_char = std::ptr::null_mut();
        let out = unsafe { anytone_identify(std::ptr::null(), &mut err) };
        assert!(out.is_null());
        assert!(take(err).contains("must not be null"));
    }

    #[test]
    fn string_free_of_null_is_noop() {
        unsafe { anytone_string_free(std::ptr::null_mut()) };
    }
}
