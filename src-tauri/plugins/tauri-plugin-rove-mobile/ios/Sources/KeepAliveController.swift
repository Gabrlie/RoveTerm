import Foundation
import Tauri
import UIKit

// The channel, activity flag, and task identifier are confined to the main queue.
final class KeepAliveController {
    private var active = false
    private var channel: Channel?
    private var backgroundTask: UIBackgroundTaskIdentifier = .invalid
    private var observations: [NSObjectProtocol] = []

    deinit {
        for observation in observations {
            NotificationCenter.default.removeObserver(observation)
        }
        let task = backgroundTask
        if task != .invalid {
            if Thread.isMainThread {
                MainActor.assumeIsolated { UIApplication.shared.endBackgroundTask(task) }
            } else {
                DispatchQueue.main.async { UIApplication.shared.endBackgroundTask(task) }
            }
        }
    }

    @MainActor
    func register(_ channel: Channel) {
        self.channel = channel
        guard observations.isEmpty else { return }

        let center = NotificationCenter.default
        observations = [
            center.addObserver(
                forName: UIApplication.didEnterBackgroundNotification,
                object: nil, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.didEnterBackground() }
            },
            center.addObserver(
                forName: UIApplication.willEnterForegroundNotification,
                object: nil, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.willEnterForeground() }
            },
        ]
    }

    @MainActor
    func update(active: Bool) {
        self.active = active
        if !active { endBackgroundTask() }
    }

    @MainActor
    private func didEnterBackground() {
        if active && backgroundTask == .invalid {
            backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "dev.roveterm.keepalive") {
                [weak self] in self?.endBackgroundTask()
            }
        }
        channel?.send(["kind": "lifecycle", "state": "background"] as JsonObject)
    }

    @MainActor
    private func willEnterForeground() {
        endBackgroundTask()
        channel?.send(["kind": "lifecycle", "state": "foreground"] as JsonObject)
    }

    @MainActor
    private func endBackgroundTask() {
        let task = backgroundTask
        guard task != .invalid else { return }
        backgroundTask = .invalid
        UIApplication.shared.endBackgroundTask(task)
    }
}
