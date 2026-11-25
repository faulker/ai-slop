# BMW R 1250 GS Adventure (K51): findings

Status: **nothing confirmed on the bike yet.** Everything below is a hypothesis to
check with read-only commands. Fill in results as you go and date each one.

## Connection
- Hardware: OBDLink MX+ plus the 10-pin round to OBD2 adapter that MotoScan uses.
- MotoScan identifies the bike as R series, K51.
- Pair the MX+ with one app at a time. Close MotoScan before running obd2-writer.

## Addressing hypotheses (Phase 0)
| # | Hypothesis | How to test | Result |
|---|---|---|---|
| 1 | D-CAN at 500 kbps, 11-bit (`ATSP6`) | `obd2 connect`, then `obd2 monitor -o k51-bus.json` | |
| 2 | BMW extended addressing: tester `6F1`, module byte first, reply from `0x600 + module` | `obd2 bmw sweep` | |
| 3 | Plain 11-bit UDS (7xx) | `obd2 ecus --sweep` | |
| 4 | Multi-frame replies work (flow control `ATFCSH 6F1`, `ATFCSD <module> 30 00 00`, `ATFCSM1`) | `obd2 bmw identify <module>` returns a full 17-char VIN | |
| 5 | Replies carry tester byte `F1` before the PCI byte (stripped in `elm327.rs`) | `obd2 -v bmw identify <module>`, look at raw RX lines | |

If #2 finds nothing but #1 shows traffic on `6xx` IDs, the reply base or tester
address is different. Look at the monitor capture and adjust `EXT_RESPONSE_BASE`
or `TESTER_HEADER`.

## Modules found
| Address | Target | System name / part no. | Notes |
|---|---|---|---|
| | | | |

Add confirmed modules to `bmw_k51.toml` as `[[module]]` entries.

## Settings mapped by diff
| Setting | Module | DID | Byte/mask | Values | Date |
|---|---|---|---|---|---|
| | | | | | |

## Write access (Phase 3, later)
- Does a coding DID write return NRC 0x33 (security access needed)? Not tested.
