import os

/// Lightweight wrapper around os.Logger for structured logging.
struct Logger {

    private let logger: os.Logger

    init(subsystem: String = Constants.subsystem, category: String) {
        self.logger = os.Logger(subsystem: subsystem, category: category)
    }

    func debug(_ message: String) {
        #if DEBUG
        logger.debug("\(message, privacy: .public)")
        #else
        logger.debug("\(message, privacy: .private)")
        #endif
    }

    func info(_ message: String) {
        logger.info("\(message, privacy: .public)")
    }

    func warning(_ message: String) {
        logger.warning("\(message, privacy: .public)")
    }

    func error(_ message: String) {
        logger.error("\(message, privacy: .public)")
    }
}
