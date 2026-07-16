import Foundation
import Network

/// Works out where a mailbox lives from nothing but its address.
///
/// The order is Thunderbird's, and it is deliberate: cheap and certain first,
/// network guesses last. Every route can fail silently — the wizard falls
/// through to manual entry rather than dead-ending, so a miss costs the user
/// some typing, never the setup.
public enum Autodiscovery {
    /// Total budget for the whole chain. Past this the wizard shows manual
    /// fields; waiting longer would be worse than asking.
    public static let budget: TimeInterval = 8

    public static func domain(of email: String) -> String? {
        let parts = email.split(separator: "@", omittingEmptySubsequences: true)
        guard parts.count == 2 else { return nil }
        let domain = parts[1].trimmingCharacters(in: .whitespaces).lowercased()
        return domain.contains(".") ? domain : nil
    }

    /// Runs the chain under a deadline. Returns nil when every route came up
    /// empty or the budget ran out — either way the wizard shows manual
    /// fields, so a slow network costs the user typing, not the setup.
    public static func resolve(email: String) async -> DiscoveredConfig? {
        await withTaskGroup(of: DiscoveredConfig?.self) { group in
            group.addTask { await chain(email: email) }
            group.addTask {
                try? await Task.sleep(nanoseconds: UInt64(budget * 1_000_000_000))
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
    }

    /// The chain itself. Every step is best-effort; the caller enforces the
    /// deadline around the whole thing.
    private static func chain(email: String) async -> DiscoveredConfig? {
        guard let domain = domain(of: email) else { return nil }

        // 1. The table — no network, no waiting.
        if let known = ProviderCatalog.lookup(domain: domain) {
            return known
        }

        // 2/3. The domain's own autoconfig. A provider publishing this knows
        // its servers better than any guess of ours.
        for url in autoconfigURLs(domain: domain, email: email) {
            if let config = await fetchAutoconfig(url: url, source: "autoconfig") {
                return config
            }
        }

        // 4. Mozilla's ISPDB — a shared table for the long tail of ISPs.
        let ispdb = URL(string: "https://autoconfig.thunderbird.net/v1.1/\(domain)")!
        if let config = await fetchAutoconfig(url: ispdb, source: "ispdb") {
            return config
        }

        // 5. MX. This is the one that matters for company domains: the address
        // says firma.de, the mail actually sits at Google or Microsoft, and
        // only OAuth will ever get in. Guessing imap.firma.de here would hand
        // the user a password field that cannot work.
        let exchangers = DNSResolver.mailExchangers(for: domain)
        for exchanger in exchangers {
            if let config = ProviderCatalog.fromMXHost(exchanger.host) {
                return config
            }
        }
        // A third-party MX often shares the mail host's domain — try its
        // autoconfig before falling back to guesswork.
        if let base = exchangers.first.map({ baseDomain(of: $0.host) }),
           base != domain,
           let known = ProviderCatalog.lookup(domain: base) {
            return known
        }

        // 6. RFC 6186: the domain naming its own IMAP server.
        if let srv = DNSResolver.imapService(for: domain), !srv.target.isEmpty {
            return DiscoveredConfig(
                imapHost: srv.target,
                imapPort: Int(srv.port),
                smtpHost: "smtp.\(domain)",
                auth: .password,
                providerLabel: domain,
                source: "srv"
            )
        }

        // 7. The guess, but only if something actually answers on 993. An
        // unreachable host in the field is worse than an empty one.
        for candidate in ["imap.\(domain)", "mail.\(domain)"] where await probe(host: candidate) {
            return DiscoveredConfig(
                imapHost: candidate,
                smtpHost: candidate.replacingOccurrences(of: "imap.", with: "smtp."),
                auth: .password,
                providerLabel: domain,
                source: "probe"
            )
        }

        return nil
    }

    private static func autoconfigURLs(domain: String, email: String) -> [URL] {
        let escaped = email.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? email
        return [
            URL(string: "https://autoconfig.\(domain)/mail/config-v1.1.xml?emailaddress=\(escaped)"),
            URL(string: "https://\(domain)/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=\(escaped)")
        ].compactMap { $0 }
    }

    /// Only the registrable-ish tail: `aspmx.l.google.com` → `google.com`.
    /// Crude on purpose — it feeds a table lookup that either hits or does not.
    private static func baseDomain(of host: String) -> String {
        let parts = host.lowercased().split(separator: ".")
        guard parts.count >= 2 else { return host.lowercased() }
        return parts.suffix(2).joined(separator: ".")
    }

    private static func fetchAutoconfig(url: URL, source: String) async -> DiscoveredConfig? {
        var request = URLRequest(url: url)
        request.timeoutInterval = 3
        request.setValue("TorroMail", forHTTPHeaderField: "User-Agent")

        guard let (data, response) = try? await URLSession.shared.data(for: request),
              let http = response as? HTTPURLResponse,
              http.statusCode == 200
        else { return nil }

        return AutoconfigParser.parse(data, source: source)
    }

    /// Can we open a TLS-less TCP connection to 993? Enough to tell a real
    /// server from a hopeful hostname; the login step does the real check.
    private static func probe(host: String, port: UInt16 = 993) async -> Bool {
        await withCheckedContinuation { continuation in
            let connection = NWConnectionProbe(host: host, port: port, timeout: 2)
            connection.run { reachable in
                continuation.resume(returning: reachable)
            }
        }
    }
}

/// "Does anything answer on this port?" — one TCP connection, then hang up.
///
/// The callback fires exactly once whether the connection lands, fails, or the
/// timeout wins, because the caller is an `async` continuation and resuming one
/// twice is a crash.
final class NWConnectionProbe: @unchecked Sendable {
    private let connection: NWConnection
    private let timeout: TimeInterval
    private let lock = NSLock()
    private var finished = false

    init(host: String, port: UInt16, timeout: TimeInterval) {
        self.connection = NWConnection(
            host: NWEndpoint.Host(host),
            port: NWEndpoint.Port(rawValue: port) ?? 993,
            using: .tcp
        )
        self.timeout = timeout
    }

    /// Holds itself alive until it answers. Nobody else keeps a reference —
    /// the caller is an `async` continuation that has already suspended — so a
    /// weak capture here would deallocate the probe the moment `run` returned
    /// and the continuation would never resume.
    func run(_ completion: @escaping @Sendable (Bool) -> Void) {
        let finish: @Sendable (Bool) -> Void = { reachable in
            self.lock.lock()
            let alreadyFinished = self.finished
            self.finished = true
            self.lock.unlock()
            guard !alreadyFinished else { return }

            self.connection.cancel()
            // Breaks the self → connection → handler → self cycle that the
            // strong capture above creates.
            self.connection.stateUpdateHandler = nil
            completion(reachable)
        }

        connection.stateUpdateHandler = { state in
            switch state {
            case .ready: finish(true)
            // `.waiting` is the interesting one: an unresolvable host parks
            // there and NWConnection retries forever, so only the timeout ends
            // it. That is what the deadline below is for.
            case .failed, .cancelled: finish(false)
            default: break
            }
        }
        connection.start(queue: .global(qos: .userInitiated))
        DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { finish(false) }
    }
}

/// Reads Mozilla's client-config XML — the format both a domain's own
/// autoconfig and the ISPDB speak.
enum AutoconfigParser {
    static func parse(_ data: Data, source: String) -> DiscoveredConfig? {
        let delegate = Delegate()
        let parser = XMLParser(data: data)
        parser.delegate = delegate
        guard parser.parse(), let incoming = delegate.incoming, !incoming.hostname.isEmpty else {
            return nil
        }

        let outgoing = delegate.outgoing
        let auth: AuthPath
        if incoming.authentication.lowercased().contains("oauth2"),
           let issuer = issuer(forHost: incoming.hostname) {
            auth = .oauth(issuer)
        } else {
            auth = .password
        }

        return DiscoveredConfig(
            imapHost: incoming.hostname,
            imapPort: incoming.port ?? 993,
            smtpHost: outgoing?.hostname ?? "",
            smtpPort: outgoing?.port ?? 587,
            auth: auth,
            providerLabel: delegate.displayName ?? incoming.hostname,
            provider: provider(forHost: incoming.hostname),
            source: source
        )
    }

    /// OAuth2 in the XML tells us the server wants a token, but not who issues
    /// it — and a token is worthless without a registered client. So an OAuth2
    /// hint only counts for issuers we can actually talk to; anything else
    /// degrades to a password field, which at least has a chance of working.
    private static func issuer(forHost host: String) -> OAuthIssuer? {
        let host = host.lowercased()
        if host.hasSuffix("gmail.com") || host.hasSuffix("googlemail.com") { return .google }
        if host.hasSuffix("office365.com") || host.hasSuffix("outlook.com") { return .microsoft }
        return nil
    }

    private static func provider(forHost host: String) -> Provider {
        switch issuer(forHost: host) {
        case .google: .gmail
        case .microsoft: .microsoft
        case nil: .imapSmtp
        }
    }

    struct Server {
        var hostname = ""
        var port: Int?
        var authentication = ""
        var socketType = ""
    }

    private final class Delegate: NSObject, XMLParserDelegate {
        var incoming: Server?
        var outgoing: Server?
        var displayName: String?

        private var current: Server?
        private var currentIsIncoming = false
        private var element = ""
        private var text = ""

        func parser(
            _ parser: XMLParser,
            didStartElement elementName: String,
            namespaceURI: String?,
            qualifiedName: String?,
            attributes: [String: String]
        ) {
            element = elementName
            text = ""

            switch elementName {
            case "incomingServer":
                // POP is listed alongside IMAP; TorroMail only speaks IMAP.
                currentIsIncoming = true
                current = attributes["type"]?.lowercased() == "imap" ? Server() : nil
            case "outgoingServer":
                currentIsIncoming = false
                current = Server()
            default:
                break
            }
        }

        func parser(_ parser: XMLParser, foundCharacters string: String) {
            text += string
        }

        func parser(
            _ parser: XMLParser,
            didEndElement elementName: String,
            namespaceURI: String?,
            qualifiedName: String?
        ) {
            let value = text.trimmingCharacters(in: .whitespacesAndNewlines)

            switch elementName {
            case "hostname": current?.hostname = value
            case "port": current?.port = Int(value)
            case "authentication":
                // Several are listed in preference order; OAuth2 anywhere in
                // the list means the server offers it.
                if current?.authentication.isEmpty ?? false || value.lowercased().contains("oauth2") {
                    current?.authentication = value
                }
            case "socketType": current?.socketType = value
            case "displayName": displayName = displayName ?? value
            case "incomingServer":
                if let current, incoming == nil { incoming = current }
                current = nil
            case "outgoingServer":
                if let current, outgoing == nil { outgoing = current }
                current = nil
            default:
                break
            }
            text = ""
        }
    }
}
