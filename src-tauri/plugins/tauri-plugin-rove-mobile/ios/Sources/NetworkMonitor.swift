import Darwin
import Foundation
import Network
import Tauri

final class NetworkMonitor {
    struct Snapshot: Encodable, Equatable {
        let interfaces: [Interface]
    }

    struct Interface: Encodable, Equatable {
        let id: UInt64
        let name: String
        let addrs: [Address]
        let defaultV4: Bool
        let defaultV6: Bool
    }

    struct Address: Encodable, Hashable {
        let ip: String
        let prefixLen: Int
    }

    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "dev.roveterm.mobile.network")
    // The monitor queue owns the latest snapshot and channel.
    private var latest = Snapshot(interfaces: [])
    private var channel: Channel?

    init() {
        monitor.pathUpdateHandler = { [weak self] path in self?.update(path) }
        monitor.start(queue: queue)
    }

    deinit {
        monitor.cancel()
    }

    func snapshot() -> Snapshot {
        queue.sync { latest }
    }

    func register(_ channel: Channel) {
        queue.async {
            self.channel = channel
            self.sendSnapshot()
        }
    }

    private func update(_ path: NWPath) {
        let value = Self.read(path)
        guard value != latest else { return }
        latest = value
        sendSnapshot()
    }

    private func sendSnapshot() {
        do {
            try channel?.send(latest)
        } catch {
            NSLog("RoveTerm could not send the network snapshot")
        }
    }

    private static func read(_ path: NWPath) -> Snapshot {
        guard path.status == .satisfied else { return Snapshot(interfaces: []) }

        let names = Set(path.availableInterfaces.map(\.name))
        let primary = path.availableInterfaces.first?.name
        let gateways = path.gateways.map(\.debugDescription).sorted()
        let interfaces = addresses(on: names).map { name, addrs in
            Interface(
                id: interfaceID(name, gateways: gateways),
                name: name,
                addrs: addrs.sorted { ($0.ip, $0.prefixLen) < ($1.ip, $1.prefixLen) },
                defaultV4: name == primary && path.supportsIPv4,
                defaultV6: name == primary && path.supportsIPv6
            )
        }
        return Snapshot(interfaces: interfaces.sorted { $0.name < $1.name })
    }

    private static func interfaceID(_ name: String, gateways: [String]) -> UInt64 {
        // FNV-1a produces a stable ID across processes and gateway ordering.
        let identity = ([name] + gateways).joined(separator: "\0")
        let hash = identity.utf8.reduce(UInt64(14695981039346656037)) { hash, byte in
            (hash ^ UInt64(byte)) &* 1099511628211
        }
        return max(1, hash & UInt64(Int64.max))
    }

    private static func addresses(on names: Set<String>) -> [String: Set<Address>] {
        var head: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&head) == 0, let head else { return [:] }
        defer { freeifaddrs(head) }

        var result: [String: Set<Address>] = [:]
        var cursor: UnsafeMutablePointer<ifaddrs>? = head
        while let entry = cursor {
            let info = entry.pointee
            cursor = info.ifa_next
            guard info.ifa_flags & UInt32(IFF_UP) != 0,
                  info.ifa_flags & UInt32(IFF_LOOPBACK) == 0,
                  let name = info.ifa_name.map({ String(cString: $0) }),
                  names.contains(name),
                  let addr = info.ifa_addr,
                  let mask = info.ifa_netmask,
                  let prefixLen = prefixLength(mask, family: Int32(addr.pointee.sa_family)),
                  let ip = numericHost(addr)
            else { continue }
            result[name, default: []].insert(Address(ip: ip, prefixLen: prefixLen))
        }
        return result
    }

    private static func numericHost(_ addr: UnsafePointer<sockaddr>) -> String? {
        var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
        guard getnameinfo(addr, socklen_t(addr.pointee.sa_len), &host, socklen_t(host.count),
                          nil, 0, NI_NUMERICHOST) == 0
        else { return nil }
        let bytes = host.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }
        let address = String(decoding: bytes, as: UTF8.self)
        return String(address.prefix { $0 != "%" })
    }

    private static func prefixLength(_ mask: UnsafePointer<sockaddr>, family: Int32) -> Int? {
        let addressOffset: Int?
        let addressLength: Int
        switch family {
        case AF_INET:
            addressOffset = MemoryLayout<sockaddr_in>.offset(of: \.sin_addr)
            addressLength = MemoryLayout<in_addr>.size
        case AF_INET6:
            addressOffset = MemoryLayout<sockaddr_in6>.offset(of: \.sin6_addr)
            addressLength = MemoryLayout<in6_addr>.size
        default:
            return nil
        }
        guard let offset = addressOffset else { return nil }
        // A short BSD netmask omits trailing zero bytes.
        let count = min(Int(mask.pointee.sa_len) - offset, addressLength)
        guard count > 0 else { return 0 }
        let bytes = UnsafeRawBufferPointer(start: UnsafeRawPointer(mask) + offset, count: count)
        return bytes.reduce(0) { $0 + $1.nonzeroBitCount }
    }
}
