//! The terminal control surface. Like the macOS app it is a place to set
//! things up and see what assistants did — never a place to read mail.
//!
//! `data` loads a snapshot of everything on disk, `app` holds what the user is
//! looking at and reacts to keys, `ui` draws both. Nothing in `ui` touches the
//! file system, so every screen can be rendered in a test from a snapshot.

pub mod app;
pub mod data;
pub mod i18n;
pub mod theme;
pub mod ui;
