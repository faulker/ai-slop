use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::obd::pid;
use crate::protocol::uds;
use crate::toyota::{did_scan, ecu_scan, enhanced_pids, write_safety};
use crate::transport::elm327::Elm327;

/// Progress update sent during long-running scan/backup operations.
#[derive(Debug, Clone)]
pub enum ProgressUpdate {
    /// (current, total, description)
    Step(usize, usize, String),
    Done(String),
}

/// Commands sent to the Elm327 actor.
#[allow(dead_code)]
pub enum ElmCommand {
    Initialize {
        reply: oneshot::Sender<Result<String>>,
    },
    ReadPid {
        name: String,
        reply: oneshot::Sender<Result<(String, f64, String)>>,
    },
    FetchEnhancedDid {
        did_hex: String,
        ecu: String,
        reply: oneshot::Sender<Result<EnhancedDidResult>>,
    },
    FetchDtcs {
        reply: oneshot::Sender<Result<Vec<DtcEntry>>>,
    },
    ClearDtcs {
        reply: oneshot::Sender<Result<()>>,
    },
    ScanEcus {
        progress: mpsc::UnboundedSender<ProgressUpdate>,
        reply: oneshot::Sender<Result<Vec<FoundEcu>>>,
    },
    ScanDidRange {
        ecu: String,
        start: u16,
        end: u16,
        test_writable: bool,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
        reply: oneshot::Sender<Result<Vec<did_scan::DiscoveredDid>>>,
    },
    ScanKwp {
        ecu: String,
        service: u8,
        start: u8,
        end: u8,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
        reply: oneshot::Sender<Result<Vec<KwpResult>>>,
    },
    VerifiedWrite {
        did_hex: String,
        data_hex: String,
        ecu: String,
        protocol: String,
        dry_run: bool,
        reply: oneshot::Sender<Result<()>>,
    },
    RestoreDid {
        did_hex: String,
        ecu: String,
        reply: oneshot::Sender<Result<()>>,
    },
    BackupAll {
        progress: mpsc::UnboundedSender<ProgressUpdate>,
        reply: oneshot::Sender<Result<()>>,
    },
    RawCommand {
        cmd: String,
        reply: oneshot::Sender<Result<String>>,
    },
    AtCommand {
        cmd: String,
        reply: oneshot::Sender<Result<String>>,
    },
    SetSession {
        session: u8,
        reply: oneshot::Sender<Result<Vec<u8>>>,
    },
    TesterPresent {
        reply: oneshot::Sender<Result<Vec<u8>>>,
    },
    ReadDidValue {
        did: u16,
        ecu: String,
        protocol: String, // "uds" or "kwp"
        reply: oneshot::Sender<Result<Vec<u8>>>,
    },
    SecurityAccess {
        ecu: String,
        level: u8,
        reply: oneshot::Sender<Result<Vec<u8>>>,
    },
    /// Full security unlock: get seed, compute key, send key — all in one shot.
    SecurityUnlock {
        ecu: String,
        level: u8,
        reply: oneshot::Sender<Result<String>>,
    },
    /// Send a raw multi-frame ISO-TP message (for data > 7 bytes).
    SendMultiFrame {
        data_hex: String,
        reply: oneshot::Sender<Result<String>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Simplified DTC entry for TUI display.
#[derive(Debug, Clone)]
pub struct DtcEntry {
    pub code: String,
}

/// Simplified enhanced DID result for TUI display.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EnhancedDidResult {
    pub did: u16,
    pub name: String,
    pub value: Option<f64>,
    pub unit: String,
    pub raw_hex: String,
}

/// Found ECU info (owned, for sending across channels).
#[derive(Debug, Clone)]
pub struct FoundEcu {
    pub name: String,
    pub tx_address: String,
}

/// KWP2000 scan result — a local identifier that returned data.
#[derive(Debug, Clone)]
pub struct KwpResult {
    pub _service: u8,
    pub local_id: u8,
    pub data: Vec<u8>,
}

/// Clone-able handle for sending commands to the Elm327 actor.
#[derive(Clone)]
pub struct ElmHandle {
    tx: mpsc::Sender<ElmCommand>,
}

#[allow(dead_code)]
impl ElmHandle {
    pub async fn initialize(&self) -> Result<String> {
        let (reply, rx) = oneshot::channel::<Result<String>>();
        let _ = self.tx.send(ElmCommand::Initialize { reply }).await;
        rx.await.unwrap_or(Err(crate::error::Error::NotConnected))
    }

    pub async fn read_pid(&self, name: &str) -> Result<(String, f64, String)> {
        let (reply, rx) = oneshot::channel::<Result<(String, f64, String)>>();
        let _ = self
            .tx
            .send(ElmCommand::ReadPid {
                name: name.to_string(),
                reply,
            })
            .await;
        rx.await.unwrap_or(Err(crate::error::Error::NotConnected))
    }

    pub fn try_read_pid(&self, name: &str) -> Option<oneshot::Receiver<Result<(String, f64, String)>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::ReadPid {
                name: name.to_string(),
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_fetch_enhanced_did(
        &self,
        did_hex: &str,
        ecu: &str,
    ) -> Option<oneshot::Receiver<Result<EnhancedDidResult>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::FetchEnhancedDid {
                did_hex: did_hex.to_string(),
                ecu: ecu.to_string(),
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_fetch_dtcs(&self) -> Option<oneshot::Receiver<Result<Vec<DtcEntry>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx.try_send(ElmCommand::FetchDtcs { reply }).ok()?;
        Some(rx)
    }

    pub fn try_clear_dtcs(&self) -> Option<oneshot::Receiver<Result<()>>> {
        let (reply, rx) = oneshot::channel();
        self.tx.try_send(ElmCommand::ClearDtcs { reply }).ok()?;
        Some(rx)
    }

    pub fn try_scan_ecus(
        &self,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
    ) -> Option<oneshot::Receiver<Result<Vec<FoundEcu>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx.try_send(ElmCommand::ScanEcus { progress, reply }).ok()?;
        Some(rx)
    }

    pub fn try_scan_kwp(
        &self,
        ecu: &str,
        service: u8,
        start: u8,
        end: u8,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
    ) -> Option<oneshot::Receiver<Result<Vec<KwpResult>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::ScanKwp {
                ecu: ecu.to_string(),
                service,
                start,
                end,
                progress,
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_scan_did_range(
        &self,
        ecu: &str,
        start: u16,
        end: u16,
        test_writable: bool,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
    ) -> Option<oneshot::Receiver<Result<Vec<did_scan::DiscoveredDid>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::ScanDidRange {
                ecu: ecu.to_string(),
                start,
                end,
                test_writable,
                progress,
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_verified_write(
        &self,
        did_hex: &str,
        data_hex: &str,
        ecu: &str,
        protocol: &str,
        dry_run: bool,
    ) -> Option<oneshot::Receiver<Result<()>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::VerifiedWrite {
                did_hex: did_hex.to_string(),
                data_hex: data_hex.to_string(),
                ecu: ecu.to_string(),
                protocol: protocol.to_string(),
                dry_run,
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_restore_did(
        &self,
        did_hex: &str,
        ecu: &str,
    ) -> Option<oneshot::Receiver<Result<()>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::RestoreDid {
                did_hex: did_hex.to_string(),
                ecu: ecu.to_string(),
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_backup_all(
        &self,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
    ) -> Option<oneshot::Receiver<Result<()>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::BackupAll { progress, reply })
            .ok()?;
        Some(rx)
    }

    pub fn try_raw_command(&self, cmd: &str) -> Option<oneshot::Receiver<Result<String>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::RawCommand {
                cmd: cmd.to_string(),
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_at_command(&self, cmd: &str) -> Option<oneshot::Receiver<Result<String>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::AtCommand {
                cmd: cmd.to_string(),
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_set_session(&self, session: u8) -> Option<oneshot::Receiver<Result<Vec<u8>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::SetSession { session, reply })
            .ok()?;
        Some(rx)
    }

    pub fn try_tester_present(&self) -> Option<oneshot::Receiver<Result<Vec<u8>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::TesterPresent { reply })
            .ok()?;
        Some(rx)
    }

    pub fn try_security_access(&self, ecu: &str, level: u8) -> Option<oneshot::Receiver<Result<Vec<u8>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::SecurityAccess {
                ecu: ecu.to_string(),
                level,
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_security_unlock(&self, ecu: &str, level: u8) -> Option<oneshot::Receiver<Result<String>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::SecurityUnlock {
                ecu: ecu.to_string(),
                level,
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub fn try_send_multi_frame(&self, data_hex: &str) -> Option<oneshot::Receiver<Result<String>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::SendMultiFrame {
                data_hex: data_hex.to_string(),
                reply,
            })
            .ok()?;
        Some(rx)
    }

    pub async fn shutdown(&self) {
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(ElmCommand::Shutdown { reply }).await;
        let _ = rx.await;
    }

    pub fn try_read_did_value(&self, did: u16, ecu: &str, protocol: &str) -> Option<oneshot::Receiver<Result<Vec<u8>>>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(ElmCommand::ReadDidValue { did, ecu: ecu.to_string(), protocol: protocol.to_string(), reply })
            .ok()?;
        Some(rx)
    }
}

/// Spawn the Elm327 actor. Returns a handle for sending commands and the task join handle.
pub fn spawn(elm: Elm327) -> (ElmHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(32);
    let handle = ElmHandle { tx };
    let join = tokio::spawn(actor_loop(elm, rx));
    (handle, join)
}

async fn actor_loop(elm: Elm327, mut rx: mpsc::Receiver<ElmCommand>) {
    // Wrap in ManuallyDrop so the serial port is never explicitly closed.
    // On macOS, closing a Bluetooth RFCOMM serial port causes the OS to
    // terminate the BT connection, requiring a full unpair/repair cycle.
    // By leaking the handle, the OS cleans up the fd on process exit
    // without triggering the BT disconnect.
    let mut elm = std::mem::ManuallyDrop::new(elm);
    while let Some(cmd) = rx.recv().await {
        match cmd {
            ElmCommand::Initialize { reply } => {
                let _ = reply.send(elm.initialize().await);
            }
            ElmCommand::ReadPid { name, reply } => {
                let _ = reply.send(pid::fetch_pid(&mut elm, &name).await);
            }
            ElmCommand::FetchEnhancedDid {
                did_hex,
                ecu,
                reply,
            } => {
                let result = fetch_enhanced_did_impl(&mut elm, &did_hex, &ecu).await;
                let _ = reply.send(result);
            }
            ElmCommand::FetchDtcs { reply } => {
                let result = fetch_dtcs_impl(&mut elm).await;
                let _ = reply.send(result);
            }
            ElmCommand::ClearDtcs { reply } => {
                let result = clear_dtcs_impl(&mut elm).await;
                let _ = reply.send(result);
            }
            ElmCommand::ScanEcus { progress, reply } => {
                let result = scan_ecus_impl(&mut elm, &progress).await;
                let _ = progress.send(ProgressUpdate::Done(
                    match &result {
                        Ok(ecus) => format!("ECU scan complete: {} found", ecus.len()),
                        Err(e) => format!("ECU scan failed: {}", e),
                    }
                ));
                let _ = reply.send(result);
            }
            ElmCommand::ScanDidRange {
                ecu,
                start,
                end,
                test_writable,
                progress,
                reply,
            } => {
                let result = scan_did_range_impl(
                    &mut elm, &ecu, start, end, test_writable, &progress,
                ).await;
                let _ = progress.send(ProgressUpdate::Done(
                    match &result {
                        Ok(dids) => format!("Scan complete: {} DID(s) found", dids.len()),
                        Err(e) => format!("Scan failed: {}", e),
                    }
                ));
                let _ = reply.send(result);
            }
            ElmCommand::ScanKwp {
                ecu,
                service,
                start,
                end,
                progress,
                reply,
            } => {
                let result = scan_kwp_impl(
                    &mut elm, &ecu, service, start, end, &progress,
                ).await;
                let _ = progress.send(ProgressUpdate::Done(
                    match &result {
                        Ok(r) => format!("KWP scan complete: {} found", r.len()),
                        Err(e) => format!("KWP scan failed: {}", e),
                    }
                ));
                let _ = reply.send(result);
            }
            ElmCommand::VerifiedWrite {
                did_hex,
                data_hex,
                ecu,
                protocol,
                dry_run,
                reply,
            } => {
                let result = if protocol == "kwp" {
                    kwp_write_impl(&mut elm, &did_hex, &data_hex, &ecu, dry_run).await
                } else {
                    write_safety::verified_write_did(&mut elm, &did_hex, &data_hex, &ecu, dry_run)
                        .await
                };
                let _ = reply.send(result);
            }
            ElmCommand::RestoreDid {
                did_hex,
                ecu,
                reply,
            } => {
                let result = write_safety::restore_did(&mut elm, &did_hex, &ecu).await;
                let _ = reply.send(result);
            }
            ElmCommand::BackupAll { progress, reply } => {
                let result = backup_all_impl(&mut elm, &progress).await;
                let _ = progress.send(ProgressUpdate::Done(
                    match &result {
                        Ok(count) => format!("Backup complete: {} DID(s) saved", count),
                        Err(e) => format!("Backup failed: {}", e),
                    }
                ));
                let _ = reply.send(result.map(|_| ()));
            }
            ElmCommand::RawCommand { cmd, reply } => {
                let result = elm.send_passthrough(&cmd).await;
                let _ = reply.send(result);
            }
            ElmCommand::AtCommand { cmd, reply } => {
                let result = elm.at_command(&cmd).await;
                let _ = reply.send(result);
            }
            ElmCommand::SetSession { session, reply } => {
                let req = uds::diagnostic_session_control(session);
                let result = uds::send_uds(&mut elm, &req).await;
                let _ = reply.send(result);
            }
            ElmCommand::TesterPresent { reply } => {
                let req = uds::tester_present();
                let result = uds::send_uds(&mut elm, &req).await;
                let _ = reply.send(result);
            }
            ElmCommand::ReadDidValue { did, ecu, protocol, reply } => {
                let result = async {
                    elm.set_header(&ecu).await?;

                    if protocol == "kwp" {
                        // KWP2000: service 0x21 (ReadDataByLocalIdentifier)
                        let cmd = format!("21 {:02X}", did as u8);
                        let raw = elm.send_passthrough(&cmd).await?;
                        let lines: Vec<String> = raw.split(|c| c == '\r' || c == '\n')
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .filter(|l| l.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
                            .collect();
                        if lines.is_empty() {
                            elm.set_header("7DF").await?;
                            return Err(crate::error::Error::Protocol("no response".into()));
                        }
                        let payload = crate::protocol::isotp::reassemble(&lines)?;
                        // KWP positive response: 0x61 [localId] [data...]
                        if payload.len() >= 2 && payload[0] == 0x61 {
                            let data = payload[2..].to_vec();
                            elm.set_header("7DF").await?;
                            Ok(data)
                        } else if payload.len() >= 3 && payload[0] == 0x7F {
                            elm.set_header("7DF").await?;
                            Err(crate::error::Error::Protocol(
                                format!("KWP negative response: NRC 0x{:02X}", payload[2])
                            ))
                        } else {
                            elm.set_header("7DF").await?;
                            Err(crate::error::Error::Protocol("unexpected KWP response".into()))
                        }
                    } else {
                        // UDS: service 0x22 (ReadDataByIdentifier)
                        match read_did_value_impl(&mut elm, did).await {
                            Ok(val) => {
                                elm.set_header("7DF").await?;
                                Ok(val)
                            }
                            Err(_first_err) => {
                                let session_req = uds::diagnostic_session_control(uds::SESSION_EXTENDED);
                                if uds::send_uds(&mut elm, &session_req).await.is_ok() {
                                    let val = read_did_value_impl(&mut elm, did).await;
                                    let _ = uds::send_uds(
                                        &mut elm,
                                        &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
                                    ).await;
                                    elm.set_header("7DF").await?;
                                    val
                                } else {
                                    elm.set_header("7DF").await?;
                                    Err(_first_err)
                                }
                            }
                        }
                    }
                }.await;
                let _ = reply.send(result);
            }
            ElmCommand::SecurityAccess { ecu, level, reply } => {
                let result = async {
                    elm.set_header(&ecu).await?;

                    // Use send_passthrough instead of send_uds — the ELM327
                    // clone's multi-frame responses don't survive parse_response
                    // filtering, but the raw text is correct.
                    let cmd = format!("{:02X} {:02X}", uds::SECURITY_ACCESS, level);
                    let raw = elm.send_passthrough(&cmd).await?;

                    // Parse the raw response manually
                    let lines: Vec<&str> = raw.split(|c| c == '\r' || c == '\n')
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .filter(|l| l.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
                        .collect();

                    if lines.is_empty() {
                        elm.set_header("7DF").await?;
                        return Err(crate::error::Error::Protocol(
                            format!("no response (raw: {:?})", raw.chars().take(80).collect::<String>())
                        ));
                    }

                    // Reassemble ISO-TP manually from raw lines
                    let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
                    let payload = crate::protocol::isotp::reassemble(&owned)?;

                    elm.set_header("7DF").await?;
                    Ok(payload)
                }.await;
                let _ = reply.send(result);
            }
            ElmCommand::SecurityUnlock { ecu, level, reply } => {
                let result = security_unlock_impl(&mut elm, &ecu, level).await;
                let _ = reply.send(result);
            }
            ElmCommand::SendMultiFrame { data_hex, reply } => {
                let result = send_multi_frame_impl(&mut elm, &data_hex).await;
                let _ = reply.send(result);
            }
            ElmCommand::Shutdown { reply } => {
                // Don't send any AT commands on shutdown — both ATZ and ATPC
                // cause the ELM327 clone's Bluetooth module to disconnect,
                // requiring a full unpair/repair cycle on macOS.
                // Just drop the serial connection cleanly; the ELM327 will
                // idle-timeout back to its default state on its own.
                let _ = reply.send(());
                break;
            }
        }
    }
}

async fn fetch_enhanced_did_impl(
    elm: &mut Elm327,
    did_hex: &str,
    ecu: &str,
) -> Result<EnhancedDidResult> {
    let did = u16::from_str_radix(did_hex.trim_start_matches("0x"), 16)
        .map_err(|_| crate::error::Error::Config(format!("invalid DID hex: {}", did_hex)))?;

    elm.set_header(ecu).await?;
    // Explicitly set receive address (TX + 8)
    let tx_val = u16::from_str_radix(ecu, 16).unwrap_or(0);
    let rx_addr = format!("{:03X}", tx_val + 8);
    let _ = elm.at_command(&format!("ATCRA {}", rx_addr)).await;

    let request = uds::read_data_by_identifier(did);
    let response = uds::send_uds(elm, &request).await?;

    if response.len() < 3 {
        return Err(crate::error::Error::Protocol("response too short".into()));
    }

    let data = &response[3..];
    let raw_hex = uds::hex_string(data);

    let dids = enhanced_pids::cached_dids();
    let (name, value, unit) = if let Some(def) = dids.iter().find(|d| d.id == did) {
        let v = evaluate_simple_formula(&def.formula, data);
        (def.name.clone(), v, def.unit.clone())
    } else {
        (format!("DID 0x{:04X}", did), None, "raw".to_string())
    };

    let _ = elm.at_command("ATAR").await;
    elm.set_header("7DF").await?;

    Ok(EnhancedDidResult {
        did,
        name,
        value,
        unit,
        raw_hex,
    })
}

fn evaluate_simple_formula(formula: &str, data: &[u8]) -> Option<f64> {
    let a = data.first().copied().unwrap_or(0) as f64;
    let b = data.get(1).copied().unwrap_or(0) as f64;
    let normalized: String = formula.split_whitespace().collect::<Vec<_>>().join(" ");
    match normalized.as_str() {
        "A" => Some(a),
        "B" => Some(b),
        "A - 40" => Some(a - 40.0),
        "B - 40" => Some(b - 40.0),
        "A * 100 / 255" | "A * 100.0 / 255.0" => Some(a * 100.0 / 255.0),
        "(A * 256 + B) / 4" => Some((a * 256.0 + b) / 4.0),
        "(A * 256 + B) / 100" => Some((a * 256.0 + b) / 100.0),
        "(A * 256 + B) / 1000" => Some((a * 256.0 + b) / 1000.0),
        "(A * 256 + B)" => Some(a * 256.0 + b),
        "A / 2 - 64" => Some(a / 2.0 - 64.0),
        "A * 3 / 255" => Some(a * 3.0 / 255.0),
        "(A - 128) * 100 / 128" => Some((a - 128.0) * 100.0 / 128.0),
        _ => None,
    }
}

async fn fetch_dtcs_impl(elm: &mut Elm327) -> Result<Vec<DtcEntry>> {
    use crate::protocol::isotp;

    let response = elm.send_obd("03").await?;
    let payload = isotp::reassemble(&response.lines)?;

    if payload.is_empty() || payload.len() < 2 || payload[0] != 0x43 {
        return Ok(Vec::new());
    }

    let num_dtcs = payload[1] as usize;
    let dtc_bytes = &payload[2..];
    let mut dtcs = Vec::new();

    for i in 0..num_dtcs {
        let offset = i * 2;
        if offset + 1 >= dtc_bytes.len() {
            break;
        }
        let b1 = dtc_bytes[offset];
        let b2 = dtc_bytes[offset + 1];
        let prefix = match (b1 >> 6) & 0x03 {
            0 => 'P',
            1 => 'C',
            2 => 'B',
            3 => 'U',
            _ => '?',
        };
        let code = format!(
            "{}{}{:X}{:X}{:X}",
            prefix,
            (b1 >> 4) & 0x03,
            b1 & 0x0F,
            (b2 >> 4) & 0x0F,
            b2 & 0x0F
        );
        dtcs.push(DtcEntry { code });
    }
    Ok(dtcs)
}

async fn clear_dtcs_impl(elm: &mut Elm327) -> Result<()> {
    use crate::protocol::isotp;

    let response = elm.send_obd("04").await?;
    let payload = isotp::reassemble(&response.lines)?;
    if payload.is_empty() || payload[0] != 0x44 {
        return Err(crate::error::Error::Protocol(
            "unexpected clear DTC response".into(),
        ));
    }
    Ok(())
}

async fn scan_ecus_impl(
    elm: &mut Elm327,
    progress: &mpsc::UnboundedSender<ProgressUpdate>,
) -> Result<Vec<FoundEcu>> {
    let total = ecu_scan::KNOWN_ECUS.len();

    // Set a moderate timeout for scanning (~300ms per probe)
    elm.at_command("ATST 4B").await?;

    // Warmup: send a broadcast TesterPresent to 7DF to ensure the CAN bus
    // and protocol are fully initialized before probing individual addresses.
    elm.set_header("7DF").await?;
    let _ = elm.send_obd("3E 00").await;

    let mut found = Vec::new();

    for (i, ecu) in ecu_scan::KNOWN_ECUS.iter().enumerate() {
        let _ = progress.send(ProgressUpdate::Step(
            i + 1,
            total,
            format!("0x{} {} — {} found", ecu.tx_address, ecu.name, found.len()),
        ));

        elm.set_header(ecu.tx_address).await?;

        // Explicitly set the expected receive address (TX + 8) so we don't
        // rely on ELM327 auto-detection which can get confused.
        let tx_val = u16::from_str_radix(ecu.tx_address, 16).unwrap_or(0);
        let rx_addr = format!("{:03X}", tx_val + 8);
        let _ = elm.at_command(&format!("ATCRA {}", rx_addr)).await;

        // Use send_obd directly instead of send_uds — send_uds treats
        // negative responses (NRC) as errors, but a negative response still
        // means the ECU is present and responding.
        match elm.send_obd("3E 00").await {
            Ok(resp) if !resp.lines.is_empty() => {
                found.push(FoundEcu {
                    name: ecu.name.to_string(),
                    tx_address: ecu.tx_address.to_string(),
                });
            }
            _ => {
                // NO DATA or error — ECU not present
            }
        }
    }

    elm.at_command("ATST FF").await?;
    // Reset to auto receive address handling
    elm.at_command("ATAR").await?;
    elm.set_header("7DF").await?;
    Ok(found)
}

/// TUI-friendly DID range scan: no stdout, per-DID progress, cancellable via channel close.
async fn scan_did_range_impl(
    elm: &mut Elm327,
    ecu: &str,
    start: u16,
    end: u16,
    test_writable: bool,
    progress: &mpsc::UnboundedSender<ProgressUpdate>,
) -> Result<Vec<did_scan::DiscoveredDid>> {
    let total = (end as u32 - start as u32 + 1) as usize;
    let mut found: Vec<did_scan::DiscoveredDid> = Vec::new();

    elm.set_header(ecu).await?;

    // Explicitly set the expected receive address (TX + 8)
    let tx_val = u16::from_str_radix(ecu, 16).unwrap_or(0);
    let rx_addr = format!("{:03X}", tx_val + 8);
    elm.at_command(&format!("ATCRA {}", rx_addr)).await?;

    // Use a moderate timeout — ATST 32 is ~200ms, balances speed vs reliability
    elm.at_command("ATST 32").await?;

    // Enter extended session (some DIDs require it)
    let session_req = uds::diagnostic_session_control(uds::SESSION_EXTENDED);
    let _ = uds::send_uds(elm, &session_req).await;

    let mut keepalive_counter = 0u32;

    for did in start..=end {
        let current = (did as u32 - start as u32 + 1) as usize;

        // Check if cancelled (progress receiver dropped)
        if progress.send(ProgressUpdate::Step(
            current,
            total,
            format!("0x{:04X} — {} found", did, found.len()),
        )).is_err() {
            // Receiver dropped = user cancelled
            break;
        }

        // TesterPresent keepalive every 50 DIDs
        keepalive_counter += 1;
        if keepalive_counter >= 50 {
            keepalive_counter = 0;
            let tp = uds::tester_present();
            let _ = uds::send_uds(elm, &tp).await;
        }

        let request = uds::read_data_by_identifier(did);
        match uds::send_uds(elm, &request).await {
            Ok(response) => {
                if response.len() >= 3 && response[0] == 0x62 {
                    let data = response[3..].to_vec();

                    let writable = if test_writable && !data.is_empty() {
                        // Write current value back to test writability
                        let write_req = uds::write_data_by_identifier(did, &data);
                        match uds::send_uds(elm, &write_req).await {
                            Ok(resp) => resp.first().copied() == Some(0x6E),
                            Err(_) => false,
                        }
                    } else {
                        false
                    };

                    found.push(did_scan::DiscoveredDid { did, data, writable });
                }
            }
            Err(_) => {
                // Expected for non-existent DIDs — skip silently
            }
        }
    }

    // Return to default session, restore timeout, filter, and header
    let _ = uds::send_uds(
        elm,
        &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
    ).await;
    elm.at_command("ATST FF").await?;
    elm.at_command("ATAR").await?;
    elm.set_header("7DF").await?;

    Ok(found)
}

/// TUI-friendly backup: reads all writable DIDs, grouped by ECU, with progress.
/// Supports both UDS (service 0x22) and KWP (service 0x21).
async fn backup_all_impl(
    elm: &mut Elm327,
    progress: &mpsc::UnboundedSender<ProgressUpdate>,
) -> Result<usize> {
    use crate::toyota::backup::BackupStore;

    let dids = enhanced_pids::cached_dids();
    let writable: Vec<&enhanced_pids::DidDefinition> = dids.iter()
        .filter(|d| d.writable)
        .collect();

    if writable.is_empty() {
        return Ok(0);
    }

    let mut store = BackupStore::load()?;
    let total = writable.len();
    let mut success = 0usize;
    let mut current = 0usize;

    // Group by ECU
    let mut by_ecu: std::collections::BTreeMap<&str, Vec<&&enhanced_pids::DidDefinition>> =
        std::collections::BTreeMap::new();
    for d in &writable {
        by_ecu.entry(&d.ecu).or_default().push(d);
    }

    for (ecu, ecu_dids) in &by_ecu {
        elm.set_header(ecu).await?;

        let _ = progress.send(ProgressUpdate::Step(
            current, total,
            format!("ECU 0x{} — {} DIDs", ecu, ecu_dids.len()),
        ));

        for did_def in ecu_dids {
            current += 1;
            if progress.send(ProgressUpdate::Step(
                current, total,
                format!("0x{:02X} {}", did_def.id, did_def.name),
            )).is_err() {
                // Cancelled
                break;
            }

            let read_result = if did_def.protocol == "kwp" {
                // KWP read: service 0x21
                let cmd = format!("21 {:02X}", did_def.id as u8);
                match elm.send_passthrough(&cmd).await {
                    Ok(raw) => {
                        let lines: Vec<String> = raw.split(|c| c == '\r' || c == '\n')
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .filter(|l| l.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
                            .collect();
                        match crate::protocol::isotp::reassemble(&lines) {
                            Ok(payload) if payload.len() >= 2 && payload[0] == 0x61 => {
                                Ok(payload[2..].to_vec())
                            }
                            _ => Err(crate::error::Error::Elm("no response".into())),
                        }
                    }
                    Err(e) => Err(e),
                }
            } else {
                // UDS read: service 0x22
                let request = uds::read_data_by_identifier(did_def.id);
                match uds::send_uds(elm, &request).await {
                    Ok(resp) if resp.len() >= 3 => Ok(resp[3..].to_vec()),
                    Ok(_) => Err(crate::error::Error::Protocol("response too short".into())),
                    Err(e) => Err(e),
                }
            };

            match read_result {
                Ok(data) => {
                    let _ = store.record(ecu, did_def.id, &data);
                    success += 1;
                }
                Err(_) => {
                    // Skip DIDs that don't respond
                }
            }
        }

        elm.set_header("7DF").await?;
    }

    Ok(success)
}

/// KWP2000 write via service 0x3B (WriteDataByLocalIdentifier).
/// Handles NRC 0x78 (responsePending) by waiting and re-reading.
async fn kwp_write_impl(
    elm: &mut Elm327,
    did_hex: &str,
    data_hex: &str,
    ecu: &str,
    dry_run: bool,
) -> Result<()> {
    let local_id = u8::from_str_radix(did_hex.trim_start_matches("0x"), 16)
        .map_err(|_| crate::error::Error::Config(format!("invalid local ID: {}", did_hex)))?;
    let data_bytes: Vec<u8> = data_hex
        .split_whitespace()
        .map(|s| u8::from_str_radix(s, 16).map_err(|_| crate::error::Error::Config(format!("invalid hex: {}", s))))
        .collect::<Result<Vec<_>>>()?;

    elm.set_header(ecu).await?;

    // Read current value first
    let read_cmd = format!("21 {:02X}", local_id);
    let raw = elm.send_passthrough(&read_cmd).await?;
    let lines: Vec<String> = raw.split(|c| c == '\r' || c == '\n')
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .filter(|l| l.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
        .collect();
    let payload = crate::protocol::isotp::reassemble(&lines)?;
    if payload.len() < 2 || payload[0] != 0x61 {
        elm.set_header("7DF").await?;
        return Err(crate::error::Error::Protocol("failed to read current value".into()));
    }

    if dry_run {
        elm.set_header("7DF").await?;
        return Ok(());
    }

    // Write: service 0x3B [localId] [data...]
    let write_cmd = format!("3B {:02X} {}",
        local_id,
        data_bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
    );
    let write_raw = elm.send_passthrough(&write_cmd).await?;

    // Parse response — handle NRC 0x78 (responsePending)
    let mut response_text = write_raw;
    for _ in 0..10 {
        if response_text.contains("7B") {
            // 0x7B = positive response (0x3B + 0x40)
            break;
        }
        if response_text.contains("78") && response_text.contains("7F") {
            // NRC 0x78 = responsePending — wait and read more
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            match elm.send_passthrough("").await {
                Ok(more) => response_text = more,
                Err(_) => break,
            }
        } else {
            break;
        }
    }

    elm.set_header("7DF").await?;

    if response_text.contains("7B") {
        Ok(())
    } else if response_text.contains("7F") {
        Err(crate::error::Error::Protocol(
            format!("KWP write rejected: {}", response_text.trim())
        ))
    } else {
        Err(crate::error::Error::Protocol(
            format!("unexpected write response: {}", response_text.trim())
        ))
    }
}

/// KWP2000 scan: sweep a single-byte local identifier range for a given service.
/// Works with service 0x21 (ReadDataByLocalIdentifier) and 0x1A (ReadEcuIdentification).
async fn scan_kwp_impl(
    elm: &mut Elm327,
    ecu: &str,
    service: u8,
    start: u8,
    end: u8,
    progress: &mpsc::UnboundedSender<ProgressUpdate>,
) -> Result<Vec<KwpResult>> {
    let total = (end as u16 - start as u16 + 1) as usize;
    let mut found: Vec<KwpResult> = Vec::new();
    let positive_response = service + 0x40;

    elm.set_header(ecu).await?;
    let tx_val = u16::from_str_radix(ecu, 16).unwrap_or(0);
    let rx_addr = format!("{:03X}", tx_val + 8);
    elm.at_command(&format!("ATCRA {}", rx_addr)).await?;
    elm.at_command("ATST 32").await?;

    for id in start..=end {
        let current = (id as u16 - start as u16 + 1) as usize;

        if progress.send(ProgressUpdate::Step(
            current,
            total,
            format!("svc 0x{:02X} id 0x{:02X} — {} found", service, id, found.len()),
        )).is_err() {
            break;
        }

        let cmd = format!("{:02X} {:02X}", service, id);
        match elm.send_obd(&cmd).await {
            Ok(resp) if !resp.lines.is_empty() => {
                // Parse the response — look for positive response byte
                let line = &resp.lines[0];
                let parts: Vec<&str> = line.split_whitespace().collect();
                // parts[0] = header (e.g. "7C8"), rest = data bytes
                if parts.len() >= 3 {
                    let bytes: Vec<u8> = parts[1..]
                        .iter()
                        .filter_map(|s| u8::from_str_radix(s, 16).ok())
                        .collect();
                    // Check for positive response: first data byte after length
                    // Single frame: [length] [service+0x40] [id] [data...]
                    if let Some(pos) = bytes.iter().position(|&b| b == positive_response) {
                        let data = if pos + 2 < bytes.len() {
                            bytes[pos + 2..].to_vec()
                        } else {
                            vec![]
                        };
                        found.push(KwpResult {
                            _service: service,
                            local_id: id,
                            data,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    elm.at_command("ATST FF").await?;
    elm.at_command("ATAR").await?;
    elm.set_header("7DF").await?;
    Ok(found)
}

/// Full security unlock: request seed, try common Toyota key algorithms, send key.
async fn security_unlock_impl(elm: &mut Elm327, ecu: &str, level: u8) -> Result<String> {
    elm.set_header(ecu).await?;
    let mut log = String::new();

    // Step 1: Request seed
    let seed_cmd = format!("{:02X} {:02X}", uds::SECURITY_ACCESS, level);
    let raw_seed = elm.send_passthrough(&seed_cmd).await?;

    // Parse seed from raw response
    let seed_lines: Vec<&str> = raw_seed.split(|c| c == '\r' || c == '\n')
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .filter(|l| l.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
        .collect();

    let owned: Vec<String> = seed_lines.iter().map(|s| s.to_string()).collect();
    let payload = crate::protocol::isotp::reassemble(&owned)?;

    let expected_resp = uds::SECURITY_ACCESS + 0x40; // 0x67
    if payload.is_empty() || payload[0] != expected_resp || payload.len() < 3 {
        elm.set_header("7DF").await?;
        return Err(crate::error::Error::Protocol(
            format!("unexpected seed response: {:02X?}", payload)
        ));
    }

    let seed = &payload[2..]; // skip 67 [level]
    log.push_str(&format!("Seed ({} bytes): {}\n", seed.len(),
        seed.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")));

    // Step 2: Try key algorithms (each gets a fresh seed if needed)
    let key_level = level + 1; // seed level N, key level N+1
    // NRC 0x13 for all key lengths 1-5 means the key must be 6 bytes.
    // Total message = 27 62 + 6 bytes = 8 bytes → needs multi-frame.
    // The ELM327 clone can't auto-frame, so we try multiple send strategies.
    let algorithms: Vec<(&str, fn(&[u8]) -> Vec<u8>)> = vec![
        ("identity (seed=key)", |s| s.to_vec()),
        ("bitwise NOT", |s| s.iter().map(|b| !b).collect()),
        ("XOR 0xAA", |s| s.iter().map(|b| b ^ 0xAA).collect()),
        ("XOR 0x55", |s| s.iter().map(|b| b ^ 0x55).collect()),
        ("+1 per byte", |s| s.iter().enumerate().map(|(i, b)| b.wrapping_add((i + 1) as u8)).collect()),
        ("-1 per byte", |s| s.iter().enumerate().map(|(i, b)| b.wrapping_sub((i + 1) as u8)).collect()),
        ("reverse", |s| { let mut v = s.to_vec(); v.reverse(); v }),
        ("swap pairs", |s| vec![s[1], s[0], s[3], s[2], s[5], s[4]]),
    ];

    for (name, algo) in &algorithms {
        // Get a fresh seed for each attempt (seed is single-use)
        let fresh_raw = elm.send_passthrough(&seed_cmd).await?;
        let fresh_lines: Vec<&str> = fresh_raw.split(|c| c == '\r' || c == '\n')
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .filter(|l| l.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
            .collect();
        let fresh_owned: Vec<String> = fresh_lines.iter().map(|s| s.to_string()).collect();
        let fresh_payload = match crate::protocol::isotp::reassemble(&fresh_owned) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if fresh_payload.len() < 3 || fresh_payload[0] != expected_resp {
            log.push_str(&format!("Seed request failed, stopping\n"));
            break;
        }
        let fresh_seed = &fresh_payload[2..];

        let key = algo(fresh_seed);
        log.push_str(&format!("Try {}: key={}\n", name,
            key.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")));

        // Build key response: 27 [key_level] [key_bytes]
        let mut key_msg: Vec<u8> = vec![uds::SECURITY_ACCESS, key_level];
        key_msg.extend_from_slice(&key);

        let key_hex = key_msg.iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" ");

        // Ensure CAN auto-formatting is ON so the OBDLink handles
        // multi-frame ISO-TP automatically (required for >7 byte messages).
        let _ = elm.at_command("ATCAF1").await;
        // Also ensure we have the right header set (may get reset between attempts)
        let _ = elm.set_header(ecu).await;

        // Send key through passthrough — the OBDLink MX+ with ATCAF1
        // will automatically handle ISO-TP multi-frame framing.
        let response = elm.send_passthrough(&key_hex).await;

        match response {
            Ok(resp) => {
                let resp_upper = resp.to_uppercase();
                if resp_upper.contains("67") {
                    log.push_str(&format!("UNLOCKED with algorithm: {}\n", name));
                    log.push_str(&format!("Response: {}\n", resp.trim()));
                    elm.set_header("7DF").await?;
                    return Ok(log);
                } else if resp_upper.contains("35") {
                    log.push_str(&format!("  Invalid key (NRC 0x35)\n"));
                } else if resp_upper.contains("36") {
                    log.push_str(&format!("  Exceeded attempts (NRC 0x36) — waiting 10s\n"));
                    tokio::time::sleep(std::time::Duration::from_secs(11)).await;
                } else if resp_upper.contains("37") {
                    log.push_str(&format!("  Required time delay not expired (NRC 0x37) — waiting 10s\n"));
                    tokio::time::sleep(std::time::Duration::from_secs(11)).await;
                } else {
                    log.push_str(&format!("  Response: {}\n", resp.trim()));
                }
            }
            Err(e) => {
                log.push_str(&format!("  Send error: {}\n", e));
            }
        }
    }

    elm.set_header("7DF").await?;
    log.push_str("All algorithms failed\n");
    Ok(log)
}

/// Send data > 7 bytes by manually constructing ISO-TP frames.
/// The ELM327 clone can't auto-frame multi-frame transmissions.
async fn send_multi_frame_impl(elm: &mut Elm327, data_hex: &str) -> Result<String> {
    let data_bytes: Vec<u8> = data_hex
        .split_whitespace()
        .filter_map(|s| u8::from_str_radix(s, 16).ok())
        .collect();

    if data_bytes.len() <= 7 {
        // Fits in single frame — send normally
        return elm.send_passthrough(data_hex).await;
    }

    let total_len = data_bytes.len();

    // Disable CAN auto-formatting so we can send raw ISO-TP frames
    elm.at_command("ATCAF0").await?;

    // Build First Frame: 10 [len] [first 6 data bytes]
    let ff_data = &data_bytes[..6.min(total_len)];
    let ff = format!(
        "10 {:02X} {}",
        total_len,
        ff_data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
    );

    // Send First Frame and read Flow Control response
    let fc_response = elm.send_passthrough(&ff).await?;

    // Check we got a flow control frame (starts with 30)
    let fc_ok = fc_response.split(|c| c == '\r' || c == '\n')
        .any(|line| {
            let trimmed = line.trim();
            // Look for "30" in the data portion (after header)
            trimmed.split_whitespace()
                .nth(1)
                .map(|b| b.starts_with("30"))
                .unwrap_or(false)
        });

    if !fc_ok {
        elm.at_command("ATCAF1").await?;
        return Err(crate::error::Error::Protocol(
            format!("no flow control received: {}", fc_response.chars().take(60).collect::<String>())
        ));
    }

    // Build Consecutive Frames
    let mut offset = 6;
    let mut seq: u8 = 1;
    let mut last_response = String::new();

    while offset < total_len {
        let end = (offset + 7).min(total_len);
        let cf_data = &data_bytes[offset..end];
        // Pad to 7 bytes
        let mut padded = cf_data.to_vec();
        while padded.len() < 7 {
            padded.push(0x00);
        }
        let cf = format!(
            "2{:X} {}",
            seq & 0x0F,
            padded.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
        );

        last_response = elm.send_passthrough(&cf).await?;
        seq += 1;
        offset = end;
    }

    // Re-enable auto-formatting
    elm.at_command("ATCAF1").await?;

    Ok(last_response)
}

async fn read_did_value_impl(elm: &mut Elm327, did: u16) -> Result<Vec<u8>> {
    let request = uds::read_data_by_identifier(did);
    let response = uds::send_uds(elm, &request).await?;
    if response.len() < 3 {
        return Err(crate::error::Error::Protocol("read response too short".into()));
    }
    Ok(response[3..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_simple_formula() {
        assert_eq!(evaluate_simple_formula("A - 40", &[80]), Some(40.0));
        assert_eq!(evaluate_simple_formula("(A * 256 + B) / 4", &[0x1A, 0xF8]), Some(1726.0));
        assert_eq!(evaluate_simple_formula("UNKNOWN", &[42]), None);
    }

    #[test]
    fn test_found_ecu_clone() {
        let ecu = FoundEcu {
            name: "ECM".into(),
            tx_address: "7E0".into(),
        };
        let cloned = ecu.clone();
        assert_eq!(cloned.name, "ECM");
    }

    #[test]
    fn test_dtc_entry_clone() {
        let dtc = DtcEntry { code: "P0300".into() };
        let cloned = dtc.clone();
        assert_eq!(cloned.code, "P0300");
    }

    #[test]
    fn test_enhanced_did_result_clone() {
        let r = EnhancedDidResult {
            did: 0x0100,
            name: "Test".into(),
            value: Some(42.0),
            unit: "V".into(),
            raw_hex: "2A".into(),
        };
        let cloned = r.clone();
        assert_eq!(cloned.did, 0x0100);
        assert_eq!(cloned.value, Some(42.0));
    }
}
