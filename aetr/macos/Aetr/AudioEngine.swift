import AVFoundation
import CoreAudio

/// Duplex audio for one session: an AVAudioEngine whose input tap is
/// converted to 48 kHz mono Float32 and handed to `onInput` (called on the
/// audio thread), plus a player node that schedules encoded PCM bursts on
/// the selected output device.
final class AudioEngine {
    /// Everything at the FFI boundary is 48 kHz mono Float32.
    static let format48kMono = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 1, interleaved: false)!

    /// Stereo target used when the input device has two or more channels, so
    /// the channels reach `RxChannel.pick` intact instead of being averaged.
    static let format48kStereo = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: false)!

    /// Which leg of a stereo input carries the radio's receive audio.
    enum RxChannel: String, CaseIterable, Identifiable {
        /// Keep whichever channel is louder, per frame.
        case auto = "Auto"
        case left = "L"
        case right = "R"

        var id: String { rawValue }

        /// Collapses one stereo frame to mono.
        ///
        /// `auto` keeps the louder side rather than averaging. APRS and TRRS
        /// cables routinely wire audio on one leg only, where an L+R average
        /// costs 6 dB and folds the dead leg's noise floor in on top. The
        /// receiver has no AGC and stops syncing near -57 dBFS RMS, so that is
        /// a real share of the budget. Android already does this
        /// (pcm16StereoToMonoFloat in AudioEngine.kt).
        func pick(left l: Float, right r: Float) -> Float {
            switch self {
            case .left: return l
            case .right: return r
            case .auto: return abs(l) >= abs(r) ? l : r
            }
        }
    }

    /// Which physical channel(s) carry TX audio out to the radio's mic input.
    enum TxChannel: String, CaseIterable, Identifiable {
        /// Duplicate the burst on both channels (the default).
        case both = "L+R"
        case left = "L"
        case right = "R"

        var id: String { rawValue }
    }

    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private var converter: AVAudioConverter?
    /// True when the converter targets stereo and `handleInput` must collapse.
    private var converterIsStereo = false
    private(set) var running = false

    /// Which stereo leg to keep. Applies from the next tap buffer.
    var rxChannel: RxChannel = .auto

    /// Which channel(s) to write TX audio onto. Some APRS/TRRS cables and
    /// interfaces only carry the radio's mic-in line on one leg; when the
    /// wiring doesn't match what's expected, the radio keys up (PTT/VOX
    /// react to a control line or general audio presence) but the actual
    /// modulating audio never reaches the mic input, and only static goes
    /// out. This lets you find the working leg without re-wiring the cable.
    var txChannel: TxChannel = .both

    /// Reports a fault that leaves receive dead but the engine nominally
    /// running: a converter that could not be built, a conversion error, or a
    /// hardware reconfiguration. Without this the only symptom is a flatlined
    /// input meter, which is indistinguishable from a silent radio.
    var onFault: ((String) -> Void)?

    /// A private aggregate device we created to bridge distinct input/output
    /// devices, destroyed on the next `stop()`. `kAudioObjectUnknown` when the
    /// session runs on a single device and no aggregate was needed.
    private var aggregate: AudioDeviceID = kAudioObjectUnknown

    /// Receives 48 kHz mono samples from the mic tap. Audio-thread context;
    /// keep it cheap (push_rx is designed for this).
    var onInput: (([Float]) -> Void)?

    /// Scales TX peak amplitude before writing to the output buffer, 0...1.
    /// 1.0 leaves the core's own burst untouched (~0.5 peak).
    ///
    /// The modem tolerates hard clipping remarkably well on its own (see
    /// survives_severe_hard_clipping in core/src/tests.rs), so this exists
    /// less as a "fix the level" control and more as a way to test whether
    /// a radio's own mic-input stage is overloading before whatever
    /// processing it applies, as distinct from an adaptive compander that
    /// wouldn't care about level at all.
    var txGain: Float = 1.0

    init() {
        engine.attach(player)
    }

    /// Decides which single device the engine's I/O unit should use. Returns
    /// the shared device when input and output match (or either is the system
    /// default), otherwise creates a private aggregate spanning both and returns
    /// it. Falls back to the input device if aggregate creation fails, so a
    /// mic-only session still works.
    private func resolveDevice(inputDevice: AudioDeviceID, outputDevice: AudioDeviceID)
        -> AudioDeviceID
    {
        if inputDevice == kAudioObjectUnknown { return outputDevice }
        if outputDevice == kAudioObjectUnknown { return inputDevice }
        if inputDevice == outputDevice { return inputDevice }
        if let agg = AudioAggregate.create(input: inputDevice, output: outputDevice) {
            aggregate = agg
            return agg
        }
        return inputDevice
    }

    /// Points an engine node's underlying AUHAL at a specific device.
    /// `kAudioObjectUnknown` leaves the system default in place.
    private func setDevice(_ device: AudioDeviceID, on unit: AudioUnit?) {
        guard device != kAudioObjectUnknown, let unit else { return }
        var id = device
        AudioUnitSetProperty(
            unit,
            kAudioOutputUnitProperty_CurrentDevice,
            kAudioUnitScope_Global,
            0,
            &id,
            UInt32(MemoryLayout<AudioDeviceID>.size))
    }

    /// Starts duplex audio on the given devices: installs the mic tap
    /// (converted to 48 kHz mono) and wires the player to the output.
    func start(inputDevice: AudioDeviceID, outputDevice: AudioDeviceID) throws {
        stop()

        // AVAudioEngine drives a single hardware I/O unit, so inputNode and
        // outputNode share one AUHAL: setting a device on each would just leave
        // the last one pinned to both directions. Point that one unit at the
        // right device: the shared device when they match, otherwise a private
        // aggregate that bridges the two (needed when the output is something
        // like a USB-C dongle with no input of its own).
        let device = resolveDevice(inputDevice: inputDevice, outputDevice: outputDevice)
        setDevice(device, on: engine.inputNode.audioUnit)

        // Stereo, not mono: play() builds the L/R buffer itself according to
        // txChannel, rather than relying on AVAudioEngine's own mono-to-stereo
        // duplication behavior when mixing down to hardware output. That
        // implicit behavior is exactly the kind of unverified assumption that
        // makes a wiring/channel mismatch hard to diagnose or work around.
        engine.connect(player, to: engine.mainMixerNode, format: Self.format48kStereo)

        let inputFormat = engine.inputNode.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0 else {
            throw NSError(
                domain: "Aetr", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Input device has no valid format"])
        }

        // Convert to stereo when the device offers two or more channels, so
        // the per-frame channel choice happens here and not inside
        // AVAudioConverter's default downmix matrix.
        converterIsStereo = inputFormat.channelCount >= 2
        let target = converterIsStereo ? Self.format48kStereo : Self.format48kMono
        guard let conv = AVAudioConverter(from: inputFormat, to: target) else {
            throw NSError(
                domain: "Aetr", code: 2,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "Cannot convert this input device's format "
                        + "(\(Int(inputFormat.sampleRate)) Hz, \(inputFormat.channelCount) ch) "
                        + "to 48 kHz. Pick a different input device."
                ])
        }
        // Only relevant when the device refused 48 kHz above; a sloppy
        // resampler is not something to leave to chance in a modem path.
        conv.sampleRateConverterQuality = AVAudioQuality.max.rawValue
        converter = conv

        engine.inputNode.installTap(onBus: 0, bufferSize: 4_800, format: inputFormat) {
            [weak self] buffer, _ in
            self?.handleInput(buffer)
        }

        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleConfigurationChange),
            name: .AVAudioEngineConfigurationChange,
            object: engine)

        engine.prepare()
        try engine.start()
        running = true
    }

    /// The hardware was reconfigured (device unplugged, rate changed, or the
    /// aggregate rebuilt). AVAudioEngine stops delivering input and does not
    /// recover on its own, so say so rather than going quietly deaf.
    @objc private func handleConfigurationChange(_ note: Notification) {
        guard running else { return }
        onFault?("The audio device changed configuration; reconnect to resume receiving.")
    }

    /// Stops the engine and removes the tap. Safe to call when idle.
    func stop() {
        NotificationCenter.default.removeObserver(
            self, name: .AVAudioEngineConfigurationChange, object: engine)
        player.stop()
        engine.inputNode.removeTap(onBus: 0)
        if engine.isRunning {
            engine.stop()
        }
        converter = nil
        running = false
        if aggregate != kAudioObjectUnknown {
            AudioAggregate.destroy(aggregate)
            aggregate = kAudioObjectUnknown
        }
    }

    /// Converts one tap buffer to 48 kHz mono and forwards it to `onInput`.
    /// Stereo inputs are collapsed here via `rxChannel` rather than by the
    /// converter, so a one-legged cable keeps its full level.
    private func handleInput(_ buffer: AVAudioPCMBuffer) {
        guard let converter, buffer.frameLength > 0 else { return }
        let target = converterIsStereo ? Self.format48kStereo : Self.format48kMono
        let ratio = target.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return }

        var supplied = false
        var error: NSError?
        let status = converter.convert(to: out, error: &error) { _, outStatus in
            if supplied {
                outStatus.pointee = .noDataNow
                return nil
            }
            supplied = true
            outStatus.pointee = .haveData
            return buffer
        }
        if status == .error {
            // Previously captured and dropped, which left receive silently
            // dead. Report it once per occurrence instead.
            onFault?("Audio conversion failed: \(error?.localizedDescription ?? "unknown error")")
            return
        }
        guard out.frameLength > 0, let channels = out.floatChannelData else { return }

        let count = Int(out.frameLength)
        if converterIsStereo && out.format.channelCount >= 2 {
            let left = channels[0]
            let right = channels[1]
            let picker = rxChannel
            var mono = [Float](repeating: 0, count: count)
            for i in 0..<count {
                mono[i] = picker.pick(left: left[i], right: right[i])
            }
            onInput?(mono)
        } else {
            onInput?(Array(UnsafeBufferPointer(start: channels[0], count: count)))
        }
    }

    /// Schedules a 48 kHz burst on the output device, written onto the
    /// channel(s) selected by [txChannel]. `completion` fires (on an
    /// arbitrary thread) once the samples have been played.
    func play(_ pcm: [Float], completion: @escaping () -> Void) {
        // Temporary diagnostic logging for the "no TX audio at all" report:
        // print unconditionally so it shows up when running the .app's Mach-O
        // directly from Terminal, without needing new UI wiring to find out
        // which stage this dies at.
        let peak = pcm.reduce(Float(0)) { max($0, abs($1)) }
        print(
            "[aetr TX] play() called: \(pcm.count) samples, peak \(peak), "
                + "channel=\(txChannel.rawValue), engine.running=\(running), "
                + "engine.isRunning=\(engine.isRunning), player.isPlaying=\(player.isPlaying)")
        guard running else {
            print("[aetr TX] ABORTED: engine.running is false, nothing will play")
            completion()
            return
        }
        guard !pcm.isEmpty else {
            print("[aetr TX] ABORTED: pcm is empty")
            completion()
            return
        }
        guard
            let buffer = AVAudioPCMBuffer(
                pcmFormat: Self.format48kStereo, frameCapacity: AVAudioFrameCount(pcm.count))
        else {
            print("[aetr TX] ABORTED: could not allocate an AVAudioPCMBuffer")
            completion()
            return
        }
        buffer.frameLength = AVAudioFrameCount(pcm.count)
        let left = buffer.floatChannelData![0]
        let right = buffer.floatChannelData![1]
        let channel = txChannel
        let gain = txGain
        // Only allocate a scaled copy when actually scaling; unity gain (the
        // default) writes the core's own samples straight through.
        let scaled = gain == 1.0 ? pcm : pcm.map { $0 * gain }
        scaled.withUnsafeBufferPointer { src in
            guard let base = src.baseAddress else { return }
            switch channel {
            case .both:
                left.update(from: base, count: scaled.count)
                right.update(from: base, count: scaled.count)
            case .left:
                left.update(from: base, count: scaled.count)
                right.update(repeating: 0, count: scaled.count)
            case .right:
                left.update(repeating: 0, count: scaled.count)
                right.update(from: base, count: scaled.count)
            }
        }
        player.scheduleBuffer(buffer, at: nil, options: [], completionCallbackType: .dataPlayedBack) {
            _ in
            print("[aetr TX] scheduleBuffer completion fired (buffer actually drained)")
            completion()
        }
        if !player.isPlaying {
            print("[aetr TX] calling player.play()")
            player.play()
        }
        print(
            "[aetr TX] after scheduling: player.isPlaying=\(player.isPlaying), "
                + "engine.isRunning=\(engine.isRunning), "
                + "mixer.outputVolume=\(engine.mainMixerNode.outputVolume)")
    }
}
