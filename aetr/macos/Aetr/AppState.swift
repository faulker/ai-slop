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
    /// B85 is the reliable default over APRS cables / Bluetooth.
    @Published var mode: ModemMode = .b85
    @Published var voiceCapSecs = 30
    /// Wait before data starts on each transmission so the radio finishes
    /// keying up (Bluetooth radios often need ~1 s).
    @Published var txDelayMs = 1000
    /// Fill the TX delay with a quiet tone so VOX-keyed radios open the
    /// transmitter during the delay instead of clipping the data.
    @Published var voxPrimer = false
    /// Auto request/answer ARQ repairs so VOX-only radios do not need a
    /// second manual PTT for error recovery.
    @Published var autoArq = true
    /// Send text encrypted (the default). Flip off to transmit in the clear
    /// on services where encryption isn't allowed; receivers read it without
    /// the passphrase. Forced off when connected with an empty passphrase.
    @Published var sendEncrypted = true
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
    /// Modem frames that synced but failed auth/checksum (wrong passphrase,
    /// corruption). Shown under the RX badge when non-zero.
    @Published var rejectedFrames: UInt64 = 0
    /// Where the receiver is losing bursts, stage by stage. The stages run in
    /// order, so the first non-zero counter with a zero successor is the point
    /// of failure. All zero means no burst energy is arriving at all, which is
    /// a routing or level problem rather than a decode one.
    @Published var rxCounters = RxCounters(
        syncCandidates: 0, preambleFailed: 0, payloadFailed: 0, framesDecoded: 0)
    /// Constellation SNR in dB from the last demodulated symbol (-99 = none).
    @Published var lastSnrDb: Float = -99
    /// Smoothed input peak 0...1 for the RX level meter.
    @Published var inputLevel: Float = 0
    /// Input RMS in dBFS. Peak alone cannot tell you whether the signal is
    /// above the receiver's sync floor, which sits near -57 dBFS RMS; below it
    /// nothing decodes and it looks exactly like a dead audio route.
    @Published var inputRmsDbfs: Float = -.infinity
    /// Tap buffers dropped because the decode queue was still busy. Any
    /// non-zero value means samples never reached the modem, which breaks the
    /// differential demodulation for that whole burst.
    @Published var droppedBuffers: UInt64 = 0
    /// Which stereo leg of the input carries the radio audio.
    @Published var rxChannel: AudioEngine.RxChannel = .auto {
        didSet { audio.rxChannel = rxChannel }
    }
    /// Which channel(s) TX audio is written onto going out to the radio.
    /// Some cables only carry the mic-in line on one leg; if the radio keys
    /// up but only static goes out, try L or R here before re-wiring anything.
    @Published var txChannel: AudioEngine.TxChannel = .both {
        didSet { audio.txChannel = txChannel }
    }
    /// TX peak scale, 0.1...1.0. Worth lowering if a radio's mic input stage
    /// is overloading before whatever processing it applies; the modem
    /// itself tolerates hard clipping well, so this is a hardware-overload
    /// probe more than a "fix the level" control.
    @Published var txGain: Float = 1.0 {
        didSet { audio.txGain = txGain }
    }
    /// When on, everything the microphone hears is written to a WAV so it can
    /// be replayed through `--analyze-wav` offline.
    @Published var captureRx = false { didSet { captureRxChanged() } }
    /// Path of the capture in progress, shown so it can be found afterwards.
    @Published var captureRxPath: String?

    private let audio = AudioEngine()
    /// Serial queue the modem decode runs on. The tap callback must not do it:
    /// a full COFDM decode (Hilbert, Schmidl-Cox, FFTs, BCH ordered-statistics
    /// decoding, CA-SCL polar list decoding) inside an audio callback can
    /// overrun the buffer deadline, and a dropped buffer looks exactly like
    /// "syncs but never decodes".
    private let decodeQueue = DispatchQueue(label: "me.faulk.aetr.decode", qos: .userInitiated)
    /// Cap on queued buffers. Past this we drop and count, rather than letting
    /// the backlog grow without bound. 24 buffers is about 2.4 s of audio,
    /// comfortably longer than the 1.26 s burst the decoder works in.
    private static let maxPendingDecodes = 24
    /// Eats spurious volume/media keys from flaky APRS headset cables.
    private let mediaKeys = MediaKeyFilter()
    private var pollTimer: Timer?

    /// Guards state shared with the audio thread: the session reference and
    /// the hold-to-record capture buffer.
    private let lock = NSLock()
    private var session: AetrSession?
    private var recordBuffer: [Float] = []
    private var recordingActive = false
    private var recordCapSamples = 0
    /// Capture-thread envelopes; published to the UI on the poll timer.
    private var inputPeakRaw: Float = 0
    private var inputRmsRaw: Float = 0
    /// Tap buffers handed to [decodeQueue] but not yet decoded.
    private var pendingDecodes = 0
    /// Buffers dropped because [pendingDecodes] hit the cap.
    private var droppedBufferCount: UInt64 = 0
    /// Accumulated capture samples. Only ever touched on [decodeQueue].
    private var captureBuffer: [Float] = []
    /// Lock-guarded mirror of `captureRx`. The audio thread must not read the
    /// @Published property directly.
    private var captureActive = false
    /// When true, mic samples update the level meter but are not decoded.
    /// Stops cable TX→RX crosstalk from looking like a real receive on Mac.
    private var suppressRx = false
    /// messageId → (last progress Date, received, total).
    private var progressWatch: [UInt64: (Date, UInt32, UInt32)] = [:]
    private var autoRepairRequestedIds: Set<UInt64> = []
    private static let autoRepairStallSecs: TimeInterval = 3.0
    private static let autoRepairResponseDelaySecs: TimeInterval = 1.2

    init() {
        refreshDevices()
        audio.onInput = { [weak self] samples in
            self?.handleInputSamples(samples)
        }
        audio.onFault = { [weak self] message in
            DispatchQueue.main.async { self?.audioError = message }
        }
    }

    /// Starts a fresh capture, or writes and closes the one in progress.
    ///
    /// The resulting WAV is the whole point of the diagnostic: replay it with
    /// `Aetr --analyze-wav <file>` to see the level, the in-band energy, and
    /// which decode stage the burst died at, without needing the radio.
    private func captureRxChanged() {
        lock.withLock { captureActive = captureRx }
        if captureRx {
            decodeQueue.async { [weak self] in self?.captureBuffer.removeAll() }
            let stamp = ISO8601DateFormatter().string(from: Date())
                .replacingOccurrences(of: ":", with: "-")
            let dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            captureRxPath = dir.appendingPathComponent("aetr-rx-\(stamp).wav").path
            return
        }
        guard let path = captureRxPath else { return }
        decodeQueue.async { [weak self] in
            guard let self else { return }
            let pcm = self.captureBuffer
            self.captureBuffer.removeAll()
            guard !pcm.isEmpty else { return }
            do {
                try writeWav48k(path: path, pcm: pcm)
            } catch {
                DispatchQueue.main.async {
                    self.errorMessage = "Could not write the capture: \(error.localizedDescription)"
                }
            }
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
        // An empty passphrase is allowed: it makes a plaintext-only session,
        // so force clear sending (encryption needs a key).
        if passphrase.isEmpty {
            sendEncrypted = false
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
                    if !self.mediaKeys.start() {
                        // Local-window-only protection. A phantom headset key
                        // while backgrounded reaches macOS unfiltered and can
                        // open Music. Actionable rather than a silent gap.
                        self.errorMessage =
                            "Input Monitoring isn't granted, so cable noise can "
                            + "trigger phantom media keys (e.g. opening Music) while "
                            + "Aetr is in the background. Enable Aetr under System "
                            + "Settings > Privacy & Security > Input Monitoring, "
                            + "then reconnect."
                    }
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
        mediaKeys.stop()
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
        rejectedFrames = 0
        rxCounters = RxCounters(
            syncCandidates: 0, preambleFailed: 0, payloadFailed: 0, framesDecoded: 0)
        lastSnrDb = -99
        inputLevel = 0
        inputRmsDbfs = -.infinity
        droppedBuffers = 0
        // Flush any capture in progress before the buffer is discarded.
        if captureRx { captureRx = false }
        lock.withLock {
            inputPeakRaw = 0
            inputRmsRaw = 0
            droppedBufferCount = 0
        }
        progressWatch.removeAll()
        autoRepairRequestedIds.removeAll()
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
        rejectedFrames = session.rejectedFrames()
        rxCounters = session.rxCounters()
        lastSnrDb = session.lastSnrDb()
        let (peak, rms, dropped) = lock.withLock {
            (inputPeakRaw, inputRmsRaw, droppedBufferCount)
        }
        inputLevel = peak
        inputRmsDbfs = AudioLevels.dbFs(rms)
        droppedBuffers = dropped
        for event in session.pollEvents() {
            handle(event)
        }
        if autoArq { maybeAutoRequestRepair() }
    }

    /// One-shot auto repair request after progress stalls (Receiving is expected).
    private func maybeAutoRequestRepair() {
        // Local TX (or half-duplex suppress) must not fire a repair; stalled
        // receives sit in .receiving so do not require .idle.
        let muted = lock.withLock { suppressRx }
        guard !isTransmitting, !muted else { return }
        let now = Date()
        guard let (id, _) = progressWatch.first(where: { id, watch in
            let (at, received, total) = watch
            return !autoRepairRequestedIds.contains(id)
                && received < total
                && now.timeIntervalSince(at) >= Self.autoRepairStallSecs
        }) else { return }
        autoRepairRequestedIds.insert(id)
        if let message = messages.first(where: { $0.messageId == id && !$0.outgoing }) {
            requestRepair(for: message)
        } else {
            // Progress row may only exist as .receiving status.
            guard let session = lock.withLock({ session }) else { return }
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                do {
                    let pcm = try session.requestRepair(messageId: id)
                    DispatchQueue.main.async { self?.transmit(pcm) {} }
                } catch {
                    DispatchQueue.main.async {
                        self?.errorMessage = "Auto repair request failed: \(error.localizedDescription)"
                    }
                }
            }
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
            progressWatch[messageId] = (Date(), received, total)
            if let i = incomingIndex(for: messageId) {
                messages[i].isVoice = isVoice
                messages[i].status = .receiving(received: received, total: total)
            } else {
                messages.append(
                    ChatMessage(
                        messageId: messageId, outgoing: false, isVoice: isVoice,
                        status: .receiving(received: received, total: total)))
            }

        case let .text(messageId, text, encrypted):
            clearProgressWatch(messageId)
            if let i = incomingIndex(for: messageId) {
                messages[i].text = text
                messages[i].isVoice = false
                messages[i].encrypted = encrypted
                messages[i].status = .complete
            } else {
                messages.append(
                    ChatMessage(
                        messageId: messageId, outgoing: false, isVoice: false,
                        encrypted: encrypted, text: text, status: .complete))
            }

        case let .voice(messageId, pcm48k, missingSpans):
            clearProgressWatch(messageId)
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
            clearProgressWatch(messageId)
            if let i = incomingIndex(for: messageId) {
                messages[i].status = .failed(reason)
            } else {
                messages.append(
                    ChatMessage(
                        messageId: messageId, outgoing: false, isVoice: false,
                        status: .failed(reason)))
            }

        case let .repairRequested(messageId, pcmResponse):
            let row = ChatMessage(
                messageId: messageId, outgoing: true, isVoice: false,
                text: "Peer requested repair of message \(String(format: "%016llx", messageId))",
                repairPcm: pcmResponse, status: .sent)
            messages.append(row)
            if autoArq {
                // Delay so the peer's request finishes and VOX can re-key.
                let rowId = row.id
                DispatchQueue.main.asyncAfter(deadline: .now() + Self.autoRepairResponseDelaySecs) {
                    [weak self] in
                    guard let self, self.autoArq,
                        let i = self.messages.firstIndex(where: { $0.id == rowId }),
                        let pcm = self.messages[i].repairPcm
                    else { return }
                    self.transmit(pcm) {
                        guard let j = self.messages.firstIndex(where: { $0.id == rowId }) else {
                            return
                        }
                        self.messages[j].repairPcm = nil
                        self.messages[j].text =
                            "Sent repair for message \(String(format: "%016llx", messageId))"
                    }
                }
            }
        }
    }

    /// Drops stall tracking once a message finalizes.
    private func clearProgressWatch(_ messageId: UInt64) {
        progressWatch.removeValue(forKey: messageId)
        autoRepairRequestedIds.remove(messageId)
    }

    // MARK: - Sending

    /// Encodes the compose text off-main and transmits the burst.
    func sendText() {
        let text = composeText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let session = lock.withLock({ session }) else { return }
        composeText = ""
        let encrypted = sendEncrypted
        let message = ChatMessage(
            outgoing: true, isVoice: false, encrypted: encrypted, text: text, status: .sending)
        messages.append(message)
        encodeAndTransmit(rowId: message.id) {
            try session.encodeText(text: text, encrypted: encrypted)
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
    /// RX is suppressed during TX so cable crosstalk cannot show up as a
    /// "received" row on this same Mac (that looked like success when OTA failed).
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
        lock.withLock { suppressRx = true }
        audio.play(pcm) { [weak self] in
            // Brief hang so the cable/squelch tail is not decoded as a peer.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                guard let self else {
                    completion()
                    return
                }
                // Modem-only flush: clear stuck TX crosstalk sync without wiping
                // in-flight reassembly or undrained Text/Progress events.
                self.lock.withLock {
                    self.session?.flushRxModem()
                    self.suppressRx = false
                }
                self.isTransmitting = false
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
        let peak = AudioLevels.peakAbs(samples)
        let rms = AudioLevels.rms(samples)
        let (currentSession, hitCap, muted, accepted, capturing) =
            lock.withLock { () -> (AetrSession?, Bool, Bool, Bool, Bool) in
                inputPeakRaw = AudioLevels.decay(current: inputPeakRaw, samplePeak: peak)
                inputRmsRaw = AudioLevels.decay(current: inputRmsRaw, samplePeak: rms)
                var capped = false
                if recordingActive && !suppressRx {
                    recordBuffer.append(contentsOf: samples)
                    if recordBuffer.count >= recordCapSamples {
                        recordBuffer = Array(recordBuffer.prefix(recordCapSamples))
                        capped = true
                    }
                }
                // Admission control for the decode queue. Dropping here and
                // saying so beats silently falling behind, because a gap in
                // the sample stream breaks the differential demodulation for
                // the whole burst it lands in.
                var admit = false
                if pendingDecodes < Self.maxPendingDecodes {
                    pendingDecodes += 1
                    admit = true
                } else {
                    droppedBufferCount &+= 1
                }
                return (session, capped, suppressRx, admit, captureActive)
            }

        if accepted {
            decodeQueue.async { [weak self] in
                guard let self else { return }
                // Capture everything the mic hears, including while muted, so
                // the recording matches what the radio actually delivered.
                if capturing {
                    self.captureBuffer.append(contentsOf: samples)
                }
                if !muted {
                    currentSession?.pushRx(pcm48k: samples)
                }
                self.lock.withLock { self.pendingDecodes -= 1 }
            }
        }

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
