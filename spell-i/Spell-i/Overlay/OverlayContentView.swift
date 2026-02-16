import AppKit

protocol OverlayContentViewDelegate: AnyObject {
    func overlayContentView(_ view: OverlayContentView, didClickUnderlineAt index: Int, screenRect: CGRect)
}

/// NSView that wraps SquigglyUnderlineView and intercepts clicks on underline rects.
final class OverlayContentView: NSView {

    weak var delegate: OverlayContentViewDelegate?

    let squigglyView = SquigglyUnderlineView()

    override var isFlipped: Bool { true }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        squigglyView.frame = bounds
        squigglyView.autoresizingMask = [.width, .height]
        addSubview(squigglyView)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // MARK: - Hit Testing

    override func hitTest(_ point: NSPoint) -> NSView? {
        if squigglyView.underline(at: point) != nil {
            return self
        }
        return nil
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        guard let underline = squigglyView.underline(at: point) else { return }
        delegate?.overlayContentView(self, didClickUnderlineAt: underline.resultIndex, screenRect: underline.rect)
    }
}
