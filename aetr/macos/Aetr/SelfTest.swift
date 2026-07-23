import Foundation

/// Passphrase and plaintext of testdata/golden_text.wav, kept in sync with
/// the golden constants in core/src/tests.rs.
private let goldenPassphrase = "correct horse battery staple"
private let goldenText =
    "aetr golden vector: the quick brown fox jumps over the lazy dog 0123456789"

/// Headless end-to-end check used by the `--selftest` launch argument:
/// creates two sessions sharing a passphrase, encodes "hello" on one, feeds
/// the PCM burst into the other's receiver in audio-callback-sized blocks,
/// and expects a matching Text event. Prints SELFTEST PASS/FAIL to stdout.
func runSelfTest() -> Bool {
    do {
        let config = SessionConfig(
            passphrase: "selftest-passphrase", mode: .b170, voiceCapSecs: 30, txDelayMs: 0,
            voxPrimer: false)
        let sender = try AetrSession(config: config)
        let receiver = try AetrSession(config: config)

        let burst = try sender.encodeText(text: "hello")
        guard !burst.isEmpty else {
            print("SELFTEST FAIL: encodeText returned an empty burst")
            return false
        }

        // Feed in ~0.1 s blocks to exercise the streaming decode path, then
        // half a second of silence so the modem flushes any tail state.
        let samples = burst + [Float](repeating: 0, count: 24_000)
        let block = 4_800
        var offset = 0
        while offset < samples.count {
            let end = min(offset + block, samples.count)
            receiver.pushRx(pcm48k: Array(samples[offset..<end]))
            offset = end
        }

        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            for event in receiver.pollEvents() {
                switch event {
                case let .text(_, text):
                    if text == "hello" {
                        print("SELFTEST PASS")
                        return true
                    }
                    print("SELFTEST FAIL: expected \"hello\", got \"\(text)\"")
                    return false
                case let .failed(_, reason):
                    print("SELFTEST FAIL: receive failed: \(reason)")
                    return false
                default:
                    continue
                }
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        print("SELFTEST FAIL: no Text event within 5 s")
        return false
    } catch {
        print("SELFTEST FAIL: \(error)")
        return false
    }
}

/// Reads a canonical 48 kHz mono 16-bit PCM WAV (the golden vector format
/// written by the core tests) and returns f32 samples, or nil on any
/// format mismatch.
private func readWav16Mono48k(path: String) -> [Float]? {
    guard let bytes = FileManager.default.contents(atPath: path), bytes.count > 44,
          bytes[0...3].elementsEqual("RIFF".utf8), bytes[8...11].elementsEqual("WAVE".utf8)
    else { return nil }

    func u16(_ at: Int) -> Int { Int(bytes[at]) | Int(bytes[at + 1]) << 8 }
    func u32(_ at: Int) -> Int {
        u16(at) | Int(bytes[at + 2]) << 16 | Int(bytes[at + 3]) << 24
    }

    var pos = 12
    var data: (offset: Int, length: Int)?
    while pos + 8 <= bytes.count {
        let len = u32(pos + 4)
        guard pos + 8 + len <= bytes.count else { return nil }
        if bytes[pos...pos + 3].elementsEqual("fmt ".utf8) {
            // PCM, mono, 48 kHz, 16-bit — anything else is not a golden wav.
            guard u16(pos + 8) == 1, u16(pos + 10) == 1,
                  u32(pos + 12) == 48_000, u16(pos + 22) == 16
            else { return nil }
        } else if bytes[pos...pos + 3].elementsEqual("data".utf8) {
            data = (pos + 8, len)
        }
        pos += 8 + len + (len & 1) // chunks are word-aligned
    }
    guard let d = data else { return nil }
    var pcm = [Float]()
    pcm.reserveCapacity(d.length / 2)
    for i in stride(from: d.offset, to: d.offset + d.length - 1, by: 2) {
        let s = Int16(truncatingIfNeeded: u16(i))
        pcm.append(Float(s) / 32768.0)
    }
    return pcm
}

/// Decodes a golden wav through a real receiving session (`pushRx` in
/// audio-callback-sized blocks) and verifies the expected plaintext.
/// Prints GOLDEN PASS/FAIL to stdout.
func runGoldenDecode(path: String) -> Bool {
    guard let pcm = readWav16Mono48k(path: path) else {
        print("GOLDEN FAIL: \(path) is not a 48 kHz mono 16-bit PCM wav")
        return false
    }
    do {
        let config = SessionConfig(
            passphrase: goldenPassphrase, mode: .b170, voiceCapSecs: 30, txDelayMs: 0,
            voxPrimer: false)
        let receiver = try AetrSession(config: config)

        let samples = pcm + [Float](repeating: 0, count: 24_000)
        let block = 4_800
        var offset = 0
        while offset < samples.count {
            let end = min(offset + block, samples.count)
            receiver.pushRx(pcm48k: Array(samples[offset..<end]))
            offset = end
        }

        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            for event in receiver.pollEvents() {
                switch event {
                case let .text(_, text):
                    if text == goldenText {
                        print("GOLDEN PASS")
                        return true
                    }
                    print("GOLDEN FAIL: unexpected text \"\(text)\"")
                    return false
                case let .failed(_, reason):
                    print("GOLDEN FAIL: receive failed: \(reason)")
                    return false
                default:
                    continue
                }
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        print("GOLDEN FAIL: no Text event within 5 s")
        return false
    } catch {
        print("GOLDEN FAIL: \(error)")
        return false
    }
}

/// Locates testdata/golden_text.wav by walking up from the app executable
/// (works from macos/build/<config>/Aetr.app in a dev checkout) and runs
/// the golden decode. Skips with a note, still passing, when the checkout
/// layout isn't present (e.g. a relocated .app).
func runGoldenDecodeIfPresent() -> Bool {
    var dir = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
    for _ in 0..<8 {
        let candidate = dir.appendingPathComponent("testdata/golden_text.wav")
        if FileManager.default.fileExists(atPath: candidate.path) {
            return runGoldenDecode(path: candidate.path)
        }
        dir.deleteLastPathComponent()
    }
    print("GOLDEN SKIP: testdata/golden_text.wav not found above executable")
    return true
}
