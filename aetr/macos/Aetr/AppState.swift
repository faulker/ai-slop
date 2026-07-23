import AVFoundation
import Combine
import CoreAudio
import Foundation

/// Central app state: owns the Rust session, the audio engine, the message
/// log, and the ~10 Hz event poll. UI state is only mutated on the main
/// thread; the audio callback touches the session and record buffer through
/// a lock because it runs on the audio thread.
final class AppState: ObservableObject {
    // Setup
    @Published var passphrase = ""
    @Published var mode: ModemMode = .b170
    @Published var voiceCapSecs = 30
    /// Wait before data starts on each transmission so the radio finishes
    /// keying up (Bluetooth radios often need ~1 s).
    @Published var txDelayMs = 1000
    /// Fill the TX delay with a quiet tone so VOX-keyed radios open the
    /// transmitter during the delay instead of clipping the data.
    @Published var voxPrimer = false
    @Published var connected = false
    @Published var connecting = false
    @Published var errorMessage: String?
    @Published var audioError: String?

    // Devices / debug
    @Published var inputDevices: [AudioDevice] = []
    @Published var outputDevices: [AudioDevice] = []
    @Published var selectedInput: AudioDeviceID = kAudioObjectUnknown {
        didSet { restartAudioIfConnected() }
    }
    @Published var selectedOutput: AudioDeviceID = kAudioObjectUnknown {
        didSet { restartAudioIfConnected() }
    }
    /// Debug: route encoded bursts straight into push_rx, bypassing audio.
    @Published var digitalLoopback = false

    // Log / status
    @Published var messages: [ChatMessage] = []
    @Published var composeText = ""
    @Published var isRecording = false
    @Published var isTransmitting = false
    @Published var rxState: RxState = .idle

    private let audio = AudioEngine()
    private var pollTimer: Timer?

    /// Guards state shared with the audio thread: the session reference and
    /// the hold-to-record capture buffer.
    private let lock = NSLock()
    private var session: AetrSession?
    private var recordBuffer: [Float] = []
    private var recordingActive = false
    private var recordCapSamples = 0

    init() {
        refreshDevices()
        audio.onInput = { [weak self] samples in
            self?.handleInputSamples(samples)
        }
    }

    /// Re-enumerates CoreAudio devices, keeping selections when possible.
    func refreshDevices() {
        inputDevices = AudioDevices.inputDevices()
        outputDevices = AudioDevices.outputDevices()
        if !inputDevices.contains(where: { $0.id == selectedInput }) {
            selectedInput = AudioDevices.defaultDevice(input: true)
        }
        if !outputDevices.contains(where: { $0.id == selectedOutput }) {
            selectedOutput = AudioDevices.defaultDevice(input: false)
        }
    }

    // MARK: - Session lifecycle

    /// Creates the session off-main (Argon2id blocks ~100 ms), then starts
    /// audio and the event poll.
    func connect() {
        guard !passphrase.isEmpty else {
            errorMessage = "Enter a passphrase first"
            return
        }
        connecting = true
        errorMessage = nil
        let config = SessionConfig(
            passphrase: passphrase, mode: mode, voiceCapSecs: UInt32(max(1, voiceCapSecs)),
            txDelayMs: UInt32(max(0, txDelayMs)), voxPrimer: voxPrimer)
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let newSession = try AetrSession(config: config)
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.lock.withLock { self.session = newSession }
                    self.connected = true
                    self.connecting = false
                    self.startAudio()
                    self.startPolling()
                }
            } catch {
                DispatchQueue.main.async {
                    self?.errorMessage = "Connect failed: \(error.localizedDescription)"
                    self?.connecting = false
                }
            }
        }
    }

    /// Tears down the session, audio, and poll timer.
    func disconnect() {
        pollTimer?.invalidate()
        pollTimer = nil
        audio.stop()
        lock.withLock {
            session = nil
            recordingActive = false
            recordBuffer = []
        }
        connected = false
        isRecording = false
        isTransmitting = false
        rxState = .idle
        audioError = nil
    }

    /// Requests mic permission and starts duplex audio. Failure is not
    /// fatal: the session stays up so digital loopback still works.
    private func startAudio() {
        AVCaptureDevice.requestAccess(for: .audio) { [weak self] granted in
            DispatchQueue.main.async {
                guard let self, self.connected else { return }
                guard granted else {
                    self.audioError = "Microphone access denied; only digital loopback will work"
                    return
                }
                do {
                    try self.audio.start(
                        inputDevice: self.selectedInput, outputDevice: self.selectedOutput)
                    self.audioError = nil
                } catch {
                    self.audioError = "Audio start failed: \(error.localizedDescription)"
                }
            }
        }
    }

    /// Applies a device picker change by restarting the engine.
    private func restartAudioIfConnected() {
        guard connected else { return }
        audio.stop()
        startAudio()
    }

    /// Drives poll_events and the RX badge at ~10 Hz.
    private func startPolling() {
        pollTimer?.invalidate()
        pollTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            self?.poll()
        }
    }

    /// Drains queued core events into the message log.
    private func poll() {
        guard let session = lock.withLock({ session }) else { return }
        rxState = session.rxState()
        for event in session.pollEvents() {
            handle(event)
        }
    }

    // MARK: - RxEvent handling

    /// Finds the log row for an in-flight incoming message.
    private func incomingIndex(for messageId: UInt64) -> Int? {
        messages.lastIndex { !$0.outgoing && $0.messageId == messageId }
    }

    /// Applies one core event to the message log.
    private func handle(_ event: RxEvent) {
        switch event {
        case let .progress(messageId, received, total, isVoice):
            if let i = incomingIndex(for: messageId) {
                messages[i].isVoice = isVoice
                messages[i].status = .receiving(received: received, total: total)
            } else {
                messages.append(
                    ChatMessage(
                        messageId: messageId, outgoing: false, isVoice: isVoice,
                        status: .receiving(received: received, total: total)))
            }

        case let .text(messageId, text):
            if let i = incomingIndex(for: messageId) {
                messages[i].text = text
                messages[i].isVoice = false
                messages[i].status = .complete
            } else {
                messages.append(
                    ChatMessage(
                        messageId: messageId, outgoing: false, isVoice: false, text: text,
                        status: .complete))
            }

        case let .voice(messageId, pcm48k, missingSpans):
            if let i = incomingIndex(for: messageId) {
                messages[i].isVoice = true
                messages[i].pcm = pcm48k
                messages[i].missingSpans = missingSpans
                messages[i].status = .complete
            } else {
                messages.append(
                    ChatMessage(
                        messageId: messageId, outgoing: false, isVoice: true, pcm: pcm48k,
                        missingSpans: missingSpans, status: .complete))
            }

        case let .failed(messageId, reason):
            if let i = incomingIndex(for: messageId) {
                messages[i].status = .failed(reason)
            } else {
                messages.append(
                    ChatMessage(
                        messageId: messageId, outgoing: false, isVoice: false,
                        status: .failed(reason)))
            }

        case let .repairRequested(messageId, pcmResponse):
            messages.append(
                ChatMessage(
                    messageId: messageId, outgoing: true, isVoice: false,
                    text: "Peer requested repair of message \(String(format: "%016llx", messageId))",
                    repairPcm: pcmResponse, status: .sent))
        }
    }

    // MARK: - Sending

    /// Encodes the compose text off-main and transmits the burst.
    func sendText() {
        let text = composeText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let session = lock.withLock({ session }) else { return }
        composeText = ""
        let message = ChatMessage(outgoing: true, isVoice: false, text: text, status: .sending)
        messages.append(message)
        encodeAndTransmit(rowId: message.id) {
            try session.encodeText(text: text)
        }
    }

    /// Encodes a recorded voice clip off-main and transmits the burst.
    private func sendVoice(_ pcm: [Float]) {
        guard !pcm.isEmpty, let session = lock.withLock({ session }) else { return }
        let secs = Double(pcm.count) / 48_000.0
        let message = ChatMessage(
            outgoing: true, isVoice: true,
            text: String(format: "Voice clip (%.1f s)", secs), status: .sending)
        messages.append(message)
        encodeAndTransmit(rowId: message.id) {
            try session.encodeVoice(pcm48k: pcm)
        }
    }

    /// Runs an encode closure on a background queue, then transmits the
    /// resulting burst and updates the row's status.
    private func encodeAndTransmit(rowId: UUID, encode: @escaping () throws -> [Float]) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let pcm = try encode()
                DispatchQueue.main.async {
                    self?.transmit(pcm) {
                        self?.setStatus(rowId: rowId, .sent)
                    }
                }
            } catch {
                DispatchQueue.main.async {
                    self?.setStatus(rowId: rowId, .failed(error.localizedDescription))
                }
            }
        }
    }

    /// Updates one log row's status by row id.
    private func setStatus(rowId: UUID, _ status: ChatMessage.Status) {
        if let i = messages.firstIndex(where: { $0.id == rowId }) {
            messages[i].status = status
        }
    }

    /// Sends a burst out: straight into push_rx in digital loopback,
    /// otherwise played on the selected output device with a TX badge.
    private func transmit(_ pcm: [Float], completion: @escaping () -> Void) {
        if digitalLoopback {
            lock.withLock { session }?.pushRx(pcm48k: pcm)
            completion()
            return
        }
        guard audio.running else {
            errorMessage = "Audio is not running; enable digital loopback or fix the audio device"
            completion()
            return
        }
        isTransmitting = true
        audio.play(pcm) { [weak self] in
            DispatchQueue.main.async {
                self?.isTransmitting = false
                completion()
            }
        }
    }

    // MARK: - Repair (ARQ)

    /// Receiver side: builds and transmits a repair request for an
    /// incomplete incoming message.
    func requestRepair(for message: ChatMessage) {
        guard let messageId = message.messageId,
            let session = lock.withLock({ session })
        else { return }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let pcm = try session.requestRepair(messageId: messageId)
                DispatchQueue.main.async {
                    self?.transmit(pcm) {}
                }
            } catch {
                DispatchQueue.main.async {
                    self?.errorMessage = "Repair request failed: \(error.localizedDescription)"
                }
            }
        }
    }

    /// Sender side: transmits the cached repair burst the core prepared in
    /// response to a peer's request.
    func sendRepair(for message: ChatMessage) {
        guard let pcm = message.repairPcm else { return }
        transmit(pcm) {}
    }

    // MARK: - Voice recording

    /// Begins capturing mic samples (the same tap that feeds push_rx).
    func startRecording() {
        guard connected, audio.running, !isRecording else { return }
        lock.withLock {
            recordBuffer = []
            recordCapSamples = max(1, voiceCapSecs) * 48_000
            recordingActive = true
        }
        isRecording = true
    }

    /// Stops capturing and sends whatever was recorded.
    func stopRecordingAndSend() {
        guard isRecording else { return }
        let pcm = lock.withLock { () -> [Float] in
            recordingActive = false
            let captured = recordBuffer
            recordBuffer = []
            return captured
        }
        isRecording = false
        // Sub-0.2 s presses are treated as accidental taps.
        if pcm.count >= 9_600 {
            sendVoice(pcm)
        }
    }

    /// Audio-thread sink: feeds the session's receiver and, while the mic
    /// button is held, the record buffer (auto-stopping at the cap).
    private func handleInputSamples(_ samples: [Float]) {
        let (currentSession, hitCap) = lock.withLock { () -> (AetrSession?, Bool) in
            var capped = false
            if recordingActive {
                recordBuffer.append(contentsOf: samples)
                if recordBuffer.count >= recordCapSamples {
                    recordBuffer = Array(recordBuffer.prefix(recordCapSamples))
                    capped = true
                }
            }
            return (session, capped)
        }
        currentSession?.pushRx(pcm48k: samples)
        if hitCap {
            DispatchQueue.main.async { [weak self] in
                self?.stopRecordingAndSend()
            }
        }
    }

    // MARK: - Playback / estimates

    /// Plays a received voice clip on the selected output device.
    func playVoice(_ message: ChatMessage) {
        guard audio.running else {
            errorMessage = "Audio is not running; cannot play the clip"
            return
        }
        audio.play(message.pcm) {}
    }

    /// Estimated airtime for a full-length voice clip at the current cap,
    /// modem mode, and TX key-up delay.
    var voiceCapAirtimeSecs: Double {
        estimateAirtimeSecs(
            mode: mode, kind: .voice, len: UInt64(max(1, voiceCapSecs)) * 48_000,
            txDelayMs: UInt32(max(0, txDelayMs)))
    }
}
