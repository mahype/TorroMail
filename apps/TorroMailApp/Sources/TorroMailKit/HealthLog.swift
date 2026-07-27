import Foundation

/// What one login attempt proved. Three-valued because "we could not get
/// there" and "it said no" need opposite handling: a wrong password will not
/// fix itself, a train tunnel will.
public enum HealthOutcome: String, Hashable, Sendable {
    case ok
    case rejected
    case unreachable
}

/// One check, as the server or the app recorded it.
public struct HealthRecord: Hashable, Sendable {
    public var accountID: String
    public var at: Date
    public var outcome: HealthOutcome
    /// `periodic`, `server-start`, `tool-call` or `manual` — kept for the log,
    /// nothing derives from it.
    public var source: String
    public var detail: String

    public init(
        accountID: String,
        at: Date,
        outcome: HealthOutcome,
        source: String = "",
        detail: String = ""
    ) {
        self.accountID = accountID
        self.at = at
        self.outcome = outcome
        self.source = source
        self.detail = detail
    }
}

/// Reads and appends `health.jsonl` — the log the MCP server and the app both
/// write, and the only thing that decides an account's status dot. Sibling of
/// `audit.jsonl`, and shares its never-throw contract: a missing file or a
/// half-written trailing line yields what it can rather than taking the
/// window down.
public enum HealthLog {
    /// How many consecutive unreachable checks it takes before an account is
    /// called broken. Below this it keeps whatever it was, because a flaky
    /// network is not a credential problem and a false red teaches people to
    /// ignore the dot.
    public static let unreachableGrace = 3

    /// `~/Library/Application Support/TorroMail/health.jsonl` — beside the
    /// policy document and the two logs the server already writes.
    public static func defaultURL(fileManager: FileManager = .default) throws -> URL {
        try fileManager
            .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("TorroMail", isDirectory: true)
            .appendingPathComponent("health.jsonl")
    }

    /// The log, oldest first, keeping the last `perAccount` records **for each
    /// account** rather than the last N lines of the file.
    ///
    /// The distinction is the whole reason this is not a plain `suffix`: with a
    /// global tail, one long assistant session against a busy account crowds
    /// every other account's records off the end, and an account with no
    /// records falls back to the stored state — the stale-green bug this
    /// feature exists to remove, back again and only under load.
    public static func load(
        perAccount: Int = 20,
        from url: URL? = nil,
        fileManager: FileManager = .default
    ) -> [HealthRecord] {
        guard let target = try? url ?? defaultURL(fileManager: fileManager),
              let text = try? String(contentsOf: target, encoding: .utf8) else {
            return []
        }
        let parsed = text
            .split(separator: "\n", omittingEmptySubsequences: true)
            .compactMap { line -> HealthRecord? in
                guard let data = line.data(using: .utf8),
                      let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let ts = raw["ts"] as? TimeInterval,
                      let account = raw["account"] as? String,
                      let word = raw["outcome"] as? String,
                      // An outcome word this build has never heard of is
                      // skipped rather than fatal: an older or newer writer
                      // must not take the status display down with it.
                      let outcome = HealthOutcome(rawValue: word) else {
                    return nil
                }
                return HealthRecord(
                    accountID: account,
                    at: Date(timeIntervalSince1970: ts),
                    outcome: outcome,
                    source: raw["source"] as? String ?? "",
                    detail: raw["detail"] as? String ?? ""
                )
            }

        // Keep the newest `perAccount` for each account, then hand them back
        // in file order so callers still see a single oldest-first sequence.
        var kept: [HealthRecord] = []
        var counts: [String: Int] = [:]
        for record in parsed.reversed() {
            let count = counts[record.accountID] ?? 0
            guard count < perAccount else { continue }
            counts[record.accountID] = count + 1
            kept.append(record)
        }
        return kept.reversed()
    }

    /// Append one record. Best-effort: a missed sample costs at most one
    /// interval, and nothing the user does should fail over a log line.
    ///
    /// Deliberately `O_APPEND` and a single `write` rather than
    /// `FileHandle.seekToEnd` plus a write: the MCP server appends to this same
    /// file from its own processes, and a seek followed by a write is two
    /// syscalls another writer can slip between — which shows up as one line
    /// stamped over another. `O_APPEND` makes the seek part of the write, so
    /// concurrent appenders interleave whole lines or not at all.
    public static func append(
        _ record: HealthRecord,
        to url: URL? = nil,
        fileManager: FileManager = .default
    ) {
        guard let target = try? url ?? defaultURL(fileManager: fileManager) else { return }
        let payload: [String: Any] = [
            "ts": record.at.timeIntervalSince1970,
            "account": record.accountID,
            "outcome": record.outcome.rawValue,
            "source": record.source,
            "detail": record.detail
        ]
        // JSON escapes any newline inside a detail string, so one record stays
        // one line whatever the server said.
        guard let data = try? JSONSerialization.data(withJSONObject: payload) else { return }
        var line = data
        line.append(0x0A)

        // The server usually creates the directory first, but the app may
        // record a check before the server has ever run.
        try? fileManager.createDirectory(
            at: target.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let descriptor = target.withUnsafeFileSystemRepresentation { path -> Int32 in
            guard let path else { return -1 }
            return open(path, O_WRONLY | O_APPEND | O_CREAT, 0o644)
        }
        guard descriptor >= 0 else { return }
        defer { close(descriptor) }
        line.withUnsafeBytes { buffer in
            guard let base = buffer.baseAddress else { return }
            var written = 0
            while written < buffer.count {
                let n = write(descriptor, base + written, buffer.count - written)
                if n > 0 {
                    written += n
                } else if n < 0 && errno == EINTR {
                    continue
                } else {
                    // A disk that is full or gone: drop the sample rather than
                    // spin. The next check writes a fresh one.
                    return
                }
            }
        }
    }

    /// The status one account's records add up to.
    ///
    /// `fallback` is what the account carried before any record existed — the
    /// state restored from disk at launch. It is also what an unreachable
    /// streak too short to count falls back to, which is the whole grace rule:
    /// nothing changes until we are sure.
    public static func derive(
        records: [HealthRecord],
        accountID: String,
        fallback: ConnectionState,
        grace: Int = unreachableGrace
    ) -> ConnectionState {
        // Sorted by time, with the caller's order breaking ties: two records a
        // second apart in the file must not be able to swap places and flip the
        // dot, and `sorted(by:)` alone gives no such guarantee.
        let mine = records.enumerated()
            .filter { $0.element.accountID == accountID }
            .sorted { ($0.element.at, $0.offset) < ($1.element.at, $1.offset) }
            .map(\.element)
        guard let last = mine.last else { return fallback }

        switch last.outcome {
        case .ok:
            return .connected
        case .rejected:
            return .failed(reason(last, or: "credentials rejected"))
        case .unreachable:
            let streak = mine.reversed().prefix { $0.outcome == .unreachable }.count
            guard streak >= grace else {
                // Not yet convinced. Whatever the last real verdict was still
                // stands — a couple of missed checks is a network, not an
                // account, and it is no evidence a refused password started
                // working again either.
                guard let settled = mine.last(where: { $0.outcome != .unreachable }) else {
                    return fallback
                }
                return settled.outcome == .ok
                    ? .connected
                    : .failed(reason(settled, or: "credentials rejected"))
            }
            return .failed(reason(last, or: "server not reachable"))
        }
    }

    /// What to show the user for a failing check: the writer's own words when
    /// it gave any, a plain sentence when it did not. An empty `.failed("")`
    /// would still be red but tell nobody why.
    private static func reason(_ record: HealthRecord, or fallback: String) -> String {
        record.detail.isEmpty ? fallback : record.detail
    }

    /// When each account was last checked, for the "last checked …" line in
    /// the account detail.
    public static func lastChecked(records: [HealthRecord]) -> [String: Date] {
        var latest: [String: Date] = [:]
        for record in records where (latest[record.accountID] ?? .distantPast) < record.at {
            latest[record.accountID] = record.at
        }
        return latest
    }
}

/// A crossing between healthy and broken — the only thing worth a
/// notification. A standing problem is not a crossing: it stays red in the
/// app, which is where a standing problem belongs.
public enum HealthTransition: Hashable, Sendable {
    case broke(accountID: String, reason: String)
    case recovered(accountID: String)
}

extension HealthLog {
    /// Each account's broken-ness, the shape `transitions` compares against.
    public static func brokenness(accounts: [MailAccount]) -> [String: Bool] {
        Dictionary(
            accounts.map { ($0.id, $0.connectionState.isBroken) },
            uniquingKeysWith: { first, _ in first }
        )
    }

    /// Which accounts crossed since `previous` was taken.
    ///
    /// Pure on purpose: deciding what counts as a crossing is the part worth
    /// testing, and it should not need a notification centre to exercise. An
    /// account missing from `previous` is new and yields nothing — it has not
    /// crossed anything, it has only appeared.
    public static func transitions(
        previous: [String: Bool],
        accounts: [MailAccount]
    ) -> [HealthTransition] {
        accounts.compactMap { account in
            guard let was = previous[account.id] else { return nil }
            let broken = account.connectionState.isBroken
            guard was != broken else { return nil }
            guard broken else { return .recovered(accountID: account.id) }
            var reason = ""
            if case let .failed(message) = account.connectionState {
                reason = message
            }
            return .broke(accountID: account.id, reason: reason)
        }
    }
}
