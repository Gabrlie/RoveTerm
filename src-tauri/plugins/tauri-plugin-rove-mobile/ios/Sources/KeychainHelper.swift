import Foundation
import LocalAuthentication
import Security

struct BridgeFailure: Error {
    let code: String
    let message: String

    init(_ code: String, _ message: String) {
        self.code = code
        self.message = message
    }
}

enum KeychainHelper {
    static let keyLength = 32

    // The generic attribute records protection alongside the secret in the same transaction.
    private static let silentMetadata = Data([0])
    private static let biometricMetadata = Data([1])

    static func load(service: String, silent: Bool, reason: String) throws -> Data? {
        var query = try identity(service)
        let metadataContext = LAContext()
        metadataContext.interactionNotAllowed = true
        defer { metadataContext.invalidate() }

        var result: CFTypeRef?
        let metadataQuery = query.merging([
            kSecUseAuthenticationContext as String: metadataContext,
            kSecReturnAttributes as String: true,
        ]) { _, new in new }
        let metadataStatus = SecItemCopyMatching(metadataQuery as CFDictionary, &result)
        if metadataStatus == errSecItemNotFound {
            return nil
        }
        let needsAuthentication: Bool
        if metadataStatus == errSecInteractionNotAllowed && !silent {
            needsAuthentication = true
        } else {
            try check(metadataStatus)
            guard let attributes = result as? [String: Any],
                  let metadata = attributes[kSecAttrGeneric as String] as? Data,
                  metadata == silentMetadata || metadata == biometricMetadata
            else {
                throw BridgeFailure("malformed", "Invalid vault authentication metadata")
            }
            needsAuthentication = metadata == biometricMetadata
        }
        if silent && needsAuthentication {
            throw BridgeFailure("denied", "Interactive authentication required")
        }

        let context = try authenticationContext(reason: reason, interactive: needsAuthentication)
        defer { context.invalidate() }
        query[kSecUseAuthenticationContext as String] = context
        query[kSecReturnData as String] = true

        result = nil
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            throw BridgeFailure("key_invalidated", "The protected vault item is missing")
        }
        try check(status)
        guard let key = result as? Data, key.count == keyLength else {
            throw BridgeFailure("malformed", "Invalid stored master key length")
        }
        return key
    }

    static func store(service: String, key: Data, biometric: Bool, reason: String) throws {
        var query = try identity(service)
        let context = try authenticationContext(reason: reason, interactive: biometric || !reason.isEmpty)
        defer { context.invalidate() }
        query[kSecUseAuthenticationContext as String] = context

        if biometric && !context.canEvaluatePolicy(.deviceOwnerAuthentication, error: nil) {
            throw BridgeFailure("biometric_unavailable", "Set up device authentication to protect this vault")
        }

        let protection = biometric
            ? kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly
            : kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let flags: SecAccessControlCreateFlags = biometric ? [.userPresence] : []
        guard let access = SecAccessControlCreateWithFlags(nil, protection, flags, nil) else {
            throw BridgeFailure("unavailable", "Could not configure vault key protection")
        }

        let attributes: [String: Any] = [
            kSecAttrAccessControl as String: access,
            kSecAttrGeneric as String: biometric ? biometricMetadata : silentMetadata,
            kSecValueData as String: key,
        ]
        let newItem = query.merging(attributes) { _, new in new }
        let status = SecItemAdd(newItem as CFDictionary, nil)
        if status == errSecDuplicateItem {
            try check(SecItemUpdate(query as CFDictionary, attributes as CFDictionary))
        } else {
            try check(status)
        }
    }

    static func authenticationStatus() -> [String: Bool] {
        var error: NSError?
        let enrolled = LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
        let code = error.flatMap { LAError.Code(rawValue: $0.code) }
        let supported = enrolled || code == .passcodeNotSet ||
            code == .biometryNotEnrolled || code == .biometryLockout
        return ["supported": supported, "enrolled": enrolled]
    }

    private static func identity(_ service: String) throws -> [String: Any] {
        guard !service.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !service.utf8.contains(0)
        else {
            throw BridgeFailure("invalid_input", "Invalid vault service name")
        }

        return [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: "master/v1",
        ]
    }

    private static func authenticationContext(reason: String, interactive: Bool) throws -> LAContext {
        let context = LAContext()
        if interactive {
            guard !reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw BridgeFailure("invalid_input", "Authentication requires a prompt title or subtitle")
            }
            context.localizedReason = reason
        } else {
            context.interactionNotAllowed = true
        }
        return context
    }

    private static func check(_ status: OSStatus) throws {
        switch status {
        case errSecSuccess:
            return
        case errSecUserCanceled, errSecAuthFailed, errSecInteractionNotAllowed:
            throw BridgeFailure("denied", "Vault access was denied")
        case errSecDecode:
            throw BridgeFailure("malformed", "Could not decode the vault item")
        default:
            throw BridgeFailure("unavailable", "Keychain operation failed (\(status))")
        }
    }
}
