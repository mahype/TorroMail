use security_framework::os::macos::keychain::{KeychainUserInteractionLock, SecKeychain};
use security_framework::passwords;

use crate::sys;

/// `errSecItemNotFound`: nothing is stored under that name.
const ITEM_NOT_FOUND: i32 = -25300;
/// `errSecDuplicateItem`: an item with that name is already there.
const DUPLICATE_ITEM: i32 = -25299;
/// `errSecInteractionNotAllowed`: answering would have needed a dialog —
/// a locked keychain, or an item whose list does not let this binary in.
const INTERACTION_NOT_ALLOWED: i32 = -25308;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The `OSStatus` the Security framework answered with.
    pub code: i32,
    pub message: String,
}

impl Error {
    fn from_status(code: i32) -> Self {
        let message = security_framework::base::Error::from_code(code)
            .message()
            .unwrap_or_else(|| format!("keychain error {code}"));
        Self { code, message }
    }

    /// Whether the keychain could not be asked without a dialog. Usually a
    /// locked keychain, which is a state of the machine rather than of the
    /// item.
    #[must_use]
    pub fn needs_interaction(&self) -> bool {
        self.code == INTERACTION_NOT_ALLOWED
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for Error {}

/// Turns the keychain's dialogs off for this process for as long as the
/// returned lock lives. `None` when the keychain refused; everything still
/// works then, it may just ask.
#[must_use]
pub fn silence_prompts() -> Option<KeychainUserInteractionLock> {
    SecKeychain::disable_user_interaction().ok()
}

/// The Team ID in this process's own code signature, or `None` for an
/// unsigned build. Read rather than hardcoded, so it tracks the signature.
#[must_use]
pub fn team_identifier() -> Option<String> {
    sys::own_team_identifier()
}

/// The stored secret. `Ok(None)` is an answer — nothing is stored under that
/// name — and distinct from not having been able to read it.
pub fn read(service: &str, account: &str) -> Result<Option<String>, Error> {
    match passwords::get_generic_password(service, account) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| Error { code: 0, message: "the stored item is not valid UTF-8".to_owned() }),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(Error::from_status(error.code())),
    }
}

/// Stores a secret, the way the app's `savePassword` does.
///
/// A new item is created with the team-scoped list. An item this build can
/// still read is refreshed in place, leaving its list alone — that list is
/// what lets the other TorroMail binaries in. An item it cannot read is a
/// legacy one whose list names a build that no longer exists; its value is
/// unreachable for good, so it is replaced, because only a fresh add attaches
/// a current list. Never delete-then-add otherwise: a delete that succeeds
/// before an add that fails drops the secret.
pub fn save(service: &str, account: &str, secret: &str) -> Result<(), Error> {
    if add(service, account, secret)? {
        return Ok(());
    }
    if read(service, account).ok().flatten().is_none() {
        // An unsigned build cannot read the team's items either, and whatever
        // it put in their place only it could read. Replacing would take the
        // secret away from the app and the server, so it leaves the item be.
        if team_identifier().is_none() {
            return Err(Error {
                code: DUPLICATE_ITEM,
                message: "this build of TorroMail is not signed, so it may not replace a keychain item \
                          the signed TorroMail programs share"
                    .to_owned(),
            });
        }
        delete(service, account)?;
        return if add(service, account, secret)? { Ok(()) } else { Err(Error::from_status(DUPLICATE_ITEM)) };
    }
    passwords::set_generic_password(service, account, secret.as_bytes()).map_err(|error| Error::from_status(error.code()))
}

/// Removing what is not there succeeds: gone is the state it aims for.
pub fn delete(service: &str, account: &str) -> Result<(), Error> {
    match passwords::delete_generic_password(service, account) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(Error::from_status(error.code())),
    }
}

/// Creates the item. `Ok(false)` when one is already there; the caller then
/// decides whether to refresh or replace it.
fn add(service: &str, account: &str, secret: &str) -> Result<bool, Error> {
    let team = team_identifier();
    match sys::add_generic_password(service, account, secret.as_bytes(), team.as_deref()) {
        0 => Ok(true),
        DUPLICATE_ITEM => Ok(false),
        status => Err(Error::from_status(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_partition_list_is_a_property_list_naming_the_team() {
        let hex = sys::partition_description("ABCDE12345");
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap_or_default())
            .collect();
        let text = String::from_utf8(bytes).unwrap_or_default();
        assert!(text.starts_with("<?xml"), "not an XML property list: {text}");
        assert!(text.contains("<key>Partitions</key>"));
        assert!(text.contains("<string>teamid:ABCDE12345</string>"));
    }

    #[test]
    fn a_test_binary_is_not_signed_by_a_team() {
        assert_eq!(team_identifier(), None);
    }
}
