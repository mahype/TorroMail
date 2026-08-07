//! The per-account search cache: one SQLite database with an FTS5 index.
//!
//! Several `torromail-mcp` processes write here concurrently — every MCP
//! client spawns its own server, and the rebuild command is yet another —
//! so the database runs in WAL mode with a busy timeout, and every write is
//! an idempotent upsert keyed by mailbox and UID. The store never breaks
//! mail access: callers treat every error here as "no cache today".

use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// One honest error type: what went wrong, in words. Callers show it or
/// shrug it off — there is nothing to match on.
#[derive(Debug)]
pub struct CacheError(pub String);

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cache: {}", self.0)
    }
}

impl std::error::Error for CacheError {}

impl From<rusqlite::Error> for CacheError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

/// A handle on one account's cache database.
pub struct CacheStore {
    connection: Connection,
    db_path: PathBuf,
}

impl CacheStore {
    /// Opens (creating the directory, database and schema as needed)
    /// `dir/<account>.sqlite`. The FTS5 probe runs here so a build whose
    /// bundled SQLite lacks FTS5 fails at open, not at the first search.
    pub fn open(dir: &Path, account_id: &str) -> Result<Self, CacheError> {
        std::fs::create_dir_all(dir)
            .map_err(|error| CacheError(format!("cannot create cache directory: {error}")))?;
        let db_path = dir.join(format!("{account_id}.sqlite"));
        let connection = Connection::open(&db_path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "busy_timeout", 5000)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS mailboxes (
                name TEXT PRIMARY KEY,
                uidvalidity INTEGER
            );
            CREATE TABLE IF NOT EXISTS messages (
                mailbox TEXT NOT NULL,
                uid INTEGER NOT NULL,
                subject TEXT NOT NULL DEFAULT '',
                sender TEXT NOT NULL DEFAULT '',
                date TEXT NOT NULL DEFAULT '',
                seen INTEGER NOT NULL DEFAULT 0,
                flagged INTEGER NOT NULL DEFAULT 0,
                body_text TEXT,
                filenames TEXT NOT NULL DEFAULT '',
                fts_rowid INTEGER,
                cached_at INTEGER NOT NULL,
                PRIMARY KEY (mailbox, uid)
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                subject, sender, body, filenames
            );
            CREATE TABLE IF NOT EXISTS attachments (
                mailbox TEXT NOT NULL,
                uid INTEGER NOT NULL,
                attachment_id TEXT NOT NULL,
                filename TEXT NOT NULL,
                media_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                path TEXT,
                cached_at INTEGER NOT NULL,
                PRIMARY KEY (mailbox, uid, attachment_id)
            );",
        )?;
        // The probe: any FTS5 query fails on a build without the module.
        connection.query_row("SELECT count(*) FROM messages_fts", [], |row| {
            row.get::<_, i64>(0)
        })?;
        Ok(Self {
            connection,
            db_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_the_schema_and_fts5_works() {
        let dir =
            std::env::temp_dir().join(format!("torromail-cache-open-{}", std::process::id()));
        let store = CacheStore::open(&dir, "work").expect("store opens");
        // Proves FTS5 was compiled into the bundled SQLite — the crate's
        // whole reason to exist. A second open proves idempotent schema
        // creation.
        drop(store);
        let again = CacheStore::open(&dir, "work").expect("store reopens");
        drop(again);
        std::fs::remove_dir_all(&dir).ok();
    }
}
