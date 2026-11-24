import Foundation
import Tauri
import UIKit
import WebKit

private protocol KeyArgs: Decodable {
    var promptTitle: String { get }
    var promptSubtitle: String { get }
}

extension KeyArgs {
    var reason: String {
        [promptTitle, promptSubtitle]
            .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .joined(separator: "\n")
    }
}

private struct LoadArgs: KeyArgs {
    let service: String
    let silent: Bool
    let promptTitle: String
    let promptSubtitle: String
}

private struct StoreArgs: KeyArgs {
    let service: String
    let key: String
    let biometric: Bool
    let promptTitle: String
    let promptSubtitle: String
}

private struct LoadResponse: Encodable {
    let key: String?
}

private struct URLArgs: Decodable {
    let url: String
}

private struct RegisterInsetsArgs: Decodable {
    let channel: Channel
}

final class RoveMobilePlugin: Plugin {
    private let insets = InsetsObserver()
    private let vaultQueue = DispatchQueue(label: "dev.roveterm.mobile.keychain")

    override func load(webview: WKWebView) {
        DispatchQueue.main.async { self.insets.attach(webview: webview) }
    }

    @objc func loadMasterKey(_ invoke: Invoke) {
        vaultQueue.async {
            Self.run(invoke) {
                let args = try invoke.parseArgs(LoadArgs.self)
                var key = try KeychainHelper.load(
                    service: args.service, silent: args.silent, reason: args.reason
                )
                defer {
                    if let count = key?.count { key?.resetBytes(in: 0..<count) }
                }
                invoke.resolve(LoadResponse(key: key?.base64EncodedString()))
            }
        }
    }

    @objc func storeMasterKey(_ invoke: Invoke) {
        vaultQueue.async {
            Self.run(invoke) {
                let args = try invoke.parseArgs(StoreArgs.self)
                guard args.key.utf8.count == 44,
                      var key = Data(base64Encoded: args.key)
                else {
                    throw BridgeFailure("invalid_input", "Invalid master key encoding")
                }
                defer { key.resetBytes(in: 0..<key.count) }
                guard key.count == KeychainHelper.keyLength else {
                    throw BridgeFailure("invalid_input", "Invalid master key length")
                }
                try KeychainHelper.store(
                    service: args.service, key: key, biometric: args.biometric, reason: args.reason
                )
                invoke.resolve()
            }
        }
    }

    @objc func biometricStatus(_ invoke: Invoke) {
        invoke.resolve(KeychainHelper.authenticationStatus())
    }

    @objc func openExternalUrl(_ invoke: Invoke) {
        Self.run(invoke) {
            let args = try invoke.parseArgs(URLArgs.self)
            guard let url = URL(string: args.url),
                  let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme),
                  let host = url.host, !host.isEmpty
            else {
                throw BridgeFailure("invalid_input", "External URL requires HTTP or HTTPS and a host")
            }
            DispatchQueue.main.async {
                UIApplication.shared.open(url) { opened in
                    if opened {
                        invoke.resolve()
                    } else {
                        invoke.reject("Could not open the external URL", code: "unavailable")
                    }
                }
            }
        }
    }

    @objc func getInsets(_ invoke: Invoke) {
        DispatchQueue.main.async { invoke.resolve(self.insets.snapshot()) }
    }

    @objc func registerInsetsChannel(_ invoke: Invoke) {
        Self.run(invoke) {
            let args = try invoke.parseArgs(RegisterInsetsArgs.self)
            // The Rust setup can be waiting for the acknowledgement on the UI thread.
            invoke.resolve()
            DispatchQueue.main.async { self.insets.register(args.channel) }
        }
    }

    private static func run(_ invoke: Invoke, _ operation: () throws -> Void) {
        do {
            try operation()
        } catch let error as BridgeFailure {
            invoke.reject(error.message, code: error.code)
        } catch {
            invoke.reject("Invalid mobile bridge arguments", code: "invalid_input")
        }
    }
}

@_cdecl("init_plugin_rove_mobile")
public func initPluginRoveMobile() -> Plugin {
    RoveMobilePlugin()
}
