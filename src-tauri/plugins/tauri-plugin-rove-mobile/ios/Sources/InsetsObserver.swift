import Tauri
import UIKit
import WebKit

private final class SafeAreaView: UIView {
    var didChange: (() -> Void)?

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        didChange?()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        didChange?()
    }
}

// UIKit state and the channel are confined to the main queue.
final class InsetsObserver {
    private struct Insets: Equatable {
        var safeArea: UIEdgeInsets = .zero
        var imeBottom: CGFloat = 0

        var payload: JsonObject {
            [
                "top": Double(safeArea.top),
                "right": Double(safeArea.right),
                "bottom": Double(safeArea.bottom),
                "left": Double(safeArea.left),
                "imeBottom": Double(imeBottom),
            ]
        }
    }

    private weak var webview: WKWebView?
    private var channel: Channel?
    private var latest: Insets?
    private var keyboardFrame: CGRect?
    private var observations: [NSObjectProtocol] = []
    private var safeAreaView: SafeAreaView?

    deinit {
        for observation in observations {
            NotificationCenter.default.removeObserver(observation)
        }
    }

    func attach(webview: WKWebView) {
        self.webview = webview
        refresh()
    }

    func register(_ channel: Channel) {
        if observations.isEmpty {
            let center = NotificationCenter.default
            observations = [
                center.addObserver(
                    forName: UIResponder.keyboardWillChangeFrameNotification,
                    object: nil, queue: .main
                ) { [weak self] notification in
                    self?.keyboardFrame = notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect
                    self?.refresh()
                },
                center.addObserver(
                    forName: UIResponder.keyboardWillHideNotification,
                    object: nil, queue: .main
                ) { [weak self] _ in
                    self?.keyboardFrame = nil
                    self?.refresh()
                },
                center.addObserver(
                    forName: UIWindow.didBecomeKeyNotification,
                    object: nil, queue: .main
                ) { [weak self] _ in self?.refresh() },
            ]
        }
        self.channel = channel
        latest = nil
        refresh()
    }

    func snapshot() -> JsonObject {
        read(window: currentWindow).payload
    }

    private var currentWindow: UIWindow? {
        if let window = webview?.window {
            return window
        }
        return UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .filter { $0.activationState == .foregroundActive }
            .flatMap { $0.windows }
            .first { $0.isKeyWindow }
    }

    private func refresh() {
        let window = currentWindow
        if safeAreaView?.window !== window {
            safeAreaView?.removeFromSuperview()
            safeAreaView = window.map { window in
                let view = SafeAreaView(frame: window.bounds)
                view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
                view.isUserInteractionEnabled = false
                view.accessibilityElementsHidden = true
                return view
            }
            if let view = safeAreaView {
                window?.addSubview(view)
                view.didChange = { [weak self] in self?.refresh() }
            }
        }

        let value = read(window: window)
        if value != latest {
            latest = value
            channel?.send(value.payload)
        }
    }

    private func read(window: UIWindow?) -> Insets {
        guard let window else { return Insets() }

        var bottom: CGFloat = 0
        if let keyboardFrame {
            let keyboard = window.convert(keyboardFrame, from: window.screen.coordinateSpace)
            let overlap = window.bounds.intersection(keyboard)
            if !overlap.isNull && overlap.maxY >= window.bounds.maxY {
                bottom = overlap.height
            }
        }
        return Insets(safeArea: window.safeAreaInsets, imeBottom: bottom)
    }
}
