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
                Text("Wait before data starts so the radio is fully keyed up.")
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

            Toggle("Digital loopback", isOn: $appState.digitalLoopback)
                .toggleStyle(.checkbox)
                .help("Debug: pipe encoded bursts straight into the receiver, bypassing audio hardware")

            Spacer()

            StateBadge()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
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
