import SwiftUI

/// Single-window UI: setup bar on top, message log in the middle, compose
/// bar at the bottom.
struct ContentView: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        VStack(spacing: 0) {
            SetupBar()
            Divider()
            DeviceBar()
            Divider()
            MessageLog()
            Divider()
            ComposeBar()
        }
    }
}

/// Passphrase, modem mode, voice cap (with airtime estimate), and the
/// Connect/Disconnect button. Settings lock while connected.
struct SetupBar: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                // Protocol settings only apply at connect time, so they
                // lock while a session is live.
                SecureField("Passphrase", text: $appState.passphrase)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 240)
                    .disabled(appState.connected)

                Picker("Mode", selection: $appState.mode) {
                    Text("85 (robust)").tag(ModemMode.b85)
                    Text("128").tag(ModemMode.b128)
                    Text("170 (fast)").tag(ModemMode.b170)
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 260)
                .disabled(appState.connected)

                Spacer()

                if appState.connecting {
                    ProgressView().controlSize(.small)
                }
                Button(appState.connected ? "Disconnect" : "Connect") {
                    appState.connected ? appState.disconnect() : appState.connect()
                }
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(appState.connecting)
            }

            HStack(spacing: 8) {
                Text("Voice cap:")
                TextField("30", value: $appState.voiceCapSecs, format: .number)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 50)
                    .disabled(appState.connected)
                Stepper("", value: $appState.voiceCapSecs, in: 1...300)
                    .labelsHidden()
                    .disabled(appState.connected)
                Text("s")
                Text(String(
                    format: "≈ %.0f s airtime per full clip; longer bursts risk mid-transmission loss.",
                    appState.voiceCapAirtimeSecs))
                    .font(.caption)
                    .foregroundColor(.secondary)
                Spacer()
            }

            // TX key-up delay: wait before data so the radio (Bluetooth or
            // VOX keyed) is fully transmitting when the burst starts.
            HStack(spacing: 8) {
                Text("TX delay:")
                TextField("1000", value: $appState.txDelayMs, format: .number)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 60)
                    .disabled(appState.connected)
                Stepper("", value: $appState.txDelayMs, in: 0...5000, step: 100)
                    .labelsHidden()
                    .disabled(appState.connected)
                Text("ms")
                Toggle("VOX primer tone", isOn: $appState.voxPrimer)
                    .disabled(appState.connected)
                    .help("Fills the delay with a quiet tone so VOX-keyed radios open TX during the delay")
                Toggle("Auto ARQ (VOX)", isOn: $appState.autoArq)
                    .help("Automatically request and send repairs so VOX-only radios do not need a second manual PTT")
                Text("Wait before data starts so the radio is fully keyed up. Keep VOX primer on with Auto ARQ.")
                    .font(.caption)
                    .foregroundColor(.secondary)
                Spacer()
            }

            if let error = appState.errorMessage {
                Text(error).font(.caption).foregroundColor(.red)
            }
            if let error = appState.audioError {
                Text(error).font(.caption).foregroundColor(.orange)
            }
        }
        .padding(10)
    }
}

/// Input/output pickers, digital loopback toggle, and the TX/RX badge.
struct DeviceBar: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        HStack(spacing: 12) {
            Picker("Input", selection: $appState.selectedInput) {
                ForEach(appState.inputDevices) { device in
                    Text(device.name).tag(device.id)
                }
            }
            .frame(maxWidth: 220)

            Picker("Output", selection: $appState.selectedOutput) {
                ForEach(appState.outputDevices) { device in
                    Text(device.name).tag(device.id)
                }
            }
            .frame(maxWidth: 220)

            Button {
                appState.refreshDevices()
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .help("Refresh audio devices")

            Picker("RX ch", selection: $appState.rxChannel) {
                ForEach(AudioEngine.RxChannel.allCases) { channel in
                    Text(channel.rawValue).tag(channel)
                }
            }
            .pickerStyle(.segmented)
            .frame(width: 130)
            .help(
                "Which stereo leg of the input carries receive audio. APRS and TRRS "
                    + "cables often wire only one side; Auto keeps whichever is louder.")

            Picker("TX ch", selection: $appState.txChannel) {
                ForEach(AudioEngine.TxChannel.allCases) { channel in
                    Text(channel.rawValue).tag(channel)
                }
            }
            .pickerStyle(.segmented)
            .frame(width: 130)
            .help(
                "Which channel(s) carry TX audio out to the radio's mic input. If the "
                    + "radio keys up but only static goes out, try L or R before "
                    + "re-wiring the cable.")

            HStack(spacing: 4) {
                Text("TX level").font(.caption)
                Slider(value: $appState.txGain, in: 0.1...1.0)
                    .frame(width: 100)
                Text(String(format: "%.2f", appState.txGain))
                    .font(.caption.monospacedDigit())
                    .frame(width: 32, alignment: .trailing)
            }
            .help(
                "Scales TX peak amplitude. Worth lowering if the radio's mic input is "
                    + "overloading before whatever processing it applies to the signal.")

            Toggle("Capture RX", isOn: $appState.captureRx)
                .toggleStyle(.checkbox)
                .help(
                    "Record everything the input hears to a WAV in Documents. Turn it "
                        + "off to write the file, then run: Aetr --analyze-wav <file>")

            Toggle("Digital loopback", isOn: $appState.digitalLoopback)
                .toggleStyle(.checkbox)
                .foregroundColor(appState.digitalLoopback ? .red : .primary)
                .help("Debug: pipe encoded bursts straight into the receiver, bypassing audio hardware. When ON, Mac reads its own sends and the radio/Android hear nothing.")
            if appState.digitalLoopback {
                Text("LOOPBACK ON — Mac will 'receive' its own sends; radio/Android hear nothing")
                    .font(.caption2.bold())
                    .foregroundColor(.red)
            }

            if appState.connected {
                VStack(alignment: .leading, spacing: 1) {
                    InputLevelMeter(level: appState.inputLevel)
                    // RMS as well as peak: the sync floor is an RMS threshold,
                    // and below it a quiet burst is indistinguishable from
                    // silence as far as the decoder is concerned.
                    HStack(spacing: 6) {
                        Text(
                            appState.inputRmsDbfs.isFinite
                                ? String(format: "RMS %.0f dBFS", appState.inputRmsDbfs)
                                : "RMS −∞ dBFS"
                        )
                        .foregroundColor(
                            appState.inputRmsDbfs.isFinite && appState.inputRmsDbfs < -57
                                ? .orange : .secondary)
                        if appState.droppedBuffers > 0 {
                            Text("· \(appState.droppedBuffers) buffer(s) dropped")
                                .foregroundColor(.red)
                        }
                    }
                    .font(.caption2.monospacedDigit())
                }
                .frame(maxWidth: 200)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 2) {
                StateBadge()
                if appState.connected && appState.rejectedFrames > 0 {
                    Text("\(appState.rejectedFrames) frame(s) rejected")
                        .font(.caption2)
                        .foregroundColor(.orange)
                }
                if appState.connected {
                    RxStageSummary()
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }
}

/// One-line summary of where the receiver is losing bursts.
///
/// The stages run in order, so read it left to right and stop at the first
/// count that does not carry through. Nothing shown at all means no burst has
/// reached the modem, which points at routing or level rather than decode.
struct RxStageSummary: View {
    @EnvironmentObject private var appState: AppState

    /// Nil until something has actually reached the modem.
    private var summary: String? {
        let c = appState.rxCounters
        guard c.syncCandidates > 0 || c.framesDecoded > 0 else { return nil }
        var parts = ["sync \(c.syncCandidates)"]
        if c.preambleFailed > 0 { parts.append("pre✗ \(c.preambleFailed)") }
        if c.payloadFailed > 0 { parts.append("pay✗ \(c.payloadFailed)") }
        parts.append("ok \(c.framesDecoded)")
        if appState.lastSnrDb > -99 {
            parts.append(String(format: "%.0f dB", appState.lastSnrDb))
        }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        if let summary {
            Text(summary)
                .font(.caption2.monospacedDigit())
                .foregroundColor(appState.rxCounters.framesDecoded > 0 ? .green : .orange)
                .help(
                    "Decode stages in order. Bursts that sync but fail the payload "
                        + "are arriving corrupted; nothing at all means no audio is "
                        + "reaching the modem.")
        }
    }
}

/// Live RX input peak meter. Flatline means the selected input is silent.
struct InputLevelMeter: View {
    let level: Float

    private var color: Color {
        switch level {
        case ..<0.02: return .gray
        case ..<0.7: return .green
        case ..<0.9: return .orange
        default: return .red
        }
    }

    var body: some View {
        HStack(spacing: 6) {
            Text("In")
                .font(.caption2.bold())
                .foregroundColor(.secondary)
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.secondary.opacity(0.2))
                    Capsule()
                        .fill(color)
                        .frame(width: max(2, geo.size.width * CGFloat(min(1, max(0, level)))))
                }
            }
            .frame(height: 8)
            Text(AudioLevels.dbFsLabel(level))
                .font(.caption2.monospacedDigit())
                .foregroundColor(.secondary)
                .frame(width: 64, alignment: .trailing)
        }
        .help("Incoming audio level from the selected input device")
    }
}

/// TX while a burst is playing, otherwise the core's receiver state.
struct StateBadge: View {
    @EnvironmentObject var appState: AppState

    private var label: (text: String, color: Color) {
        if appState.isTransmitting { return ("TX", .red) }
        switch appState.rxState {
        case .idle: return ("Idle", .gray)
        case .syncing: return ("Syncing", .orange)
        case .receiving: return ("Receiving", .green)
        }
    }

    var body: some View {
        Text(label.text)
            .font(.caption.bold())
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(label.color.opacity(0.2))
            .foregroundColor(label.color)
            .clipShape(Capsule())
    }
}

/// Scrolling list of sent/received messages.
struct MessageLog: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 6) {
                    ForEach(appState.messages) { message in
                        MessageRow(message: message)
                            .id(message.id)
                    }
                }
                .padding(10)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .onChange(of: appState.messages.count) { _ in
                if let last = appState.messages.last {
                    proxy.scrollTo(last.id, anchor: .bottom)
                }
            }
        }
    }
}

/// One message row: text or voice, status line, and repair actions.
struct MessageRow: View {
    @EnvironmentObject var appState: AppState
    let message: ChatMessage

    /// Short status string under the message content.
    private var statusText: String {
        switch message.status {
        case .sending: return "Encoding…"
        case .sent: return "Sent"
        case let .receiving(received, total): return "Receiving \(received)/\(total)"
        case .complete: return "Received"
        case let .failed(reason): return "Failed: \(reason)"
        }
    }

    private var isFailed: Bool {
        if case .failed = message.status { return true }
        return false
    }

    var body: some View {
        HStack {
            if message.outgoing { Spacer(minLength: 60) }

            VStack(alignment: .leading, spacing: 4) {
                if case let .receiving(received, total) = message.status {
                    ProgressView(value: Double(received), total: Double(max(1, total)))
                        .frame(maxWidth: 160)
                }

                if message.isVoice && !message.outgoing {
                    HStack(spacing: 8) {
                        Button {
                            appState.playVoice(message)
                        } label: {
                            Label("Play", systemImage: "play.circle.fill")
                        }
                        .disabled(message.pcm.isEmpty)
                        Text(String(format: "%.1f s", Double(message.pcm.count) / 48_000.0))
                            .font(.caption)
                            .foregroundColor(.secondary)
                        if !message.missingSpans.isEmpty {
                            Label(
                                "\(message.missingSpans.count) missing span(s)",
                                systemImage: "exclamationmark.triangle")
                                .font(.caption)
                                .foregroundColor(.orange)
                        }
                    }
                } else if !message.text.isEmpty {
                    Text(message.text)
                        .textSelection(.enabled)
                }

                HStack(spacing: 8) {
                    Text(statusText)
                        .font(.caption)
                        .foregroundColor(isFailed ? .red : .secondary)

                    // Text messages sent or received in the clear are badged so
                    // it is obvious they had no encryption.
                    if !message.isVoice && !message.encrypted {
                        Label("CLEAR", systemImage: "lock.open")
                            .font(.caption2.bold())
                            .foregroundColor(.orange)
                    }

                    if message.canRequestRepair {
                        Button("Request repair") {
                            appState.requestRepair(for: message)
                        }
                        .font(.caption)
                    }

                    if message.repairPcm != nil {
                        Button("Send repair") {
                            appState.sendRepair(for: message)
                        }
                        .font(.caption)
                    }
                }
            }
            .padding(8)
            .background(
                (message.outgoing ? Color.accentColor.opacity(0.15) : Color.gray.opacity(0.12)))
            .clipShape(RoundedRectangle(cornerRadius: 8))

            if !message.outgoing { Spacer(minLength: 60) }
        }
    }
}

/// Text field + Send, and the hold-to-record mic button.
struct ComposeBar: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        HStack(spacing: 10) {
            // Encrypt-on-send toggle. Disabled (forced clear) with no passphrase.
            Button {
                appState.sendEncrypted.toggle()
            } label: {
                Image(systemName: appState.sendEncrypted ? "lock.fill" : "lock.open")
                    .foregroundColor(appState.sendEncrypted ? .accentColor : .orange)
            }
            .buttonStyle(.borderless)
            .disabled(!appState.connected || appState.passphrase.isEmpty)
            .help(appState.sendEncrypted
                ? "Sending encrypted — click to send in the clear"
                : "Sending in the clear (unencrypted) — click to encrypt")

            TextField("Message", text: $appState.composeText)
                .textFieldStyle(.roundedBorder)
                .onSubmit { appState.sendText() }

            Button("Send") {
                appState.sendText()
            }
            .disabled(!appState.connected || appState.composeText.isEmpty)

            // Hold to record, release to send.
            Image(systemName: appState.isRecording ? "mic.fill" : "mic")
                .font(.title2)
                .foregroundColor(appState.isRecording ? .red : (appState.connected ? .accentColor : .gray))
                .frame(width: 36, height: 30)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { _ in
                            if !appState.isRecording { appState.startRecording() }
                        }
                        .onEnded { _ in
                            appState.stopRecordingAndSend()
                        }
                )
                .help("Hold to record a voice clip; release to send")
        }
        .padding(10)
    }
}
