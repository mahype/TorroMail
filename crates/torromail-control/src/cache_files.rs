//! What one account keeps on this machine besides its configuration: the
//! SQLite cache with its WAL siblings, and the attachments retained for it.
//! One list, so measuring and removing can never disagree about what counts.

use std::path::{Path, PathBuf};

/// Every path that belongs to the account's local footprint, whether or not
/// it exists. Empty for an id that is not a plain name — such an id must
/// never become part of a path.
#[must_use]
pub fn paths_for(data_directory: &Path, account_id: &str) -> Vec<PathBuf> {
    if !crate::account::is_plain_id(account_id) {
        return Vec::new();
    }
    let cache = data_directory.join("cache");
    vec![
        cache.join(format!("{account_id}.sqlite")),
        cache.join(format!("{account_id}.sqlite-wal")),
        cache.join(format!("{account_id}.sqlite-shm")),
        data_directory.join("attachments").join(account_id),
    ]
}

/// The real on-disk footprint — measured, not estimated.
#[must_use]
pub fn size_bytes(data_directory: &Path, account_id: &str) -> u64 {
    paths_for(data_directory, account_id).iter().map(|path| size_of(path, 0)).sum()
}

fn size_of(path: &Path, depth: usize) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    // Attachments nest a level or two; anything deeper is not ours to walk.
    if !metadata.is_dir() || depth > 4 {
        return 0;
    }
    std::fs::read_dir(path)
        .map(|entries| entries.flatten().map(|entry| size_of(&entry.path(), depth + 1)).sum())
        .unwrap_or(0)
}

/// "12 KB", "3.4 MB" — decimal units, as file managers show them.
#[must_use]
pub fn label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if value >= 100.0 { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}
