//! Resolving `SecretRef`s from the macOS keychain.
//!
//! The app stores passwords under service `TorroMail`; the document only
//! ever carries the reference. On first access macOS asks the user once to
//! allow `torromail-mcp` — the standard pattern until code signing makes it
//! promptless. The secret never touches disk, environment, or logs.
//!
//! The module is public for one reason: [`unreadable_secret`] is a ruling
//! about account health, and a ruling that only a real keychain can provoke is
//! a ruling nothing pins.

use torromail_core::{CoreError, CoreResult, SecretRef};

/// What a failed keychain **read** means. Pure and platform-free on purpose:
/// the call that fails exists only on macOS, but the verdict about it holds
/// everywhere and is therefore testable without a keychain to break.
///
/// `CredentialRejected`, not `ProviderFailure` — the health log turns the
/// first into a red dot at once and grants the second three strikes and a
/// grace period first, and this failure has no business waiting. An item this
/// process cannot read it cannot read later either: keychain items are scoped
/// to the signing Team ID when they are created and refreshed in place
/// afterwards, so an item an older build wrote stays out of reach for good,
/// and the consent dialog that might have rescued the read is disabled in this
/// process by design. Nothing here heals on its own, which is the only thing
/// the grace period is for. Meanwhile the repair is real and the user's to
/// make: `savePassword` replaces an item it cannot read rather than updating
/// it, so re-entering the password in the app genuinely fixes this.
///
/// An earlier reading had it the other way round — that a secret we could not
/// read "can never accuse the password", so it must be a reachability problem.
/// That confuses *whose* password is at issue. Nobody is claiming the mailbox
/// refused the login; the claim is that the account needs the user, which is
/// exactly what the accusing bucket exists to say. Filed as unreachable it
/// instead spends 45 minutes silent and then blames the server.
///
/// `cause` lands last and in parentheses because the Security framework's own
/// sentence for this is "the user name or password you entered is not valid",
/// localised into the OS language — a sentence about the *keychain item* that
/// reads as an accusation against the mailbox password it is not about. It is
/// kept for whoever debugs this; it must never be the instruction, and this
/// string reaches the app's UI and `health.jsonl` verbatim.
#[must_use]
pub fn unreadable_secret(cause: &str) -> CoreError {
    CoreError::CredentialRejected(format!(
        "enter this account's password again in TorroMail — \
         the stored one could not be read from the keychain ({cause})"
    ))
}

/// Which keychain error it was does not change the answer, so this asks. macOS
/// does distinguish them — `security_framework` carries the `OSStatus`, so
/// `errSecItemNotFound`, `errSecAuthFailed` and `errSecInteractionNotAllowed`
/// are all tellable apart — but every one of them ends at the same sentence:
/// there is no secret this process can use, and the user has to supply one.
/// `errSecUserCanceled` cannot even arise, since `main` turns off keychain
/// user interaction before anything reads. Branching would buy a different
/// diagnostic for an identical repair.
pub(crate) fn resolve_secret(secret_ref: &SecretRef) -> CoreResult<String> {
    let (service, account) = secret_ref.keychain_location().ok_or_else(unsupported_scheme)?;
    lookup(service, account)
}

/// Writes a secret back. Only OAuth needs this: a renewed access token is
/// worth nothing if the next process has to fetch it again, and the refresh
/// token itself rotates on some providers — losing that would lock the
/// account out for good.
///
/// A failed write stays a `ProviderFailure`, and not by omission. By the time
/// this runs the same item has already been read successfully and a fresh
/// token has already been obtained, so the credentials are demonstrably fine
/// and the Team ID scoping that makes reads permanent does not apply — what
/// failed is the write alone, which is the transient kind. Reporting it as a
/// credential problem would tell the user to re-enter a password that is
/// correct, and for an OAuth account a password is not even the thing they
/// hold.
pub(crate) fn store_secret(secret_ref: &SecretRef, secret: &str) -> CoreResult<()> {
    let (service, account) = secret_ref.keychain_location().ok_or_else(unsupported_scheme)?;
    store(service, account, secret)
}

/// A `SecretRef` in a scheme no resolver knows. Nothing the user did causes
/// this and nothing they can do fixes it — the app writes the reference, so a
/// wrong one is our bug. `ProviderFailure` keeps it out of the bucket that
/// fires a desktop notification about a password that is not the problem.
fn unsupported_scheme() -> CoreError {
    CoreError::ProviderFailure("unsupported secret reference scheme".to_owned())
}

#[cfg(target_os = "macos")]
fn lookup(service: &str, account: &str) -> CoreResult<String> {
    let bytes = security_framework::passwords::get_generic_password(service, account)
        .map_err(|error| unreadable_secret(&error.to_string()))?;
    // A stored item that is not text is unusable and will stay unusable, and
    // the repair is the same re-entry — so it is the same ruling, not a
    // neighbouring one. Only the diagnostic differs.
    String::from_utf8(bytes).map_err(|_| unreadable_secret("the stored item is not valid UTF-8"))
}

#[cfg(target_os = "macos")]
fn store(service: &str, account: &str, secret: &str) -> CoreResult<()> {
    security_framework::passwords::set_generic_password(service, account, secret.as_bytes())
        .map_err(|error| CoreError::ProviderFailure(format!("keychain write failed: {error}")))
}

/// Everywhere else the desktop's Secret Service holds the secrets, under the
/// same service and account names the macOS keychain uses.
///
/// Two failures that look alike and are not. A secret that is *not there* is
/// the macOS ruling unchanged: nothing heals it, the user has to enter the
/// password, so it is `CredentialRejected` and the dot goes red at once. A
/// service that *cannot be reached* — no session bus, a keyring nobody has
/// unlocked yet — is a state of the machine, often a passing one right after
/// login, and says nothing about the account: that stays `ProviderFailure`,
/// which the health log grants its three strikes.
#[cfg(not(target_os = "macos"))]
fn lookup(service: &str, account: &str) -> CoreResult<String> {
    match torromail_control::secrets::platform_store().get(service, account) {
        Ok(Some(secret)) => Ok(secret),
        Ok(None) => Err(unreadable_secret("nothing is stored for this account")),
        Err(error) => Err(CoreError::ProviderFailure(error.to_string())),
    }
}

#[cfg(not(target_os = "macos"))]
fn store(service: &str, account: &str, secret: &str) -> CoreResult<()> {
    torromail_control::secrets::platform_store()
        .set(service, account, secret)
        .map_err(|error| CoreError::ProviderFailure(format!("secret write failed: {error}")))
}
