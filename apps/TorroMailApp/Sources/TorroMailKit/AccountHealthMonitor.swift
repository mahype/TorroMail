import Foundation

#if canImport(AppKit)
import AppKit
#endif

/// Checks every account on a timer and writes what it finds to
/// `health.jsonl`. Nothing here touches the UI: the app watches that file the
/// same way it watches the audit log, so one path updates the dots no matter
/// who wrote the record — this monitor, a tool call, or a starting server.
///
/// Checks run one after another rather than at once. A handful of accounts
/// arriving as a burst of simultaneous logins is exactly the kind of traffic
/// providers throttle.
///
/// `@unchecked` because the invariant is one the compiler cannot see, not one
/// that is missing: every mutable property is read and written inside
/// `lock.withLock`, and the wake observer only inside `start`/`stop`/`deinit`,
/// which the owner calls from the main actor. The alternative — leaving the
/// class non-`Sendable` — is not safer, only quieter about it: a timer handler
/// and a notification block are both `@Sendable`, so the state would cross
/// threads either way.
public final class AccountHealthMonitor: @unchecked Sendable {
    /// Fifteen minutes. Not a setting: the user cannot make a better decision
    /// about it than we can, and a control that is not a decision does not
    /// belong in the UI.
    public static let defaultInterval: TimeInterval = 900

    /// How long after waking to wait before checking. A wake notification
    /// arrives before the Wi-Fi has reassociated, so a check fired on it
    /// reports the machine's state and files it against the account: three of
    /// those in a row is exactly the streak that turns a working account red.
    /// Half a minute is enough for the network to come back and still far
    /// inside the fifteen the user would otherwise wait.
    private static let wakeDelay: TimeInterval = 30

    private let interval: TimeInterval
    private let logURL: URL?
    private let check: @Sendable (_ accountID: String, _ executableName: String) -> AccountCheckResult
    /// Serial, and the only place a check ever runs — so "one login at a time"
    /// is a property of the queue rather than something every caller has to
    /// remember.
    private let queue = DispatchQueue(label: "de.torro.mail.health", qos: .utility)

    private let lock = NSLock()
    /// What to check, as last published from the main actor.
    ///
    /// A snapshot rather than a closure back into the model on purpose: the
    /// checks run on a background queue, and reaching into main-actor state
    /// from there is a runtime trap, not a compile error.
    private var accountIDs: [String] = []
    private var executableName = ""
    /// Bumped by every `start`, `stop` and wake. A pass carries the number it
    /// began under and abandons the moment it stops matching — the only way to
    /// interrupt a pass, because `cancel()` does nothing to a handler that is
    /// already running and a pass can sit in one account's TLS handshake for
    /// half a minute.
    private var generation = 0
    /// The pending tick. Held so `stop()` can cancel a wait that has not fired
    /// yet, and because an unreferenced source is a source nobody can stop.
    private var timer: DispatchSourceTimer?

    private var wakeObserver: NSObjectProtocol?
    private var wakeCenter: NotificationCenter?

    /// `logURL` and `check` exist for the contract test, which has to pin the
    /// scheduling decisions without spawning a process per account or writing
    /// to the real log. Both default to what the app uses.
    public init(
        interval: TimeInterval = defaultInterval,
        logURL: URL? = nil,
        check: @escaping @Sendable (_ accountID: String, _ executableName: String) -> AccountCheckResult = {
            AccountCheck.run(accountID: $0, executableName: $1)
        }
    ) {
        self.interval = interval
        self.logURL = logURL
        self.check = check
    }

    /// Publish what the next pass should check. Called from the main actor
    /// whenever the accounts or the configured executable change, including
    /// once before `start()`.
    ///
    /// Takes the lock rather than hopping onto the work queue: a queue hop
    /// would put the new accounts behind whatever pass is running, and a pass
    /// with a hung account in it runs for minutes. A pass reads the snapshot
    /// once, when it begins, so an update landing mid-pass takes effect next
    /// time rather than half way down the list.
    public func update(accountIDs: [String], executableName: String) {
        lock.withLock {
            self.accountIDs = accountIDs
            self.executableName = executableName
        }
    }

    /// Starts the timer and checks immediately: a status the app shows at
    /// launch should be from seconds ago, not from whenever it last quit.
    public func start() {
        observeWake()
        restart(after: 0)
    }

    public func stop() {
        // Bumping the generation is what actually stops things: it cancels the
        // pending tick, and tells a pass that is already half way through its
        // accounts to drop the rest.
        lock.withLock {
            generation += 1
            timer?.cancel()
            timer = nil
        }
        stopObservingWake()
    }

    deinit { stop() }

    // MARK: - Scheduling

    /// Begins a new generation and schedules its first pass. Everything that
    /// wants the cadence to start over — launch, a restart, waking up — goes
    /// through here, so there is exactly one way a tick gets scheduled and no
    /// way to end up with two cadences running at once.
    private func restart(after delay: TimeInterval) {
        let generation = lock.withLock { () -> Int in
            self.generation += 1
            self.timer?.cancel()
            self.timer = nil
            return self.generation
        }
        schedule(after: delay, generation: generation)
    }

    /// A one-shot timer per tick, rescheduled only once a pass has finished.
    ///
    /// A repeating timer would be wrong here: with several unreachable
    /// accounts a pass can outlast its own period — `AccountCheck.run` waits
    /// out a hung server for `timeout + 2` seconds, and that is per account —
    /// and dispatch answers an overrun by firing the moment the queue frees up.
    /// The interval would stop being a quiet gap and become a floor of zero:
    /// checks back to back, forever, which is the traffic this monitor is
    /// spaced out to avoid. Scheduling the next tick from the end of the pass
    /// makes the interval mean what it says. The cost is drift — passes land
    /// every `interval + pass duration` — which nothing here depends on.
    ///
    /// The leeway is a tenth of the wait: nothing turns on a health check
    /// landing on the second, and a timer that may be coalesced with the
    /// machine's other wakeups is a timer that does not keep a sleeping laptop
    /// awake.
    private func schedule(after delay: TimeInterval, generation: Int) {
        lock.withLock {
            // Stopped, restarted or woken while the last pass ran: that
            // generation no longer owns the cadence and must not schedule for
            // the one that does.
            guard self.generation == generation else { return }
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + delay, leeway: .milliseconds(Int(delay * 100)))
            timer.setEventHandler { [weak self] in
                self?.tick(generation: generation)
            }
            // Stored and resumed under the same lock the handler takes: a
            // zero-delay tick can otherwise run its whole pass and schedule its
            // successor before this assignment lands, leaving us holding — and
            // later cancelling — the wrong timer.
            self.timer = timer
            timer.resume()
        }
    }

    /// The next tick is scheduled after the pass whatever the pass made of
    /// itself — a pass with nothing it could check must not be the end of the
    /// cadence, or configuring the binary later would never start the checks
    /// again. The one exception is a pass that was stopped, and that one takes
    /// care of itself: `schedule` refuses a generation that is no longer
    /// current.
    private func tick(generation: Int) {
        runPass(generation: generation)
        schedule(after: interval, generation: generation)
    }

    // MARK: - One pass

    /// One pass over every account. Runs on `queue`, so passes are serial by
    /// construction and no two logins overlap.
    private func runPass(generation: Int) {
        let (accountIDs, executable) = lock.withLock {
            (self.accountIDs, self.executableName)
        }
        // Without a name, `MCPExecutableLocator` resolves to `/usr/bin/env`
        // with an empty argument, which fails per account with a message about
        // `env` — an unreachable record every fifteen minutes, and every dot
        // red inside the hour, over a cleared text field. Nothing to check is
        // not the same as nothing reachable, and only one of the two belongs in
        // the log.
        guard !executable.isEmpty else { return }

        for accountID in accountIDs {
            // Checked before each account rather than once at the top: the
            // account before this one may have held the pass for half a minute,
            // and by now the app may be quitting.
            guard isCurrent(generation) else { return }
            let result = check(accountID, executable)
            HealthLog.append(
                HealthRecord(
                    accountID: accountID,
                    at: Date(),
                    outcome: result.outcome,
                    source: "periodic",
                    detail: result.detail
                ),
                to: logURL
            )
        }
    }

    private func isCurrent(_ generation: Int) -> Bool {
        lock.withLock { self.generation == generation }
    }

    // MARK: - Waking

    /// Waking is the other moment the stored status is most likely stale, so a
    /// wake restarts the cadence rather than adding a pass to it: an extra pass
    /// would double up with the one the old timer still has pending, and two
    /// passes racing to log the same accounts is the burst of logins this
    /// monitor exists to spread out.
    ///
    /// `queue: nil` delivers the notification on the thread that posted it —
    /// the main thread, since that is where AppKit posts. That is the right
    /// place for a block that only takes a lock and schedules a timer, and it
    /// avoids the case a queue would introduce, where the block is still
    /// waiting to be delivered when `stop()` removes the observer.
    private func observeWake() {
        #if canImport(AppKit)
        stopObservingWake()
        let center = NSWorkspace.shared.notificationCenter
        wakeCenter = center
        wakeObserver = center.addObserver(
            forName: NSWorkspace.didWakeNotification,
            object: nil,
            queue: nil
        ) { [weak self] _ in
            // One weak capture, unwrapped once: the block outlives the monitor
            // it belongs to only until `stop()` removes it, and nothing here
            // should be the reason a stopped monitor stays alive.
            guard let self else { return }
            self.restart(after: Self.wakeDelay)
        }
        #endif
    }

    private func stopObservingWake() {
        #if canImport(AppKit)
        if let wakeObserver {
            wakeCenter?.removeObserver(wakeObserver)
        }
        wakeObserver = nil
        wakeCenter = nil
        #endif
    }
}
