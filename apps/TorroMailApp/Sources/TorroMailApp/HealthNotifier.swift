import Foundation
import UserNotifications

import TorroMailKit

/// Delivers what `HealthLog.transitions` decided. Deciding *whether* an
/// account crossed lives in TorroMailKit, where it is a pure function with
/// tests; this type only knows how to put one on screen and how to ask for
/// permission once.
///
/// It deliberately says nothing about a standing problem. An account that has
/// been broken for a week is red in the window and silent everywhere else — a
/// notification every fifteen minutes would only teach the user that
/// TorroMail's notifications are noise, and the one that finally mattered
/// would be dismissed with the rest.
///
/// Lives in the app target, never in TorroMailKit: `UNUserNotificationCenter`
/// traps in a process without a bundle identifier, and the contract test runs
/// as a bare executable.
@MainActor
final class HealthNotifier: NSObject, UNUserNotificationCenterDelegate {
    /// Each account's broken-ness as of the last reconcile — the "before" half
    /// of a crossing.
    private var known: [String: Bool] = [:]

    /// The authorization request, held as the task that answers it rather than
    /// as the answer. See `authorized()`.
    private var authorization: Task<Bool, Never>?

    /// Whether the baseline has been taken. Until it has, `reconcile` records
    /// and says nothing: this is what makes the wiring order-proof, and it is
    /// needed, because `.task` and an `initial: true` `.onChange` are not
    /// ordered by anything the app controls — and the `.onChange` in fact goes
    /// first, holding the restored accounts, a hundred milliseconds before
    /// `.task` has derived anything.
    private var hasBaseline = false

    /// Takes the baseline a crossing is measured against: what the health log
    /// says at launch, so an account that was already broken when the app quit
    /// does not announce itself again — only a genuine crossing does.
    ///
    /// Deliberately not the accounts as they were *restored*, which is the
    /// tempting reading of the same sentence and is wrong: the state file
    /// stores one `isVerified` flag per account, so a failed account comes
    /// back as merely untested. A baseline taken from that reads every
    /// standing problem as a fresh break, at every launch, forever — the exact
    /// noise this whole feature is arranged to avoid.
    func seed(accounts: [MailAccount]) {
        known = HealthLog.brokenness(accounts: accounts)
        hasBaseline = true
    }

    func reconcile(accounts: [MailAccount]) {
        guard hasBaseline else {
            known = HealthLog.brokenness(accounts: accounts)
            return
        }
        let names = Dictionary(
            accounts.map { ($0.id, $0.name) },
            uniquingKeysWith: { first, _ in first }
        )
        for transition in HealthLog.transitions(previous: known, accounts: accounts) {
            switch transition {
            case let .broke(accountID, reason):
                notify(
                    title: names[accountID] ?? accountID,
                    body: brokenBody(reason: reason),
                    id: Self.brokenID(accountID)
                )
            case let .recovered(accountID):
                // The alarm goes with the all-clear. Left standing in
                // Notification Centre it is a red flag about a problem that is
                // over, and the user cannot tell the two notifications apart
                // by age alone.
                UNUserNotificationCenter.current()
                    .removeDeliveredNotifications(withIdentifiers: [Self.brokenID(accountID)])
                notify(
                    title: names[accountID] ?? accountID,
                    body: L("Reachable again."),
                    id: "health-ok-\(accountID)"
                )
            }
        }
        // Also drops accounts the user removed, which should not keep a slot.
        known = HealthLog.brokenness(accounts: accounts)
    }

    /// One identifier per account and direction, so a second break replaces
    /// the first rather than stacking a pile of them about one account.
    private static func brokenID(_ accountID: String) -> String {
        "health-broken-\(accountID)"
    }

    /// Our sentence first, the server's words after it.
    ///
    /// `reason` is whatever the check recorded, which is usually a mail
    /// server's own English — `credentials rejected: NO [AUTHENTICATIONFAILED]`
    /// — and sometimes a locale-mangled system error. It is not a sentence to
    /// open a notification with, and a notification body is truncated from the
    /// end, so leading with it is how the user ends up reading half a protocol
    /// trace and no statement of what happened.
    ///
    /// Relegating it, rather than dropping it, because some of these are the
    /// only actionable thing there is: Gmail answers a plain password with
    /// "Application-specific password required:" and a link to the page that
    /// fixes it. Losing that to keep the body tidy would cost the user the
    /// answer to make room for the question.
    private func brokenBody(reason: String) -> String {
        let sentence = L("TorroMail can no longer reach this account.")
        let detail = reason.trimmingCharacters(in: .whitespacesAndNewlines)
        return detail.isEmpty ? sentence : "\(sentence)\n\(detail)"
    }

    private func notify(title: String, body: String, id: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        let request = UNNotificationRequest(identifier: id, content: content, trigger: nil)
        Task { @MainActor in
            guard await authorized() else { return }
            // Best-effort, like the health log itself: the account is red in
            // the window either way, and nothing the user does should fail
            // because a banner did not make it to the screen.
            try? await UNUserNotificationCenter.current().add(request)
        }
    }

    /// Whether the user agreed to be told. Asked once, on the first crossing
    /// there is something to say about — not at launch, where a permission
    /// sheet would greet somebody who has not done anything yet and cannot
    /// know what they are agreeing to. A refusal is final: the app falls back
    /// to its own status display and never asks again.
    ///
    /// Held as the task that answers rather than as a `Bool`, and every
    /// notification awaits it, because the sheet stays up for as long as the
    /// user takes to read it: against a flag, the first crossing this install
    /// ever sees — the one that raised the sheet, and the one most worth
    /// hearing — would be dropped while it was still on screen. Awaiting is
    /// not a queue of pending notifications: the content is already built, all
    /// callers wait on the same single decision, and a refusal resolves every
    /// one of them to `false` at once and for good.
    private func authorized() async -> Bool {
        if let authorization { return await authorization.value }
        let task = Task { @MainActor () -> Bool in
            let center = UNUserNotificationCenter.current()
            // Set here rather than at launch for the same reason as the
            // request itself: `current()` is the call that needs a bundle, and
            // this is the first moment the app has any use for it.
            center.delegate = self
            return (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
        }
        authorization = task
        return await task.value
    }

    /// macOS drops a banner for the frontmost app unless the delegate asks for
    /// it, and the frontmost app is very often TorroMail: the account list, the
    /// window the user opened *because* something looked wrong, is exactly
    /// where a break must not go unmentioned.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound])
    }
}
