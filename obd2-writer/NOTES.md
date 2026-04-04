  TOML — Replaced the wrong UDS DID 0x1A01 with the verified KWP local ID 0xA7:
  - Protocol: kwp (uses service 0x21 read / 0x3B write)
  - Values: 0xC0 = ON, 0x00 = OFF

  Code — Added KWP2000 protocol support:
  - DidDefinition now has a protocol field ("uds" or "kwp")
  - Read: KWP uses service 0x21, UDS uses 0x22
  - Write: KWP uses service 0x3B with NRC 0x78 (responsePending) handling
  - Settings screen passes protocol through to all read/write operations

  Go to the Settings tab — you should see "Seatbelt Warning Chime" listed. Press r to read its current value (should show C0). Then you can write 00 to disable it.

  But first — test it from the Raw tab to make sure it works:

  ATSH 7C0
  21 A7
  3B A7 00
  21 A7