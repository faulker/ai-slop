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

    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private var converter: AVAudioConverter?
    private(set) var running = false

    /// A private aggregate device we created to bridge distinct input/output
    /// devices, destroyed on the next `stop()`. `kAudioObjectUnknown` when the
    /// session runs on a single device and no aggregate was needed.
    private var aggregate: AudioDeviceID = kAudioObjectUnknown

    /// Receives 48 kHz mono samples from the mic tap. Audio-thread context;
    /// keep it cheap (push_rx is designed for this).
    var onInput: (([Float]) -> Void)?

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

        engine.connect(player, to: engine.mainMixerNode, format: Self.format48kMono)

        let inputFormat = engine.inputNode.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0 else {
            throw NSError(
                domain: "Aetr", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Input device has no valid format"])
        }
        converter = AVAudioConverter(from: inputFormat, to: Self.format48kMono)
        engine.inputNode.installTap(onBus: 0, bufferSize: 4_800, format: inputFormat) {
            [weak self] buffer, _ in
            self?.handleInput(buffer)
        }

        engine.prepare()
        try engine.start()
        running = true
    }

    /// Stops the engine and removes the tap. Safe to call when idle.
    func stop() {
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
    private func handleInput(_ buffer: AVAudioPCMBuffer) {
        guard let converter, buffer.frameLength > 0 else { return }
        let ratio = Self.format48kMono.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: Self.format48kMono, frameCapacity: capacity)
        else { return }

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
        guard status != .error, out.frameLength > 0, let channel = out.floatChannelData else {
            return
        }
        onInput?(Array(UnsafeBufferPointer(start: channel[0], count: Int(out.frameLength))))
    }

    /// Schedules a 48 kHz mono burst on the output device. `completion`
    /// fires (on an arbitrary thread) once the samples have been played.
    func play(_ pcm: [Float], completion: @escaping () -> Void) {
        guard running, !pcm.isEmpty,
            let buffer = AVAudioPCMBuffer(
                pcmFormat: Self.format48kMono, frameCapacity: AVAudioFrameCount(pcm.count))
        else {
            completion()
            return
        }
        buffer.frameLength = AVAudioFrameCount(pcm.count)
        pcm.withUnsafeBufferPointer { src in
            buffer.floatChannelData![0].update(from: src.baseAddress!, count: pcm.count)
        }
        player.scheduleBuffer(buffer, at: nil, options: [], completionCallbackType: .dataPlayedBack) {
            _ in
            completion()
        }
        if !player.isPlaying {
            player.play()
        }
    }
}
