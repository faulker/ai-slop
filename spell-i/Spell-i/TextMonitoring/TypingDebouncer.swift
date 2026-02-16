import Foundation

/// Debounces keystroke events to avoid running spell checks on every character.
/// Fires after the user pauses typing for a configurable interval.
final class TypingDebouncer {

    /// Called when the debounce interval elapses after the last keystroke.
    var onDebounced: (() -> Void)?

    /// Debounce interval in seconds.
    var interval: TimeInterval

    private var timer: Timer?

    init(interval: TimeInterval = Constants.defaultDebounceInterval) {
        self.interval = interval
    }

    /// Call this on every keystroke. Resets the debounce timer efficiently.
    func keystroke() {
        if let timer = timer, timer.isValid {
            timer.fireDate = Date(timeIntervalSinceNow: interval)
        } else {
            timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: false) { [weak self] _ in
                self?.onDebounced?()
            }
        }
    }

    /// Immediately fires the debounced action if a timer is pending.
    func flush() {
        guard timer?.isValid == true else { return }
        timer?.invalidate()
        onDebounced?()
    }

    /// Cancels any pending debounce without firing.
    func cancel() {
        timer?.invalidate()
        timer = nil
    }

    deinit {
        cancel()
    }
}
