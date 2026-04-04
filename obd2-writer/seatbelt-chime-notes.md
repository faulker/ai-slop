# Seatbelt Chime & Body Customization DID Discovery Notes

## Confirmed ECU UDS Addresses (from commaai/opendbc)

From [opendbc/car/toyota/values.py](https://github.com/commaai/opendbc/blob/master/opendbc/car/toyota/values.py):

| ECU | UDS Address | Notes |
|-----|-------------|-------|
| **Combination Meter** | **0x7C0** | Confirmed — responds to UDS |
| Body Control Module | (0x750, 0x40) | Sub-addressed via 0x750, KWP protocol |
| Electronic Parking Brake | (0x750, 0x2C) | Sub-addressed via 0x750 |
| Central Gateway | (0x750, 0x5F) | Sub-addressed via 0x750, KWP protocol |
| Telematics | (0x750, 0xC7) | Sub-addressed via 0x750 |
| EPS/EMPS | 0x7A0, 0x7A1 | KWP protocol |
| Steering Angle Sensor | 0x7B3 | KWP protocol |
| SRS Airbag | 0x780 | |
| 2nd SRS Airbag | 0x784 | |
| HVAC | 0x7C4 | |
| 2nd ABS / Brake/EPB | 0x730 | |
| ECM | 0x7E0 | |
| Transmission | 0x7E1, 0x701 | |
| HV Battery | 0x713, 0x747 | Hybrid only |
| Motor Generator | 0x716, 0x724 | Hybrid only |

**Key finding**: 0x750 is a shared gateway address. The BCM, Central Gateway, EPB, and Telematics all live behind it with sub-addressing. This explains why our ECU scan finds "BCM (Body)" at 0x750 — it's actually a gateway that routes to multiple modules.

### CAN Bus Message IDs (from opendbc DBC files)

These are broadcast CAN messages (not UDS diagnostic), but they confirm which module produces each signal:

| CAN ID | Name | Key Signals |
|--------|------|-------------|
| 1552 (0x610) | BODY_CONTROL_STATE_2 | Meter brightness, slider dimmed |
| 1553 (0x611) | UI_SETTING | Units, odometer |
| 1556 (0x614) | BLINKERS_STATE | Turn signals, hazard |
| 1568 (0x620) | BODY_CONTROL_STATE | **SEATBELT_DRIVER_UNLATCHED**, door open states, parking brake, meter dimmed |
| 1570 (0x622) | LIGHT_STALK | Headlight mode, low beam, fog, tail, parking |
| 1571 (0x623) | CERTIFICATION_ECU | Door lock feedback, keyfob lock/unlock feedback |
| 1592 (0x638) | DOOR_LOCKS | Lock status, locked via keyfob |

Note: CAN ID 1568 broadcasts `SEATBELT_DRIVER_UNLATCHED` — this is the combination meter reporting seatbelt status, confirming that the Combination Meter ECU (0x7C0) manages seatbelt-related functions.

## ECU-to-Setting Mapping (from Toyota Service Manual)

Based on the Toyota Tacoma (2015-2018) Service Manual, Techstream settings are distributed across three ECUs — NOT all on the BCM at 0x750 as the TOML placeholders assumed.

### Combination Meter ECU (likely 0x7C0)
Techstream path: `Body Electrical > Combination Meter > Utility > Customize`

Settings confirmed in this ECU:
- **Seatbelt warning buzzer (driver)** — ON/OFF
- **Seatbelt warning buzzer (passenger)** — ON/OFF
- **Lane change flashing times** — OFF, 3-7 flashes
- **Key reminder sound cycle** — Fast/Normal/Slow
- **ODO display time after ignition off** — 30s/60s/600s/OFF

### Main Body ECU / Multiplex Network Body ECU (address TBD — likely 0x750)
Techstream path: `Body Electrical > Main Body > Utility > Customize`

**Door Lock Settings:**
- Automatic Door Lock Function — OFF / Link Shift / Link Speed
- Automatic Door Unlock Function — OFF / Link D-door / Link Shift
- Unlock Key Twice — OFF / ON (driver-first vs all doors)
- Auto Lock Time — 30s / 60s / 120s
- Wireless Auto Lock — OFF / ON
- Open Door Warning — OFF / ON

**Wireless Remote Settings:**
- Wireless Control — OFF / ON
- Hazard Answer Back — OFF / ON (flash on lock/unlock)
- Wireless Buzzer Resp — OFF / ON (horn chirp on lock)
- Wireless Buzzer Vol — Level 0-7 (default: 5)
- Panic Function — OFF / ON

**Lighting Settings:**
- DRL Function — OFF / ON
- Light Auto OFF Delay — OFF / 30s / 60s / 90s (headlight auto-off timer)
- Headlamps-On Sensitivity — -2 to +2
- Headlamps Auto-Off Timer — Off / 30s / 60s / 90s

**Illuminated Entry:**
- Lighting Time — 7.5s / 10s / 15s / 30s
- Interior lights when ACC OFF — OFF / ON
- Interior lights when door unlocked — OFF / ON
- Room light when approached (smart key) — OFF / ON

### Certification ECU / Smart Key ECU (address TBD)
Techstream path: `Body Electrical > Certification > Utility > Customize`

- Park Wait Time — 0.5s / 1.5s / 2.5s / 5s
- Ignition Available Area — Front / All
- Door Unlock Mode2 — All / Driver
- Engine Start Indicator — OFF / ON
- Key Low Battery Warning — OFF / ON

## Corrected ECU Assignments for TOML Placeholders

| TOML Setting | Old ECU | Correct ECU | Notes |
|---|---|---|---|
| Seatbelt Warning Buzzer | BCM (750) | Combination Meter (7C0?) | Confirmed in service manual data list |
| Auto Door Lock (by Speed) | BCM (750) | Main Body ECU (750?) | "Link Speed" option |
| Auto Door Unlock (Shift to P) | BCM (750) | Main Body ECU (750?) | "Link Shift" option |
| Daytime Running Lights (DRL) | BCM (750) | Main Body ECU (750?) | "DRL Function" ON/OFF |
| Turn Signal Lane-Change Flashes | BCM (750) | Combination Meter (7C0?) | Data list shows 3-7 flashes |
| Headlight Auto-Off Timer | BCM (750) | Main Body ECU (750?) | OFF/30s/60s/90s |
| Key-Off Power Timer | BCM (750) | Unknown | Not found in service manual pages |
| Horn Chirp on Lock | BCM (750) | Main Body ECU (750?) | "Wireless Buzzer Resp" ON/OFF |
| Reverse Tilt Mirrors | BCM (750) | Unknown | Not found in service manual pages |
| Smart Key Detection Range | BCM (750) | Certification ECU | Park Wait Time / Ignition Area |

## Unverified Claim (from J2534 source)

- **ECU**: 0x7C0 (Combination Meter)
- **DID**: 0x1A01 (Warning settings block)
- **Byte 0**: Driver seatbelt chime (0x00=OFF, 0x01=ON)
- **Byte 1**: Passenger seatbelt chime (0x00=OFF, 0x01=ON)

Neither 0x7C0 nor 0x1A01 could be independently verified from public sources.

## DID Discovery Strategy

Since we now know WHICH ECU each setting lives on, we can do targeted scans:

### For Combination Meter settings (seatbelt, lane change flashes):
```
# Verify ECU responds
ATSH 7C0
3E 00

# If no response, try nearby addresses
ATSH 7C1
3E 00

# Scan DID ranges on whichever responds
# Toyota manufacturer-specific range:
Scan 1A00-1AFF on 7C0
# Also try:
Scan 0100-01FF on 7C0
Scan B000-B0FF on 7C0
```

### For Main Body ECU settings (door locks, DRL, headlights, horn chirp):
```
ATSH 750
3E 00

# Scan ranges
Scan 0100-01FF on 750
Scan 1A00-1AFF on 750
Scan B000-B1FF on 750
```

### For Certification ECU settings (smart key):
```
# Try common Toyota certification ECU addresses
ATSH 760
3E 00

ATSH 740
3E 00
```

## Sniffing Carista (Best Method)

If you have Carista, use Wireshark to capture Bluetooth traffic while toggling each setting. This reveals the exact ECU address and DID for every customization.

## Important Notes

- The 0xB1xx DID addresses in the TOML are confirmed WRONG — they were fabricated placeholders
- The Main Body ECU may or may not be at 0x750 — it could also be 0x750 responding under a different name in our ECU scan
- "Coming home lights" (headlight delay after exit) = "Light Auto OFF Delay" on Main Body ECU
- Lane change flash count is on the Combination Meter, NOT the lighting/body ECU
- The Tacoma has fewer customization options than other Toyota models (Camry, Highlander)
- Disabling seatbelt chime only stops the extended warning — the initial 5 dings always remain

## Updated ECU Address Assignments

Based on all sources combined:

| Setting | ECU | UDS Address | Confidence |
|---|---|---|---|
| Seatbelt chime (driver & passenger) | Combination Meter | **0x7C0** | **High** — opendbc confirms address, service manual confirms ECU |
| Lane change flash count | Combination Meter | **0x7C0** | **High** — service manual data list |
| Key reminder sound | Combination Meter | **0x7C0** | High |
| ODO display after ignition off | Combination Meter | **0x7C0** | High |
| Auto door lock/unlock | Main Body ECU | **0x750** (sub-addr 0x40?) | Medium — may need KWP not UDS |
| DRL on/off | Main Body ECU | **0x750** | Medium |
| Headlight auto-off timer | Main Body ECU | **0x750** | Medium |
| Horn chirp on lock | Main Body ECU | **0x750** | Medium |
| Wireless buzzer volume | Main Body ECU | **0x750** | Medium |
| Interior light timers | Main Body ECU | **0x750** | Medium |
| Smart key area / park wait | Certification ECU | **Unknown** | Low — CAN ID 1571 but UDS address unknown |
| Reverse tilt mirrors | Unknown | Unknown | Not found in any source |
| Key-off power timer | Unknown | Unknown | Not found in any source |

**Important**: opendbc notes that the BCM at 0x750 uses KWP protocol (not UDS) with sub-addressing. This means UDS Mode 22/2E may not work for door lock, DRL, and lighting settings. Those may require KWP2000 protocol instead. The Combination Meter at 0x7C0 responds to UDS, so seatbelt chime and lane-change flashes are the most accessible targets.

## Sources

- [Toyota Tacoma Service Manual: Customize Parameters - Lighting System](https://www.ttguide.net/customize_parameters-2026.html)
- [Toyota Tacoma Service Manual: Customize Parameters - Power Door Lock](https://www.ttguide.net/customize_parameters-1756.html)
- [Toyota Tacoma Service Manual: Customize Parameters - Wireless Door Lock](https://www.ttguide.net/customize_parameters-1799.html)
- [Toyota Tacoma Service Manual: Customize Parameters - Smart Key System](https://www.ttguide.net/customize_parameters-1770.html)
- [Toyota Tacoma Service Manual: Data List - Meter/Gauge System](https://www.ttguide.net/data_list_active_test-2076.html)
- [Toyota Tacoma Service Manual: DRL Relay Circuit](https://www.ttguide.net/daytime_running_light_relay_circuit-2042.html)
- [3rd Gen Tips and Tricks - Tacoma3G](https://tacoma3g.com/threads/3rd-gen-tips-and-tricks.8138/)
- [Comprehensive TechStream options checklist - Tacoma World](https://www.tacomaworld.com/threads/comprehensive-checklist-for-all-common-techstream-customizable-options.429205/)
- [commaai/opendbc Toyota values.py - ECU addresses](https://github.com/commaai/opendbc/blob/master/opendbc/car/toyota/values.py)
- [commaai/opendbc Toyota _toyota_2017.dbc - CAN message definitions](https://github.com/commaai/opendbc/blob/master/opendbc/dbc/generator/toyota/_toyota_2017.dbc)
- [Techstream Tips and Tricks - Tacoma World](https://www.tacomaworld.com/threads/techstream-tips-and-tricks.597998/)
- [Where is seatbelt chime option in Techstream - ToyotaNation](https://www.toyotanation.com/threads/where-is-seatbelt-chime-option-in-techstream-v12-30-017.1629566/)
