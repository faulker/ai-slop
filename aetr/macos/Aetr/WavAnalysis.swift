import Accelerate
import Foundation

/// Offline analysis of a captured 48 kHz mono WAV.
///
/// This is the payoff of the capture toggle: take the recording the Mac made
/// of a transmission that did not decode, replay it through a real receiving
/// session, and read off both what the audio looked like and which decode
/// stage the burst died at. No radio, no second device, and repeatable.

/// The modem's occupied band: 256 carriers at 6.25 Hz spacing on 1500 Hz.
private let bandLoHz: Float = 700
private let bandHiHz: Float = 2293.75

/// Level and spectral measurements of a capture, independent of decoding.
private struct Measurements {
    let peak: Float
    let rms: Float
    let dc: Float
    let clippedFraction: Float
    let inBandFraction: Float
}

/// Measures a capture: level, DC, clipping, and how much of the energy sits
/// inside the modem's band rather than being hum, hiss, or speech.
private func measure(_ pcm: [Float]) -> Measurements {
    var peak: Float = 0
    var sumSquares = 0.0
    var sum = 0.0
    var clipped = 0
    for s in pcm {
        let a = abs(s)
        if a > peak { peak = a }
        sumSquares += Double(s) * Double(s)
        sum += Double(s)
        if a >= 0.999 { clipped += 1 }
    }
    let n = max(1, pcm.count)
    return Measurements(
        peak: peak,
        rms: Float((sumSquares / Double(n)).squareRoot()),
        dc: Float(sum / Double(n)),
        clippedFraction: Float(clipped) / Float(n),
        inBandFraction: inBandEnergyFraction(pcm))
}

/// Fraction of total energy falling between 700 and 2294 Hz, via a Goertzel
/// sweep. A capture full of energy that is mostly outside the band is picking
/// up something other than the modem, and the sync metric normalizes against
/// total power, so that energy actively works against acquisition.
private func inBandEnergyFraction(_ pcm: [Float]) -> Float {
    guard pcm.count >= 4096 else { return 0 }
    // One 4096-point window from the loudest part of the capture.
    let window = 4096
    var bestStart = 0
    var bestEnergy = 0.0
    var i = 0
    while i + window <= pcm.count {
        var e = 0.0
        for j in i..<(i + window) { e += Double(pcm[j]) * Double(pcm[j]) }
        if e > bestEnergy {
            bestEnergy = e
            bestStart = i
        }
        i += window
    }
    guard bestEnergy > 0 else { return 0 }

    let slice = Array(pcm[bestStart..<(bestStart + window)])
    let binHz = Float(48_000) / Float(window)
    var inBand = 0.0
    var total = 0.0
    // Goertzel per bin up to Nyquist; window is small enough that this is
    // cheap and it avoids pulling in a full FFT setup for one measurement.
    var bin = 1
    while Float(bin) * binHz < 24_000 {
        let freq = Float(bin) * binHz
        let w = 2 * Float.pi * Float(bin) / Float(window)
        let coeff = 2 * cos(w)
        var s0: Float = 0
        var s1: Float = 0
        var s2: Float = 0
        for sample in slice {
            s0 = sample + coeff * s1 - s2
            s2 = s1
            s1 = s0
        }
        let power = Double(s1 * s1 + s2 * s2 - coeff * s1 * s2)
        total += power
        if freq >= bandLoHz && freq <= bandHiHz { inBand += power }
        bin += 1
    }
    return total > 0 ? Float(inBand / total) : 0
}

/// Formats a linear level as dBFS, or "-inf" for silence.
private func dbfs(_ level: Float) -> String {
    level > 1e-9 ? String(format: "%.1f dBFS", 20 * log10(level)) : "-inf dBFS"
}

/// Maps a mode name to the core enum.
private func parseMode(_ name: String) -> ModemMode? {
    switch name.lowercased() {
    case "b85": return .b85
    case "b128": return .b128
    case "b170": return .b170
    default: return nil
    }
}

/// Measures and decodes a captured WAV, printing a report. Returns true when
/// something decoded.
func runWavAnalysis(path: String, passphrase: String, mode: String) -> Bool {
    guard let modemMode = parseMode(mode) else {
        print("ANALYZE FAIL: unknown mode \"\(mode)\" (expected b85, b128, or b170)")
        return false
    }
    let pcm: [Float]
    do {
        pcm = try readWav48k(path: path)
    } catch {
        print("ANALYZE FAIL: \(error.localizedDescription)")
        return false
    }
    guard !pcm.isEmpty else {
        print("ANALYZE FAIL: \(path) contains no samples")
        return false
    }

    let m = measure(pcm)
    print("file      \(path)")
    print(String(format: "duration  %.2f s (%d samples at 48 kHz)",
                 Double(pcm.count) / 48_000, pcm.count))
    print("peak      \(dbfs(m.peak))")
    print("rms       \(dbfs(m.rms))")
    print(String(format: "dc offset %.4f", m.dc))
    print(String(format: "clipped   %.2f%% of samples", m.clippedFraction * 100))
    print(String(format: "in band   %.0f%% of energy in 700-2294 Hz", m.inBandFraction * 100))

    // The receiver has no AGC and the sync metric clamps its power term, so
    // there is a hard floor below which nothing syncs at all.
    if m.rms < 0.0014 {
        print("WARNING   below the ~-57 dBFS RMS sync floor; nothing will decode")
    }
    if m.clippedFraction > 0.01 {
        print("NOTE      heavily clipped, though the modem tolerates this well")
    }
    if m.inBandFraction < 0.2 {
        print("WARNING   most energy is outside the modem band; check the audio source")
    }

    let receiver: AetrSession
    do {
        receiver = try AetrSession(
            config: SessionConfig(
                passphrase: passphrase, mode: modemMode, voiceCapSecs: 30, txDelayMs: 0,
                voxPrimer: false))
    } catch {
        print("ANALYZE FAIL: could not open a session: \(error.localizedDescription)")
        return false
    }

    // Feed in audio-callback-sized blocks, plus trailing silence so the
    // block-oriented decoder flushes its last symbol.
    let samples = pcm + [Float](repeating: 0, count: 24_000)
    var offset = 0
    while offset < samples.count {
        let end = min(offset + 4_800, samples.count)
        receiver.pushRx(pcm48k: Array(samples[offset..<end]))
        offset = end
    }

    let c = receiver.rxCounters()
    print("")
    print("sync candidates  \(c.syncCandidates)")
    print("preamble failed  \(c.preambleFailed)")
    print("payload failed   \(c.payloadFailed)")
    print("frames decoded   \(c.framesDecoded)")
    print("rejected (auth)  \(receiver.rejectedFrames())")
    if c.framesDecoded > 0 || c.payloadFailed > 0 {
        print(String(format: "constellation    %.1f dB", receiver.lastSnrDb()))
        print(String(format: "carrier offset   %.1f Hz", receiver.lastCfoHz()))
    }
    print("")
    print(verdict(counters: c, rejected: receiver.rejectedFrames(), measured: m))

    var decoded = false
    for event in receiver.pollEvents() {
        switch event {
        case let .text(_, text, _):
            print("TEXT: \(text)")
            decoded = true
        case let .voice(_, pcm, _):
            print("VOICE: \(pcm.count) samples")
            decoded = true
        case let .failed(_, reason):
            print("FAILED: \(reason)")
        default:
            continue
        }
    }
    return decoded
}

/// Turns the stage counters and the level measurements into the one sentence
/// worth acting on.
///
/// The counters alone are not enough to separate the two ways of getting zero
/// syncs. A dead audio route and a burst mangled badly enough to defeat
/// acquisition both read as `syncCandidates == 0`, and they call for opposite
/// investigations. The measurements are what tell them apart: a healthy level
/// with the energy in band means the audio is arriving and something is
/// wrecking it, not that it never showed up.
private func verdict(counters c: RxCounters, rejected: UInt64, measured m: Measurements) -> String {
    if c.framesDecoded > 0 && rejected == 0 {
        return "VERDICT: the audio path is fine; frames decoded cleanly."
    }
    if c.framesDecoded > 0 && rejected > 0 {
        return "VERDICT: the modem decoded but authentication failed. "
            + "Passphrase mismatch between the two ends, or the wrong --passphrase here."
    }
    if c.syncCandidates == 0 {
        if m.rms < 0.0014 {
            return "VERDICT: the capture is below the sync floor. Raise the level at "
                + "the source (sending device volume, radio audio out, or input gain) "
                + "until the RMS above is comfortably better than -57 dBFS."
        }
        if m.inBandFraction < 0.2 {
            return "VERDICT: there is plenty of audio but almost none of it is in the "
                + "modem's 700-2294 Hz band. The input is picking up something other "
                + "than the burst: wrong device, wrong cable leg, or a squelched radio."
        }
        return "VERDICT: the burst is arriving at a healthy level and in the right "
            + "band, yet acquisition never fires. The signal is being altered badly "
            + "enough to defeat the sync symbol itself. A time-varying gain does "
            + "exactly this: check that the sending side is not routing the burst "
            + "through a voice-communication audio path, where the platform's AGC "
            + "and noise suppression apply."
    }
    if c.payloadFailed > 0 {
        return "VERDICT: bursts arrive and sync, then the payload will not decode. "
            + "Something is altering the signal in flight. A gain that moves on the "
            + "symbol timescale is the usual cause (an AGC or compressor in the "
            + "sending device's voice audio path), followed by dropped audio buffers."
    }
    return "VERDICT: bursts are detected but the preamble will not decode. "
        + "That is low signal-to-noise or energy outside the modem band; "
        + "check the in-band percentage above."
}
