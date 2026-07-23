import Foundation

/// One row in the message log: sent or received, text or voice, plus the
/// special "peer asked us to repair" row carrying a ready-to-send burst.
struct ChatMessage: Identifiable {
    /// Delivery/reassembly state driven by RxEvents (incoming) or the
    /// encode-and-transmit pipeline (outgoing).
    enum Status: Equatable {
        case sending
        case sent
        case receiving(received: UInt32, total: UInt32)
        case complete
        case failed(String)
    }

    let id = UUID()
    /// Protocol message id; known for incoming messages and repair
    /// requests, nil for outgoing (the core doesn't expose it on encode).
    var messageId: UInt64?
    let outgoing: Bool
    var isVoice: Bool
    var text: String = ""
    /// Received voice clip, 48 kHz mono, ready to play.
    var pcm: [Float] = []
    /// Span indices the core filled with silence in `pcm`.
    var missingSpans: [UInt32] = []
    /// Set on RepairRequested rows: the burst to transmit when the user
    /// hits "Send repair".
    var repairPcm: [Float]?
    var status: Status
    let timestamp = Date()

    /// Whether the row should offer a "Request repair" action: an incoming
    /// message that failed/timed out or a voice clip with silence gaps.
    var canRequestRepair: Bool {
        guard !outgoing, messageId != nil, repairPcm == nil else { return false }
        if case .failed = status { return true }
        return isVoice && !missingSpans.isEmpty
    }
}
