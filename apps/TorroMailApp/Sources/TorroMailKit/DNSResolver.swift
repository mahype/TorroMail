import Foundation
import dnssd

/// MX and SRV lookups. Foundation resolves names but will not hand out these
/// record types, so this drops to `dnssd` — the same resolver the system uses,
/// caches included.
///
/// Both queries are best-effort: autodiscovery treats a nil as "this route had
/// nothing to say" and moves on to the next one.
public enum DNSResolver {
    public struct SRVRecord: Hashable, Sendable {
        public let priority: UInt16
        public let weight: UInt16
        public let port: UInt16
        public let target: String
    }

    struct MXRecord: Hashable {
        let preference: UInt16
        let host: String
    }

    private static let typeMX: UInt16 = 15
    private static let typeSRV: UInt16 = 33
    private static let classIN: UInt16 = 1

    /// Mail exchangers for a domain, lowest preference first. This is how a
    /// company domain reveals that it is really hosted at Google or Microsoft.
    static func mailExchangers(for domain: String, timeout: TimeInterval = 2) -> [MXRecord] {
        let records = query(name: domain, type: typeMX, timeout: timeout).compactMap { data -> MXRecord? in
            guard data.count > 2 else { return nil }
            let preference = UInt16(data[0]) << 8 | UInt16(data[1])
            guard let host = decodeName(data, from: 2) else { return nil }
            return MXRecord(preference: preference, host: host)
        }
        return records.sorted { $0.preference < $1.preference }
    }

    /// RFC 6186 service location: `_imaps._tcp.<domain>` names the IMAP server
    /// a domain wants clients to use.
    public static func imapService(for domain: String, timeout: TimeInterval = 2) -> SRVRecord? {
        let records = query(name: "_imaps._tcp.\(domain)", type: typeSRV, timeout: timeout)
            .compactMap { data -> SRVRecord? in
                guard data.count > 6 else { return nil }
                let priority = UInt16(data[0]) << 8 | UInt16(data[1])
                let weight = UInt16(data[2]) << 8 | UInt16(data[3])
                let port = UInt16(data[4]) << 8 | UInt16(data[5])
                guard let target = decodeName(data, from: 6), !target.isEmpty else { return nil }
                return SRVRecord(priority: priority, weight: weight, port: port, target: target)
            }
        return records.min { $0.priority < $1.priority }
    }

    /// A DNS name in wire format: length-prefixed labels, zero-terminated.
    /// Compression pointers (RFC 1035 §4.1.4) cannot be followed here because
    /// only the rdata is in hand, not the message it came from — mDNSResponder
    /// expands them for unicast answers, so hitting one means something is off
    /// and nil is the honest answer.
    private static func decodeName(_ data: [UInt8], from start: Int) -> String? {
        var labels: [String] = []
        var index = start

        while index < data.count {
            let length = Int(data[index])
            if length == 0 {
                return labels.joined(separator: ".")
            }
            if length & 0xC0 != 0 { return nil }
            let from = index + 1
            let to = from + length
            guard to <= data.count else { return nil }
            guard let label = String(bytes: data[from..<to], encoding: .utf8) else { return nil }
            labels.append(label)
            index = to
        }
        return nil
    }

    /// One synchronous query. `dnssd` is callback-driven over a socket, so the
    /// answers are collected into a box and the socket is pumped until the
    /// resolver says it is done or the clock runs out.
    private static func query(name: String, type: UInt16, timeout: TimeInterval) -> [[UInt8]] {
        final class Collector {
            var answers: [[UInt8]] = []
            var moreComing = true
        }

        let collector = Collector()
        var service: DNSServiceRef?

        let callback: DNSServiceQueryRecordReply = {
            _, flags, _, errorCode, _, _, _, rdlen, rdata, _, context in
            guard let context else { return }
            let collector = Unmanaged<Collector>.fromOpaque(context).takeUnretainedValue()
            collector.moreComing = flags & kDNSServiceFlagsMoreComing != 0

            guard errorCode == kDNSServiceErr_NoError, let rdata, rdlen > 0 else { return }
            let buffer = UnsafeRawBufferPointer(start: rdata, count: Int(rdlen))
            collector.answers.append([UInt8](buffer))
        }

        let context = Unmanaged.passUnretained(collector).toOpaque()
        let status = DNSServiceQueryRecord(
            &service, 0, 0, name, type, classIN, callback, context
        )
        guard status == kDNSServiceErr_NoError, let service else { return [] }
        defer { DNSServiceRefDeallocate(service) }

        let socket = DNSServiceRefSockFD(service)
        guard socket >= 0 else { return [] }

        let deadline = Date().addingTimeInterval(timeout)
        while collector.answers.isEmpty || collector.moreComing {
            let remaining = deadline.timeIntervalSinceNow
            guard remaining > 0 else { break }

            var descriptors = pollfd(fd: socket, events: Int16(POLLIN), revents: 0)
            let ready = poll(&descriptors, 1, Int32(remaining * 1000))
            guard ready > 0 else { break }
            guard DNSServiceProcessResult(service) == kDNSServiceErr_NoError else { break }
        }
        return collector.answers
    }
}
