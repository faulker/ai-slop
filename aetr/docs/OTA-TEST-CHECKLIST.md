# Over-the-Air Test Checklist

Manual validation procedure for aetr on real hardware. Do this in two stages: acoustic (speaker to mic, no radios) first, then real analog FM radios. Every automated test already passes before you start (`cargo test -p aetr-core --release`, macOS `--selftest`, Android JVM tests), so any failure here is an analog-path problem, not a protocol bug.

## Setup common to both stages

- [ ] Two devices running aetr (any mix of macOS and Android).
- [ ] Same passphrase entered on both, character for character.
- [ ] Same modem mode selected on both (start with B170). Mode mismatch means silent decode failure, no error.
- [ ] Volume/mic permissions granted; correct input and output devices selected in the app.
- [ ] Quiet room for the acoustic stage.

## Stage 1: acoustic (speaker to mic)

Purpose: prove the analog audio path (DAC, speaker, air, mic, ADC) before adding radios.

- [ ] Place the devices ~30 cm apart, sender speaker facing receiver mic.
- [ ] Set sender volume to ~60%.
- [ ] Send a short text ("hello ota"). Receiver shows Syncing during the burst, then the text.
- [ ] Send a multi-chunk text (> 143 bytes). Watch the Progress count climb; message completes.
- [ ] Send a short voice clip (2-3 s). Received clip plays back intelligibly (codec2 at 1200 bps is robotic but understandable).
- [ ] Repeat the short text at low volume (~20%) and high volume (~100%). Note the levels where decode starts failing; clipping at full volume is the usual culprit.
- [ ] Wrong-passphrase check: change the passphrase on one side, send a text, confirm nothing appears (frames are silently dropped) and no crash.

## Stage 2: real radios

Two analog FM radios (FRS/GMRS/ham as licensed). Device A audio out into radio A mic, radio B speaker into device B mic. Cable connections beat acoustic coupling if you have the adapters.

### Levels

- [ ] Start with sender device volume ~50% into the radio mic. Too hot overdeviates FM and clips; too low loses sync in noise.
- [ ] Send the short text at several TX volume levels; record the working range.
- [ ] On the receiving radio, set speaker volume to a moderate, consistent level into the device mic. Record what works.

### PTT: manual vs VOX, and TX delay

- [ ] Manual PTT: key up, then hit send; the configurable TX delay (default 1000 ms) plus 0.1 s lead-in covers the keying. Unkey after the audio clearly ends (there is 0.5 s of tail).
- [ ] VOX, if the radio supports it: enable the "VOX primer tone" toggle so the TX delay carries a quiet tone that trips VOX before the preamble. If the first burst of a multi-burst message consistently fails but later ones decode, VOX attack time is eating the preamble; raise the TX delay, increase VOX sensitivity, or use manual PTT.
- [ ] Tune the TX delay: drop it toward 0 until the first burst starts failing, then add ~500 ms margin. Record the working value for your radio.

### Bluetooth-connected radio (if your radio does TX/RX audio over Bluetooth)

- [ ] Pair the radio with the device; pick it as input AND output in the app's device pickers (Android prompts for the Bluetooth permission on Android 12+).
- [ ] Confirm the app reports the Bluetooth route as active before sending (SCO setup is asynchronous; the first second after connecting can be dead air).
- [ ] Start in the robust B85 mode: the Bluetooth hands-free link is narrowband with a lossy speech codec (CVSD/mSBC) and can degrade denser modes. Step up to B128/B170 only if B85 is solid.
- [ ] Send the short text; if the first burst never decodes but later ones do, the radio's Bluetooth key-up latency is eating the preamble — raise the TX delay until the first burst is reliable, then record the value.
- [ ] Repeat the multi-chunk text and a short voice clip over the Bluetooth path.
- [ ] Check RX: leave the app receiving via the radio's Bluetooth mic path for several minutes; confirm no route flapping (SCO drop/reconnect) interrupts reassembly.

### Squelch

- [ ] Set receiving radio squelch as low as tolerable. Tight squelch chops burst edges.
- [ ] Confirm the squelch tail (the noise burst when the sender unkeys) does not corrupt decode; the trailing silence should absorb it.
- [ ] If available, test with squelch fully open (constant hiss between bursts). Decode should still work; the modem syncs on the preamble.

### Mode matching

- [ ] B170 both ends: multi-chunk text and voice work at close range.
- [ ] B85 both ends: same tests; expect longer airtime, better noise tolerance.
- [ ] Deliberate mismatch (B170 vs B85): confirm nothing decodes and nothing crashes, then restore.

### Repair-request (ARQ) flow

- [ ] Send a long text (> 1 kB, so 10+ chunks). Briefly interrupt reception mid-message (unplug the receive audio cable or block the mic for one burst).
- [ ] Receiver shows an incomplete Progress count and stays in Receiving.
- [ ] Trigger the repair request on the receiver; confirm the sender receives it and offers/transmits the repair burst.
- [ ] Key the repair transmission; the message completes on the receiver.
- [ ] Confirm a repair request for an already-complete message is rejected client-side.

### Voice degradation

- [ ] Send a 10 s voice clip at close range: plays fully, no gaps.
- [ ] Send it again, interrupting one burst mid-transmission: clip plays with a silence gap at the interrupted span's position, surrounding audio intact.
- [ ] Increase distance/reduce signal (or add interference) until spans start dropping; confirm degradation is graceful (gaps, not garbage or crashes).
- [ ] Try a clip near the configured cap (default 30 s); note the displayed airtime estimate and confirm it matches reality within a few seconds.

## Record for each pass

Radio model, band/channel, power, cable vs acoustic coupling, TX/RX volume settings, mode, distance, and which checklist items failed. Failures at a given level/mode combination feed back into README guidance.
