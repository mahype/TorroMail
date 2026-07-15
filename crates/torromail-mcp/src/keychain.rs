//! Resolving `SecretRef`s from the macOS keychain.
//!
//! The app stores passwords under service `TorroMail`; the document only
//! ever carries the reference. On first access macOS asks the user once to
//! allow `torromail-mcp` — the standard pattern until code signing makes it
//! promptless. The secret never touches disk, environment, or logs.

use torromail_core::{CoreError, CoreResult, SecretRef};

pub(crate) fn resolve_secret(secret_ref: &SecretRef) -> CoreResult<String> {
    let (service, account) = secret_ref.keychain_location().ok_or_else(|| {
        CoreError::ProviderFailure("unsupported secret reference scheme".to_owned())
    })?;
    lookup(service, account)
}

#[cfg(target_os = "macos")]
fn lookup(service: &str, account: &str) -> CoreResult<String> {
    let bytes = security_framework::passwords::get_generic_password(service, account)
        .map_err(|error| CoreError::ProviderFailure(format!("keychain lookup failed: {error}")))?;
    String::from_utf8(bytes)
        .map_err(|_| CoreError::ProviderFailure("secret is not valid UTF-8".to_owned()))
}

#[cfg(not(target_os = "macos"))]
fn lookup(_service: &str, _account: &str) -> CoreResult<String> {
    Err(CoreError::ProviderFailure(
        "keychain secrets are only available on macOS".to_owned(),
    ))
}
