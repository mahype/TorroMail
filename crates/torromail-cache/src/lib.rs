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

/// The header facts one message row is keyed and described by.
pub struct SummaryRow<'a> {
    pub mailbox: &'a str,
    pub uid: u32,
    pub subject: &'a str,
    pub sender: &'a str,
    pub date: &'a str,
}

/// One listed attachment, as it rides along with a body fetch.
pub struct AttachmentRow<'a> {
    pub attachment_id: &'a str,
    pub filename: &'a str,
    pub media_type: &'a str,
    pub size_bytes: usize,
}

/// A cached message as reads see it. `body_text: None` means the row was
/// never filled beyond its headers — not an empty body.
pub struct CachedMessage {
    pub mailbox: String,
    pub uid: u32,
    pub subject: String,
    pub sender: String,
    pub date: String,
    pub seen: bool,
    pub flagged: bool,
    pub body_text: Option<String>,
    pub attachments: Vec<CachedAttachment>,
}

pub struct CachedAttachment {
    pub attachment_id: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: usize,
    pub path: Option<String>,
}

/// One local full-text hit, newest first.
pub struct CacheHit {
    pub mailbox: String,
    pub uid: u32,
    pub subject: String,
    pub sender: String,
    pub date: String,
    pub snippet: String,
}

/// A handle on one account's cache database.
pub struct CacheStore {
    connection: Connection,
    db_path: PathBuf,
}

impl CacheStore {
    /// Records the server-stated UIDVALIDITY. A changed value means every
    /// cached UID of that mailbox names a different message now — the rows
    /// are wiped before anything new lands.
    pub fn note_uidvalidity(
        &self,
        mailbox: &str,
        uidvalidity: Option<u32>,
    ) -> Result<(), CacheError> {
        let known: Option<Option<u32>> = self
            .connection
            .query_row(
                "SELECT uidvalidity FROM mailboxes WHERE name = ?1",
                [mailbox],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;

        match known {
            Some(stored) if stored == uidvalidity => Ok(()),
            Some(_) => {
                self.wipe_mailbox(mailbox)?;
                self.connection.execute(
                    "UPDATE mailboxes SET uidvalidity = ?2 WHERE name = ?1",
                    rusqlite::params![mailbox, uidvalidity],
                )?;
                Ok(())
            }
            None => {
                self.connection.execute(
                    "INSERT INTO mailboxes (name, uidvalidity) VALUES (?1, ?2)
                     ON CONFLICT(name) DO UPDATE SET uidvalidity = excluded.uidvalidity",
                    rusqlite::params![mailbox, uidvalidity],
                )?;
                Ok(())
            }
        }
    }

    fn wipe_mailbox(&self, mailbox: &str) -> Result<(), CacheError> {
        self.connection.execute(
            "DELETE FROM messages_fts WHERE rowid IN
                (SELECT fts_rowid FROM messages WHERE mailbox = ?1 AND fts_rowid IS NOT NULL)",
            [mailbox],
        )?;
        self.connection
            .execute("DELETE FROM messages WHERE mailbox = ?1", [mailbox])?;
        self.connection
            .execute("DELETE FROM attachments WHERE mailbox = ?1", [mailbox])?;
        Ok(())
    }

    /// Header facts from a search hit. Keeps an existing row's body and
    /// flags — a summary knows nothing about either.
    pub fn upsert_summary(&self, row: &SummaryRow) -> Result<(), CacheError> {
        self.connection.execute(
            "INSERT INTO messages (mailbox, uid, subject, sender, date, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(mailbox, uid) DO UPDATE SET
                subject = excluded.subject,
                sender = excluded.sender,
                date = excluded.date,
                cached_at = excluded.cached_at",
            rusqlite::params![row.mailbox, row.uid, row.subject, row.sender, row.date, now()],
        )?;
        self.refresh_fts(row.mailbox, row.uid)
    }

    /// A full read: headers, flags, rendered body, and the attachment
    /// listing that rode along with the fetch.
    pub fn upsert_body(
        &self,
        row: &SummaryRow,
        seen: bool,
        flagged: bool,
        body_text: &str,
        attachments: &[AttachmentRow],
    ) -> Result<(), CacheError> {
        let filenames = attachments
            .iter()
            .map(|attachment| attachment.filename)
            .collect::<Vec<_>>()
            .join(" ");
        self.connection.execute(
            "INSERT INTO messages
                (mailbox, uid, subject, sender, date, seen, flagged, body_text, filenames, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(mailbox, uid) DO UPDATE SET
                subject = excluded.subject,
                sender = excluded.sender,
                date = excluded.date,
                seen = excluded.seen,
                flagged = excluded.flagged,
                body_text = excluded.body_text,
                filenames = excluded.filenames,
                cached_at = excluded.cached_at",
            rusqlite::params![
                row.mailbox,
                row.uid,
                row.subject,
                row.sender,
                row.date,
                seen,
                flagged,
                body_text,
                filenames,
                now()
            ],
        )?;
        for attachment in attachments {
            self.connection.execute(
                "INSERT INTO attachments
                    (mailbox, uid, attachment_id, filename, media_type, size_bytes, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(mailbox, uid, attachment_id) DO UPDATE SET
                    filename = excluded.filename,
                    media_type = excluded.media_type,
                    size_bytes = excluded.size_bytes,
                    cached_at = excluded.cached_at",
                rusqlite::params![
                    row.mailbox,
                    row.uid,
                    attachment.attachment_id,
                    attachment.filename,
                    attachment.media_type,
                    attachment.size_bytes,
                    now()
                ],
            )?;
        }
        self.refresh_fts(row.mailbox, row.uid)
    }

    /// Rewrites one row's full-text entry from its current column values.
    fn refresh_fts(&self, mailbox: &str, uid: u32) -> Result<(), CacheError> {
        let existing: Option<i64> = self.connection.query_row(
            "SELECT fts_rowid FROM messages WHERE mailbox = ?1 AND uid = ?2",
            rusqlite::params![mailbox, uid],
            |row| row.get(0),
        )?;
        if let Some(rowid) = existing {
            self.connection
                .execute("DELETE FROM messages_fts WHERE rowid = ?1", [rowid])?;
        }
        self.connection.execute(
            "INSERT INTO messages_fts (subject, sender, body, filenames)
             SELECT subject, sender, coalesce(body_text, ''), filenames
             FROM messages WHERE mailbox = ?1 AND uid = ?2",
            rusqlite::params![mailbox, uid],
        )?;
        let rowid = self.connection.last_insert_rowid();
        self.connection.execute(
            "UPDATE messages SET fts_rowid = ?3 WHERE mailbox = ?1 AND uid = ?2",
            rusqlite::params![mailbox, uid, rowid],
        )?;
        Ok(())
    }

    pub fn get_message(
        &self,
        mailbox: &str,
        uid: u32,
    ) -> Result<Option<CachedMessage>, CacheError> {
        let found = self
            .connection
            .query_row(
                "SELECT subject, sender, date, seen, flagged, body_text
                 FROM messages WHERE mailbox = ?1 AND uid = ?2",
                rusqlite::params![mailbox, uid],
                |row| {
                    Ok(CachedMessage {
                        mailbox: mailbox.to_owned(),
                        uid,
                        subject: row.get(0)?,
                        sender: row.get(1)?,
                        date: row.get(2)?,
                        seen: row.get(3)?,
                        flagged: row.get(4)?,
                        body_text: row.get(5)?,
                        attachments: Vec::new(),
                    })
                },
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        let Some(mut message) = found else {
            return Ok(None);
        };

        let mut rows = self.connection.prepare(
            "SELECT attachment_id, filename, media_type, size_bytes, path
             FROM attachments WHERE mailbox = ?1 AND uid = ?2 ORDER BY attachment_id",
        )?;
        let attachments = rows
            .query_map(rusqlite::params![mailbox, uid], |row| {
                Ok(CachedAttachment {
                    attachment_id: row.get(0)?,
                    filename: row.get(1)?,
                    media_type: row.get(2)?,
                    size_bytes: row.get::<_, i64>(3)?.max(0) as usize,
                    path: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        message.attachments = attachments;
        Ok(Some(message))
    }

    /// Local full-text hits, newest first. A query FTS cannot parse answers
    /// empty — a search cache has no business erroring a search.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<CacheHit>, CacheError> {
        let Some(fts_query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT m.mailbox, m.uid, m.subject, m.sender, m.date,
                    substr(coalesce(m.body_text, ''), 1, 160)
             FROM messages_fts f
             JOIN messages m ON m.fts_rowid = f.rowid
             WHERE messages_fts MATCH ?1
             ORDER BY m.cached_at DESC, m.uid DESC
             LIMIT ?2",
        )?;
        let hits = statement.query_map(rusqlite::params![fts_query, limit as i64], |row| {
            Ok(CacheHit {
                mailbox: row.get(0)?,
                uid: row.get(1)?,
                subject: row.get(2)?,
                sender: row.get(3)?,
                date: row.get(4)?,
                snippet: row.get(5)?,
            })
        });
        match hits {
            Ok(rows) => Ok(rows.collect::<Result<Vec<_>, _>>()?),
            // An unparseable MATCH expression is a caller query, not a bug.
            Err(_) => Ok(Vec::new()),
        }
    }
}

/// Every whitespace token as a quoted prefix term, ANDed — the substring-ish
/// matching people expect from a mail search. `None` for an empty query.
fn fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|token| format!("\"{}\"*", token.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(terms.join(" AND "))
}

/// Seconds since the epoch; the store's only clock.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
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

    #[test]
    fn summaries_bodies_and_search_round_trip() {
        let dir = std::env::temp_dir().join(format!("torromail-cache-rw-{}", std::process::id()));
        let store = CacheStore::open(&dir, "work").expect("store opens");
        store.note_uidvalidity("INBOX", Some(7)).expect("noted");
        let row = SummaryRow {
            mailbox: "INBOX",
            uid: 4711,
            subject: "Rechnung März",
            sender: "billing@example.com",
            date: "Sat, 01 Aug 2026 10:00:00 +0200",
        };
        store.upsert_summary(&row).expect("summary lands");
        // Upsert is idempotent — same key, no duplicate.
        store.upsert_summary(&row).expect("second summary is fine");
        store
            .upsert_body(
                &row,
                true,
                false,
                "Anbei die Rechnung für März.",
                &[AttachmentRow {
                    attachment_id: "2",
                    filename: "rechnung.pdf",
                    media_type: "application/pdf",
                    size_bytes: 8,
                }],
            )
            .expect("body lands");

        let cached = store
            .get_message("INBOX", 4711)
            .expect("read works")
            .expect("row exists");
        assert_eq!(cached.body_text.as_deref(), Some("Anbei die Rechnung für März."));
        assert_eq!(cached.attachments.len(), 1);
        assert!(cached.seen);

        // Subject, body and attachment filename are all searchable.
        for query in ["rechnung märz", "anbei", "rechnung.pdf"] {
            let hits = store.search(query, 10).expect("search works");
            assert_eq!(hits.len(), 1, "query {query:?}");
            assert_eq!(hits[0].uid, 4711);
        }
        assert!(
            store
                .search("nichts dergleichen", 10)
                .expect("search works")
                .is_empty()
        );
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_changed_uidvalidity_invalidates_the_mailbox() {
        let dir = std::env::temp_dir().join(format!("torromail-cache-uv-{}", std::process::id()));
        let store = CacheStore::open(&dir, "work").expect("store opens");
        store.note_uidvalidity("INBOX", Some(7)).expect("noted");
        store
            .upsert_summary(&SummaryRow {
                mailbox: "INBOX",
                uid: 1,
                subject: "Alt",
                sender: "a@example.com",
                date: "",
            })
            .expect("lands");
        // Same value: rows survive. New value: rows go.
        store.note_uidvalidity("INBOX", Some(7)).expect("noted again");
        assert!(store.get_message("INBOX", 1).expect("read").is_some());
        store.note_uidvalidity("INBOX", Some(8)).expect("changed");
        assert!(store.get_message("INBOX", 1).expect("read").is_none());
        assert!(store.search("alt", 10).expect("search").is_empty());
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_connections_write_concurrently() {
        let dir = std::env::temp_dir().join(format!("torromail-cache-wal-{}", std::process::id()));
        let first = CacheStore::open(&dir, "work").expect("first opens");
        let second = CacheStore::open(&dir, "work").expect("second opens");
        first
            .upsert_summary(&SummaryRow {
                mailbox: "INBOX",
                uid: 1,
                subject: "Eins",
                sender: "a@example.com",
                date: "",
            })
            .expect("first writes");
        second
            .upsert_summary(&SummaryRow {
                mailbox: "INBOX",
                uid: 2,
                subject: "Zwei",
                sender: "b@example.com",
                date: "",
            })
            .expect("second writes");
        assert!(first.get_message("INBOX", 2).expect("read").is_some());
        drop((first, second));
        std::fs::remove_dir_all(&dir).ok();
    }
}
