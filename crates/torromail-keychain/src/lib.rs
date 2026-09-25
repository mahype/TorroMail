//! The macOS keychain, used the way the TorroMail app uses it.
//!
//! A fresh item gets an access list whose application list is empty ("any
//! application") and whose partition list names the signing Team ID. The
//! partition is the gate macOS actually enforces, so every binary signed by
//! that team — the app, the server it bundles, the terminal surface — reads
//! and writes the item without a dialog, and a rebuild keeps working because
//! the grant is keyed to the team, not to one binary's hash. This mirrors
//! `KeychainStore.teamScopedAccess()` in the app; an item either side creates
//! is one the other can read.
//!
//! An unsigned build has no team to scope to and stores its items with the
//! keychain's default list, which names only itself. Those work for that build
//! alone — good enough for a local `cargo run`, useless for sharing.
//!
//! Nothing here asks the user. [`silence_prompts`] turns the keychain's
//! dialogs off for the whole process; a read that would need one fails
//! instead, and the callers treat that as "this secret has to be entered
//! again", which is what it means.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod sys;

#[cfg(target_os = "macos")]
pub use macos::*;
