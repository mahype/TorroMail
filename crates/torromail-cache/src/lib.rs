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

/// What the app and `mail_get_cache_status` report: real counts, real bytes.
pub struct CacheStatus {
    pub message_count: u64,
    pub attachment_count: u64,
    pub db_size_bytes: u64,
    pub last_write: Option<u64>,
}

impl CacheStore {
    /// Marks one listed attachment as downloaded-and-retained at `path` —
    /// the next download answers from disk instead of the server. Carries
    /// the payload facts too, so a download without a prior body read still
    /// leaves an honest row.
    pub fn record_attachment_file(
        &self,
        mailbox: &str,
        uid: u32,
        attachment: &AttachmentRow,
        path: &str,
    ) -> Result<(), CacheError> {
        self.connection.execute(
            "INSERT INTO attachments
                (mailbox, uid, attachment_id, filename, media_type, size_bytes, path, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(mailbox, uid, attachment_id) DO UPDATE SET
                filename = excluded.filename,
                media_type = excluded.media_type,
                size_bytes = excluded.size_bytes,
                path = excluded.path,
                cached_at = excluded.cached_at",
            rusqlite::params![
                mailbox,
                uid,
                attachment.attachment_id,
                attachment.filename,
                attachment.media_type,
                attachment.size_bytes,
                path,
                now()
            ],
        )?;
        Ok(())
    }

    /// The retained attachment's row, when a download was recorded and its
    /// file still exists — filename and media type ride along so a cache hit
    /// can answer without the server.
    pub fn retained_attachment(
        &self,
        mailbox: &str,
        uid: u32,
        attachment_id: &str,
    ) -> Result<Option<CachedAttachment>, CacheError> {
        let found = self
            .connection
            .query_row(
                "SELECT filename, media_type, size_bytes, path FROM attachments
                 WHERE mailbox = ?1 AND uid = ?2 AND attachment_id = ?3",
                rusqlite::params![mailbox, uid, attachment_id],
                |row| {
                    Ok(CachedAttachment {
                        attachment_id: attachment_id.to_owned(),
                        filename: row.get(0)?,
                        media_type: row.get(1)?,
                        size_bytes: row.get::<_, i64>(2)?.max(0) as usize,
                        path: row.get(3)?,
                    })
                },
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(found.filter(|attachment| {
            attachment
                .path
                .as_deref()
                .is_some_and(|path| Path::new(path).exists())
        }))
    }

    /// The retained file's path, when this attachment was downloaded before
    /// and the file is still there. A recorded path whose file vanished
    /// answers `None` — the disk is the truth, the row only remembers.
    pub fn attachment_path(
        &self,
        mailbox: &str,
        uid: u32,
        attachment_id: &str,
    ) -> Result<Option<String>, CacheError> {
        let path: Option<String> = self
            .connection
            .query_row(
                "SELECT path FROM attachments
                 WHERE mailbox = ?1 AND uid = ?2 AND attachment_id = ?3",
                rusqlite::params![mailbox, uid, attachment_id],
                |row| row.get(0),
            )
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(path.filter(|candidate| Path::new(candidate).exists()))
    }

    /// Drops everything above `level`: retained files and attachment rows
    /// below `Attachments`, bodies below `Bodies`, everything at `Off`.
    /// File deletion is best-effort — a locked file must not wedge a trim.
    pub fn trim(&self, level: torromail_core::CacheLevel) -> Result<(), CacheError> {
        use torromail_core::CacheLevel;

        if level < CacheLevel::Attachments {
            let mut statement = self
                .connection
                .prepare("SELECT path FROM attachments WHERE path IS NOT NULL")?;
            let retained = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for path in retained {
                let _ = std::fs::remove_file(&path);
            }
            drop(statement);
            self.connection.execute("DELETE FROM attachments", [])?;
        }
        if level < CacheLevel::Bodies {
            self.connection.execute(
                "UPDATE messages SET body_text = NULL, filenames = ''",
                [],
            )?;
            // The index must forget the bodies too.
            let mut statement = self
                .connection
                .prepare("SELECT mailbox, uid FROM messages")?;
            let keys = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for (mailbox, uid) in keys {
                self.refresh_fts(&mailbox, uid)?;
            }
        }
        if level < CacheLevel::Headers {
            self.connection.execute("DELETE FROM messages_fts", [])?;
            self.connection.execute("DELETE FROM messages", [])?;
            self.connection.execute("DELETE FROM mailboxes", [])?;
        }
        Ok(())
    }

    /// Honest numbers for the status tool and the app's storage row.
    pub fn status(&self) -> Result<CacheStatus, CacheError> {
        let message_count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))?;
        let attachment_count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM attachments", [], |row| row.get(0))?;
        let last_write: Option<i64> = self.connection.query_row(
            "SELECT max(cached_at) FROM messages",
            [],
            |row| row.get(0),
        )?;

        // The WAL sibling holds real bytes until a checkpoint folds it in.
        let mut db_size_bytes = 0u64;
        for candidate in [
            self.db_path.clone(),
            PathBuf::from(format!("{}-wal", self.db_path.display())),
        ] {
            if let Ok(metadata) = std::fs::metadata(&candidate) {
                db_size_bytes += metadata.len();
            }
        }

        Ok(CacheStatus {
            message_count: message_count.max(0) as u64,
            attachment_count: attachment_count.max(0) as u64,
            db_size_bytes,
            last_write: last_write.and_then(|stamp| u64::try_from(stamp).ok()),
        })
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

    #[test]
    fn retained_attachments_trim_and_status_agree() {
        let dir = std::env::temp_dir().join(format!("torromail-cache-att-{}", std::process::id()));
        let store = CacheStore::open(&dir, "work").expect("store opens");
        let row = SummaryRow {
            mailbox: "INBOX",
            uid: 1,
            subject: "S",
            sender: "a@example.com",
            date: "",
        };
        store
            .upsert_body(
                &row,
                false,
                false,
                "Text",
                &[AttachmentRow {
                    attachment_id: "2",
                    filename: "a.pdf",
                    media_type: "application/pdf",
                    size_bytes: 1,
                }],
            )
            .expect("body lands");

        // Retain a real file so trim can delete it.
        let blob = dir.join("blob-a.pdf");
        std::fs::write(&blob, b"x").expect("blob written");
        store
            .record_attachment_file(
                "INBOX",
                1,
                &AttachmentRow {
                    attachment_id: "2",
                    filename: "a.pdf",
                    media_type: "application/pdf",
                    size_bytes: 1,
                },
                &blob.to_string_lossy(),
            )
            .expect("recorded");
        assert_eq!(
            store
                .attachment_path("INBOX", 1, "2")
                .expect("lookup")
                .as_deref(),
            Some(blob.to_string_lossy().as_ref())
        );

        let status = store.status().expect("status");
        assert_eq!(status.message_count, 1);
        assert_eq!(status.attachment_count, 1);
        assert!(status.db_size_bytes > 0);
        assert!(status.last_write.is_some());

        // Trim to Bodies: attachment rows and the file go, the body stays.
        store
            .trim(torromail_core::CacheLevel::Bodies)
            .expect("trimmed");
        assert!(store.attachment_path("INBOX", 1, "2").expect("lookup").is_none());
        assert!(!blob.exists());
        assert!(
            store
                .get_message("INBOX", 1)
                .expect("read")
                .expect("row")
                .body_text
                .is_some()
        );

        // Trim to Headers: the body goes too, the summary stays searchable.
        store
            .trim(torromail_core::CacheLevel::Headers)
            .expect("trimmed");
        assert!(
            store
                .get_message("INBOX", 1)
                .expect("read")
                .expect("row")
                .body_text
                .is_none()
        );
        assert_eq!(store.search("s", 10).expect("search").len(), 1);

        // Trim to Off: nothing left.
        store
            .trim(torromail_core::CacheLevel::Off)
            .expect("trimmed");
        assert_eq!(store.status().expect("status").message_count, 0);
        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }
}
