# Real Search Cache — Implementation Plan (Spec Phase 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The cache section stops being a facade: one level picker in the
permissions' language backed by a real SQLite/FTS5 store per account, filled
write-through, searchable, rebuildable, trimmable, with an honest storage
figure.

**Architecture:** A new `torromail-cache` crate owns SQLite (rusqlite,
bundled, WAL). `torromail-core` swaps its five `CachePolicy` booleans for a
`CacheLevel` enum and learns each mailbox's UIDVALIDITY. `torromail-mcp`
opens one store per configured account, feeds it from the tool paths, merges
local FTS hits into `mail_search`, serves cache hits, and grows
`--rebuild-cache` / `--trim-cache` commands. The Swift app migrates its
settings to the single level, publishes `cache: {level}`, and gets a working
storage row with rebuild progress. Spec:
`docs/superpowers/specs/2026-08-07-attachments-and-real-cache-design.md`.

**Tech Stack:** Rust + rusqlite (bundled SQLite with FTS5) — the first
external dependency, deliberately quarantined in its own crate; SwiftUI.

## Global Constraints

- Workspace lints stay binding: no `unwrap`, no `todo!`, no unsafe.
- `torromail-core` keeps zero external dependencies — rusqlite lives only in
  `crates/torromail-cache`.
- Levels and JSON values: `off` / `headers` / `bodies` / `attachments`.
  German labels: Aus / Betreff & Absender / Ganze E-Mails / E-Mails &
  Anhänge. Effective level = min(chosen, account read access).
- Legacy mapping (Swift decode AND server-side policy parsing):
  `local_cache_enabled: false` → `off`; modes `metadata`/`headers` →
  `headers`; `body`/`fullText` → `bodies`; index booleans ignored.
- Only configured accounts (real connection facts) are cached; fixture
  accounts never touch a store. The empty browse query stays a pure provider
  listing.
- Cache DB: `~/Library/Application Support/TorroMail/cache/<account>.sqlite`
  (WAL + busy_timeout 5000 ms); attachment blobs stay in the phase-1
  `attachments/` tree, the DB holds rows about them.
- Rebuild defaults: newest 500 header rows per readable mailbox, newest 100
  INBOX bodies when the effective level allows; attachments never prefetched.
- Verification: `cargo test`, `cargo build -p torromail-mcp`, both Swift
  commands from AGENTS.md.
- Commits: `feat(cache):` / `feat(core):` / `feat(mcp):` / `feat(app):` /
  `docs:` + the Co-Authored-By line.

---

### Task 1: Crate scaffold — `torromail-cache` with schema and FTS5 proof

**Files:**
- Create: `crates/torromail-cache/Cargo.toml`, `crates/torromail-cache/src/lib.rs`
- Modify: `Cargo.toml` (workspace members), `crates/torromail-mcp/Cargo.toml`
  (dependency added here already so later tasks only use it)

**Interfaces (produced, all `pub`):**

```rust
pub enum CacheLevel { Off, Headers, Bodies, Attachments }   // Ord: Off < … < Attachments
impl CacheLevel {
    pub fn as_str(self) -> &'static str;                     // "off" | "headers" | "bodies" | "attachments"
    pub fn parse(text: &str) -> Option<CacheLevel>;
    pub fn parse_legacy(local_cache_enabled: bool, mode: &str) -> CacheLevel;
}
pub struct CacheStore { … }
impl CacheStore {
    /// Opens (creating schema and parent dir as needed) `dir/<account>.sqlite`.
    pub fn open(dir: &std::path::Path, account_id: &str) -> Result<CacheStore, CacheError>;
}
pub struct CacheError(pub String);                           // Display; one honest error type
```

`CacheLevel` lives HERE (not in core) so core stays dependency-free and the
MCP layer has one authority; core's own enum in Task 4 re-implements the same
four values for the account model and maps 1:1 (documented in both places).
Wait — two enums for one concept is drift waiting to happen. Instead: core
defines `CacheLevel` (it has no deps to fear), and `torromail-cache` depends
on `torromail-core` and uses core's enum. That is the layering every other
type already follows. `parse`/`parse_legacy`/`as_str` live on core's enum.

**Corrected interface split:** `torromail-core` (Task 1 includes this small
core edit): `CacheLevel` enum + `as_str` + `parse` + `parse_legacy` +
`Ord`; `torromail-cache`: `CacheStore`, `CacheError`.

Schema (executed in `open`):

```sql
CREATE TABLE IF NOT EXISTS mailboxes (
  name TEXT PRIMARY KEY,
  uidvalidity INTEGER            -- NULL = server never said; NULL matches NULL
);
CREATE TABLE IF NOT EXISTS messages (
  mailbox TEXT NOT NULL,
  uid INTEGER NOT NULL,
  subject TEXT NOT NULL DEFAULT '',
  sender TEXT NOT NULL DEFAULT '',
  date TEXT NOT NULL DEFAULT '',
  seen INTEGER NOT NULL DEFAULT 0,
  flagged INTEGER NOT NULL DEFAULT 0,
  body_text TEXT,                -- NULL until a body was fetched at level >= bodies
  filenames TEXT NOT NULL DEFAULT '',  -- space-joined attachment filenames, for FTS
  cached_at INTEGER NOT NULL,
  PRIMARY KEY (mailbox, uid)
);
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
  subject, sender, body, filenames,
  content=''                     -- contentless; we insert/delete explicitly
);
CREATE TABLE IF NOT EXISTS attachments (
  mailbox TEXT NOT NULL,
  uid INTEGER NOT NULL,
  attachment_id TEXT NOT NULL,
  filename TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  path TEXT,                     -- NULL until downloaded and retained
  cached_at INTEGER NOT NULL,
  PRIMARY KEY (mailbox, uid, attachment_id)
);
```

Contentless FTS5 note: deletes from a `content=''` table need
`INSERT INTO messages_fts(messages_fts, rowid, …) VALUES('delete', …)` with
the original column values — which we do not keep. Therefore use a normal
(contentful) FTS5 table instead: `USING fts5(subject, sender, body, filenames)`
and store the strings twice; the duplication is bytes, not bugs, and delete
by rowid works. The `messages` row keeps `fts_rowid INTEGER` referencing it.

**Final schema decision (binding for later tasks):** contentful FTS5 table,
`messages.fts_rowid INTEGER` column added, all FTS writes go through helper
`fn refresh_fts(connection, mailbox, uid)` which deletes any old rowid and
inserts the current row's values.

- [ ] **Step 1: Failing test** (`crates/torromail-cache/src/lib.rs`, `mod tests`)

```rust
#[test]
fn open_creates_the_schema_and_fts5_works() {
    let dir = std::env::temp_dir().join(format!("torromail-cache-open-{}", std::process::id()));
    let store = CacheStore::open(&dir, "work").expect("store opens");
    // Proves FTS5 was compiled into the bundled SQLite — the crate's whole
    // reason to exist. A second open proves idempotent schema creation.
    drop(store);
    let again = CacheStore::open(&dir, "work").expect("store reopens");
    drop(again);
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2:** `cargo test -p torromail-cache` → fails (crate/test missing).
- [ ] **Step 3:** Create the crate (workspace member, `rusqlite = { version = "0.32", features = ["bundled"] }`, `torromail-core` path dep), `CacheLevel` in core with its three functions and unit tests there, `CacheStore::open` with `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;` and the schema (including one `SELECT count(*) FROM messages_fts` probe in `open` so a build without FTS5 fails loudly at open, not at first search).
- [ ] **Step 4:** `cargo test -p torromail-cache -p torromail-core` → green.
- [ ] **Step 5:** Commit `feat(cache): SQLite store scaffold with FTS5 and cache levels`.

---

### Task 2: Store writes and reads — summaries, bodies, UIDVALIDITY, search

**Files:** `crates/torromail-cache/src/lib.rs`

**Interfaces:**

```rust
pub struct SummaryRow<'a> { pub mailbox: &'a str, pub uid: u32, pub subject: &'a str, pub sender: &'a str, pub date: &'a str }
pub struct AttachmentRow<'a> { pub attachment_id: &'a str, pub filename: &'a str, pub media_type: &'a str, pub size_bytes: usize }
pub struct CachedMessage { pub mailbox: String, pub uid: u32, pub subject: String, pub sender: String, pub date: String, pub seen: bool, pub flagged: bool, pub body_text: Option<String>, pub attachments: Vec<CachedAttachment> }
pub struct CachedAttachment { pub attachment_id: String, pub filename: String, pub media_type: String, pub size_bytes: usize, pub path: Option<String> }
pub struct CacheHit { pub mailbox: String, pub uid: u32, pub subject: String, pub sender: String, pub date: String, pub snippet: String }

impl CacheStore {
    /// Records the server-stated UIDVALIDITY; a changed value wipes the
    /// mailbox's rows (messages, fts, attachments) before anything new lands.
    pub fn note_uidvalidity(&self, mailbox: &str, uidvalidity: Option<u32>) -> Result<(), CacheError>;
    pub fn upsert_summary(&self, row: &SummaryRow) -> Result<(), CacheError>;      // keeps body/flags if present
    pub fn upsert_body(&self, row: &SummaryRow, seen: bool, flagged: bool, body_text: &str, attachments: &[AttachmentRow]) -> Result<(), CacheError>;
    pub fn get_message(&self, mailbox: &str, uid: u32) -> Result<Option<CachedMessage>, CacheError>;
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<CacheHit>, CacheError>;   // FTS match, newest first
}
```

FTS query construction: each whitespace token becomes `"token"*` (quoted
prefix search), tokens joined with `AND` — mirrors the substring-ish matching
users expect; a query that FTS rejects answers `Ok(vec![])`, never an error.

- [ ] **Step 1: Failing tests** — three tests in `mod tests`:

```rust
#[test]
fn summaries_bodies_and_search_round_trip() {
    let dir = std::env::temp_dir().join(format!("torromail-cache-rw-{}", std::process::id()));
    let store = CacheStore::open(&dir, "work").expect("store opens");
    store.note_uidvalidity("INBOX", Some(7)).expect("noted");
    let row = SummaryRow { mailbox: "INBOX", uid: 4711, subject: "Rechnung März", sender: "billing@example.com", date: "Sat, 01 Aug 2026 10:00:00 +0200" };
    store.upsert_summary(&row).expect("summary lands");
    // Upsert is idempotent — same key, no duplicate.
    store.upsert_summary(&row).expect("second summary is fine");
    store.upsert_body(&row, true, false, "Anbei die Rechnung für März.", &[AttachmentRow { attachment_id: "2", filename: "rechnung.pdf", media_type: "application/pdf", size_bytes: 8 }]).expect("body lands");

    let cached = store.get_message("INBOX", 4711).expect("read works").expect("row exists");
    assert_eq!(cached.body_text.as_deref(), Some("Anbei die Rechnung für März."));
    assert_eq!(cached.attachments.len(), 1);
    assert!(cached.seen);

    // Subject, body and attachment filename are all searchable.
    for query in ["rechnung märz", "anbei", "rechnung.pdf"] {
        let hits = store.search(query, 10).expect("search works");
        assert_eq!(hits.len(), 1, "query {query:?}");
        assert_eq!(hits[0].uid, 4711);
    }
    assert!(store.search("nichts dergleichen", 10).expect("search works").is_empty());
    drop(store);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_changed_uidvalidity_invalidates_the_mailbox() {
    let dir = std::env::temp_dir().join(format!("torromail-cache-uv-{}", std::process::id()));
    let store = CacheStore::open(&dir, "work").expect("store opens");
    store.note_uidvalidity("INBOX", Some(7)).expect("noted");
    store.upsert_summary(&SummaryRow { mailbox: "INBOX", uid: 1, subject: "Alt", sender: "a@example.com", date: "" }).expect("lands");
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
    first.upsert_summary(&SummaryRow { mailbox: "INBOX", uid: 1, subject: "Eins", sender: "a@example.com", date: "" }).expect("first writes");
    second.upsert_summary(&SummaryRow { mailbox: "INBOX", uid: 2, subject: "Zwei", sender: "b@example.com", date: "" }).expect("second writes");
    assert!(first.get_message("INBOX", 2).expect("read").is_some());
    drop((first, second));
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2:** run → compile failure. **Step 3:** implement (transactions
  per call; `refresh_fts` helper as decided in Task 1; `upsert_summary` uses
  `INSERT … ON CONFLICT(mailbox,uid) DO UPDATE` touching only
  subject/sender/date/cached_at so a body row keeps its body; newest-first =
  `ORDER BY cached_at DESC, uid DESC`). **Step 4:** green. **Step 5:** commit
  `feat(cache): write-through rows, UIDVALIDITY invalidation, FTS search`.

---

### Task 3: Store attachments, trim, status

**Files:** `crates/torromail-cache/src/lib.rs`

**Interfaces:**

```rust
pub struct CacheStatus { pub message_count: u64, pub attachment_count: u64, pub db_size_bytes: u64, pub last_write: Option<u64> }
impl CacheStore {
    /// Marks one listed attachment as downloaded-and-retained at `path`.
    pub fn record_attachment_file(&self, mailbox: &str, uid: u32, attachment_id: &str, path: &str) -> Result<(), CacheError>;
    /// The retained path, if this attachment was downloaded before.
    pub fn attachment_path(&self, mailbox: &str, uid: u32, attachment_id: &str) -> Result<Option<String>, CacheError>;
    /// Drops everything above `level`: bodies below `Bodies`, attachment rows
    /// (and their files on disk) below `Attachments`, everything at `Off`.
    pub fn trim(&self, level: torromail_core::CacheLevel) -> Result<(), CacheError>;
    pub fn status(&self) -> Result<CacheStatus, CacheError>;
    /// Timestamp source for cached_at/last_write — seconds since epoch.
    /// (Internal; `status` reports the max cached_at.)
}
```

- [ ] **Step 1: Failing test:**

```rust
#[test]
fn retained_attachments_trim_and_status_agree() {
    let dir = std::env::temp_dir().join(format!("torromail-cache-att-{}", std::process::id()));
    let store = CacheStore::open(&dir, "work").expect("store opens");
    let row = SummaryRow { mailbox: "INBOX", uid: 1, subject: "S", sender: "a@example.com", date: "" };
    store.upsert_body(&row, false, false, "Text", &[AttachmentRow { attachment_id: "2", filename: "a.pdf", media_type: "application/pdf", size_bytes: 1 }]).expect("body lands");

    // Retain a real file so trim can delete it.
    let blob = dir.join("blob-a.pdf");
    std::fs::write(&blob, b"x").expect("blob written");
    store.record_attachment_file("INBOX", 1, "2", &blob.to_string_lossy()).expect("recorded");
    assert_eq!(store.attachment_path("INBOX", 1, "2").expect("lookup").as_deref(), Some(blob.to_string_lossy().as_ref()));

    let status = store.status().expect("status");
    assert_eq!(status.message_count, 1);
    assert_eq!(status.attachment_count, 1);
    assert!(status.db_size_bytes > 0);
    assert!(status.last_write.is_some());

    // Trim to Bodies: attachment rows and the file go, the body stays.
    store.trim(torromail_core::CacheLevel::Bodies).expect("trimmed");
    assert!(store.attachment_path("INBOX", 1, "2").expect("lookup").is_none());
    assert!(!blob.exists());
    assert!(store.get_message("INBOX", 1).expect("read").expect("row").body_text.is_some());

    // Trim to Headers: the body goes too, the summary stays searchable by subject.
    store.trim(torromail_core::CacheLevel::Headers).expect("trimmed");
    assert!(store.get_message("INBOX", 1).expect("read").expect("row").body_text.is_none());

    // Trim to Off: nothing left.
    store.trim(torromail_core::CacheLevel::Off).expect("trimmed");
    assert_eq!(store.status().expect("status").message_count, 0);
    drop(store);
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] Steps 2–4: fail → implement (trim deletes retained files best-effort
  before dropping rows; `db_size_bytes` = size of the sqlite file plus `-wal`
  sibling via `std::fs::metadata`) → green.
- [ ] **Step 5:** Commit `feat(cache): retained attachments, trim, status`.

---

### Task 4: Core — `CacheLevel` replaces `CachePolicy`; UIDVALIDITY reaches providers

**Files:**
- Modify: `crates/torromail-core/src/lib.rs`, `crates/torromail-core/src/imap_provider.rs`
- Tests: `crates/torromail-core/tests/core_contract.rs`, `crates/torromail-core/tests/imap_contract.rs`

**Interfaces:**
- `CacheLevel` (landed in Task 1) replaces `CachePolicy` everywhere:
  `Account`/`AccountDraft` field becomes `cache_level: CacheLevel`
  (default `Headers`), getter `cache_level()`; `CachePolicy` is deleted; the
  `core_contract.rs` import list drops it. `CacheLevel::ceiling(read: ReadAccess) -> CacheLevel`
  maps None→Off, Headers→Headers, FullMessage→Bodies, WithAttachments→Attachments;
  `effective(chosen, read) = min(chosen, ceiling(read))` as a free function
  `pub fn effective_cache_level(chosen: CacheLevel, read: ReadAccess) -> CacheLevel`.
- `MailProvider` gains a defaulted method — fixtures stay untouched:

```rust
    /// The server-stated UIDVALIDITY of `mailbox`, if this provider ever
    /// learned one. The cache keys its rows by it; `None` means "unknown".
    fn mailbox_generation(&self, _account_id: &AccountId, _mailbox: &str) -> Option<u32> {
        None
    }
```

  (plus the forwarding arm in the `&mut T` blanket impl).
- `ImapClient::select` parses `UIDVALIDITY` from the untagged SELECT
  responses (`* OK [UIDVALIDITY 123] …`) into a `BTreeMap<String, u32>`;
  `ImapClient::uidvalidity(mailbox) -> Option<u32>` reads it;
  `ImapMailProvider::mailbox_generation` selects (to learn it) and answers.

- [ ] **Step 1: Failing tests** — in `core_contract.rs`:

```rust
#[test]
fn cache_levels_are_capped_by_the_read_permission() {
    use torromail_core::{CacheLevel, effective_cache_level};
    assert_eq!(effective_cache_level(CacheLevel::Attachments, ReadAccess::FullMessage), CacheLevel::Bodies);
    assert_eq!(effective_cache_level(CacheLevel::Headers, ReadAccess::WithAttachments), CacheLevel::Headers);
    assert_eq!(effective_cache_level(CacheLevel::Bodies, ReadAccess::None), CacheLevel::Off);
    assert_eq!(CacheLevel::parse_legacy(false, "fullText"), CacheLevel::Off);
    assert_eq!(CacheLevel::parse_legacy(true, "metadata"), CacheLevel::Headers);
    assert_eq!(CacheLevel::parse_legacy(true, "fullText"), CacheLevel::Bodies);
}
```

  in `imap_contract.rs` (extend the existing select script constants with a
  `* OK [UIDVALIDITY 9] UIDs valid` line before `t2 OK SELECT completed` in
  ONE new test):

```rust
#[test]
fn select_captures_uidvalidity_for_the_cache() {
    let mut script = login_script();
    script.extend([
        line("* 1 EXISTS"),
        line("* OK [UIDVALIDITY 9] UIDs valid"),
        line("t2 OK SELECT completed"),
    ]);
    script.extend(fetch_script("t3", 101, "", HEADERS, BODY));
    let client = ImapClient::connect(ScriptedTransport::new(script, SentLog::default()), "work@example.com", "app-secret").expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);
    let _ = provider.get_message(&account_id, "INBOX/101").expect("fetch succeeds");
    assert_eq!(provider.mailbox_generation(&account_id, "INBOX"), Some(9));
}
```

- [ ] Steps 2–4: fail → implement → whole-workspace green (the `CachePolicy`
  deletion ripples into `core_contract.rs` and nothing else — `grep -rn
  CachePolicy` must end empty outside the plan/spec docs).
- [ ] **Step 5:** Commit `feat(core): cache levels replace cache policy; SELECT captures UIDVALIDITY`.

---

### Task 5: Policy document + `mail_get_cache_status` become honest

**Files:**
- Modify: `crates/torromail-mcp/src/policy_document.rs`, `crates/torromail-mcp/src/lib.rs`
- Test: `crates/torromail-mcp/tests/tool_contract.rs`

**Interfaces:**
- `CacheFacts` shrinks to `{ pub level: CacheLevel }`. `parse_cache_facts`
  reads `cache.level` (via `CacheLevel::parse`), falls back to the legacy
  shape (via `CacheLevel::parse_legacy` on `local_cache_enabled` + `mode`),
  and defaults to `Headers` when no cache block exists.
- `LineMcpServer` gains `cache_dir: Option<PathBuf>` (sibling `cache/` of the
  policy file, like `attachments_dir`) — set in `with_policy_path`.
- `handle_mail_get_cache_status` answers per account:

```json
{ "account_id": "work", "level": "bodies", "effective_level": "headers",
  "message_count": 512, "attachment_count": 3, "size_bytes": 1849344,
  "last_write": 1786430000 }
```

  `effective_level` = `effective_cache_level(facts.level, policy read)`.
  Counts come from `CacheStore::status` when the DB exists; a missing DB
  answers zeros — never an error. `size_bytes` = DB size + the account's
  `attachments/<account>` directory size.

- [ ] **Step 1: Failing test** (tool_contract):

```rust
#[test]
fn cache_status_reports_levels_and_real_counts() {
    let path = temp_policy_path("cache-status");
    std::fs::write(&path, r#"{"version":1,"accounts":[{"id":"work","read":"headers","write":{},"send":false,"per_folder":false,"folder_rules":{},"cache":{"level":"bodies"}}]}"#).expect("policy written");
    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());
    let response = server.handle_line(r#"{"jsonrpc":"2.0","id":70,"method":"tools/call","params":{"name":"mail_get_cache_status","arguments":{"account_id":"work"}}}"#).expect("a response");
    std::fs::remove_file(&path).ok();
    // The chosen level survives; the headers-only read permission caps it.
    assert!(response.contains(r#"\"level\":\"bodies\""#), "got: {response}");
    assert!(response.contains(r#"\"effective_level\":\"headers\""#), "got: {response}");
    assert!(response.contains(r#"\"message_count\":0"#), "got: {response}");
}

#[test]
fn legacy_cache_blocks_still_parse() {
    let path = temp_policy_path("cache-legacy");
    std::fs::write(&path, r#"{"version":1,"accounts":[{"id":"work","read":"with_attachments","write":{},"send":false,"per_folder":false,"folder_rules":{},"cache":{"local_cache_enabled":true,"mode":"fullText","index_bodies":true,"index_attachments":true,"storage":"12 MB"}}]}"#).expect("policy written");
    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());
    let response = server.handle_line(r#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"mail_get_cache_status","arguments":{"account_id":"work"}}}"#).expect("a response");
    std::fs::remove_file(&path).ok();
    assert!(response.contains(r#"\"level\":\"bodies\""#), "got: {response}");
    assert!(response.contains(r#"\"effective_level\":\"bodies\""#), "got: {response}");
}
```

  The pre-existing cache-status test (around line 604, asserting the old
  five fields echo) changes to expect the new shape.

- [ ] Steps 2–4: fail → implement → green (check `grep -n 'local_cache_enabled' crates/torromail-mcp` ends empty outside tests for legacy parsing).
- [ ] **Step 5:** Commit `feat(mcp): cache status reports level, cap, and real counts`.

---

### Task 6: Write-through and cache hits in the tool paths

**Files:**
- Modify: `crates/torromail-mcp/src/lib.rs`
- Test: `crates/torromail-mcp/tests/tool_contract.rs`

**Interfaces:**
- `LineMcpServer` holds `caches: RefCell<BTreeMap<AccountId, Rc<CacheStore>>>`;
  `fn cache_for(&self, account_id, facts_are_configured: bool, level) -> Option<Rc<CacheStore>>`
  answers `None` for fixture identities, `Off` levels, or a store that fails
  to open (a broken cache must never break mail access — log nothing, serve
  live).
- `run_with_connection` passes the handle: the closure signature grows a
  `cache: Option<&CacheStore>` parameter alongside the effective level
  (compute both from the same reloaded document the runtime came from).
- Write-through points inside handlers, all best-effort (`let _ = …`):
  - `handle_mail_search`: after the service answers, for every hit whose
    `message_id` splits as `mailbox/uid`, `note_uidvalidity(mailbox,
    provider.mailbox_generation(...))` once per distinct mailbox, then
    `upsert_summary`.
  - `handle_mail_get_message` / `handle_mail_get_thread` with bodies and
    effective level ≥ `Bodies`: `upsert_body` (attachment rows from
    `message.attachments()`).
  - `handle_mail_get_attachment` at level `Attachments`:
    `record_attachment_file` after the write; before fetching, ask
    `attachment_path` — an existing file answers `from_cache: true` without
    touching the provider (policy checks still run first via
    `service.download_allowed`; a denied mailbox never serves from cache
    either — resolve the mailbox from the split message id).
  - `handle_mail_get_message` cache hit: when `include_body` and the cached
    row has a body, answer from cache (marked `"from_cache": true` on the
    message JSON) without a provider fetch — but only after the same policy
    checks the live path runs (`ReadHeaders` account-wide + folder,
    `ReadBody` folder). The mailbox comes from splitting the id; ids that do
    not split (fixtures) never hit the cache.
- Message JSON grows an optional `from_cache` field (absent = live).

- [ ] **Step 1: Failing test** (fixture providers produce non-splittable ids,
  so these tests use `mailbox/uid`-shaped fixture ids):

```rust
/// Fixture messages whose ids look like IMAP ids, so the cache path engages.
fn imap_shaped_mailbox() -> FixtureMailProvider {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account, "INBOX", "INBOX/7", "t1", "Rechnung März", "billing@example.com", "s", "Anbei die Rechnung.",
    );
    FixtureMailProvider::new([message])
}

#[test]
fn read_messages_land_in_the_cache_and_serve_the_next_read() {
    let path = temp_policy_path("cache-write-through");
    std::fs::write(&path, r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{},"send":false,"per_folder":false,"folder_rules":{},"cache":{"level":"bodies"}}]}"#).expect("policy written");
    let opens = Rc::new(Cell::new(0usize));
    let counter = opens.clone();
    let server = LineMcpServer::with_connect_override(path.clone(), true, move |_account| {
        counter.set(counter.get() + 1);
        Ok(Box::new(imap_shaped_mailbox()) as Box<dyn MailProvider>)
    });

    let request = r#"{"jsonrpc":"2.0","id":80,"method":"tools/call","params":{"name":"mail_get_message","arguments":{"account_id":"work","message_id":"INBOX/7","include_body":true}}}"#;
    let first = server.handle_line(request).expect("a response");
    let second = server.handle_line(request).expect("a response");
    let cache_file = path.parent().expect("dir").join("cache/work.sqlite");
    let cached_exists = cache_file.exists();
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(path.parent().expect("dir").join("cache/work.sqlite")).ok();
    std::fs::remove_dir_all(path.parent().expect("dir").join("cache")).ok();

    assert!(first.contains("Anbei die Rechnung"), "got: {first}");
    assert!(cached_exists, "the store was created");
    assert!(second.contains(r#"\"from_cache\":true"#), "got: {second}");
}
```

  (A caveat this test bakes in: the second read never reaches the provider —
  with fixtures the observable difference is the `from_cache` marker, since
  the pooled fixture connection would have answered anyway.)

  Wait — `with_connect_override(path, true, …)` marks the account
  *fixture-identity* (no imap block), and fixture identities skip the cache
  per the constraints. The test would fail forever. **Resolution (binding):**
  the cache engages for accounts whose *message ids split* — the
  configured/fixture distinction is expressed through `cache.level` plus the
  id shape, not through `ConnIdentity`. The constraint line "only configured
  accounts are cached" is implemented as "only ids of the `mailbox/uid`
  shape are cached", which is what actually protects fixture data. Update
  the Global Constraints reading accordingly; the spec's intent (fixture
  accounts stay in-memory) holds because fixture ids (`m1`, `draft-1`) never
  split.

- [ ] Steps 2–4: fail → implement → green, plus a second test asserting a
  headers-level policy caches no body (`cache/work.sqlite` row's body stays
  NULL → third read still live: response lacks `from_cache`).
- [ ] **Step 5:** Commit `feat(mcp): tool reads fill the cache and cache hits answer reads`.

---

### Task 7: Search merges the local index; unreachable servers degrade to cache

**Files:**
- Modify: `crates/torromail-core/src/lib.rs` (SearchHit), `crates/torromail-mcp/src/lib.rs`
- Test: `crates/torromail-mcp/tests/tool_contract.rs`

**Interfaces:**
- `SearchHit` gains `from_cache: bool` (default false; builder
  `with_from_cache(bool)`, getter). `PolicyEngine` derives `Clone`.
- `handle_mail_search` on a non-empty query: run the service search; then
  query `cache.search(query, limit)`, map hits to `SearchHit` (id =
  `mailbox/uid`, `from_cache: true`), drop those whose mailbox the policy
  blocks (`policy.allows_in(mailbox, Capability::Search)` via the cloned
  engine), dedupe against the live set by message id, append. Result JSON:
  hits carry `"from_cache"`, the set carries `"source": "live"`.
- When the service search fails with `Core(ProviderFailure(_))` and a cache
  exists: answer from the cache alone (`"source": "cache_only"`), no error.
  Other failures pass through. The empty-query browse never consults the
  cache (recency is the server's to answer).
- The merged hits enter the session store (so `mail_refine_search` refines
  over them) — build the result set from the merged list via
  `sessions.create(...)` exactly like the live path does today: move the
  set-creation out of `MailAccessService::search` usage by doing the merge
  BEFORE `sessions.create`. Concretely: replace the handler's single
  `service.search(…)` call with `service.search_hits(…)` — a new core method
  identical to `search` but returning `Vec<SearchHit>` without creating the
  session — then merge, then `sessions.create(account_id, query, merged,
  100, 7200)`. `MailAccessService::search` keeps existing behavior for other
  callers (none today — fold it into `search_hits` + a thin wrapper).

- [ ] **Step 1: Failing test:**

```rust
#[test]
fn search_merges_cached_hits_and_survives_a_dead_server() {
    let path = temp_policy_path("cache-search-merge");
    std::fs::write(&path, r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{},"send":false,"per_folder":false,"folder_rules":{},"cache":{"level":"bodies"}}]}"#).expect("policy written");

    // Seed the cache directly — the store is the same one the server opens.
    let cache_dir = path.parent().expect("dir").join("cache");
    let store = torromail_cache::CacheStore::open(&cache_dir, "work").expect("store opens");
    store.upsert_body(
        &torromail_cache::SummaryRow { mailbox: "INBOX", uid: 9, subject: "Vertrag Entwurf", sender: "legal@example.com", date: "" },
        false, false, "Der Vertragsentwurf im Anhang.", &[],
    ).expect("seeded");
    drop(store);

    // A provider whose search finds nothing (fixture has no matching mail),
    // so any hit for "vertrag" can only come from the cache.
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account| {
        Ok(Box::new(imap_shaped_mailbox()) as Box<dyn MailProvider>)
    });
    let search = r#"{"jsonrpc":"2.0","id":81,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"vertrag"}}}"#;
    let merged = server.handle_line(search).expect("a response");
    assert!(merged.contains("Vertrag Entwurf"), "got: {merged}");
    assert!(merged.contains(r#"\"from_cache\":true"#), "got: {merged}");
    assert!(merged.contains(r#"\"source\":\"live\""#), "got: {merged}");

    // A dead connection: open fails → the cache answers alone.
    let dead = LineMcpServer::with_connect_override(path.clone(), true, |_account| {
        Err(CoreError::ProviderFailure("no route to host".to_owned()))
    });
    let degraded = dead.handle_line(search).expect("a response");
    std::fs::remove_file(&path).ok();
    std::fs::remove_dir_all(&cache_dir).ok();
    assert!(degraded.contains("Vertrag Entwurf"), "got: {degraded}");
    assert!(degraded.contains(r#"\"source\":\"cache_only\""#), "got: {degraded}");
}
```

  Note the second half: the failure happens in `open_connection`, before any
  handler runs — `run_with_connection` must learn the degraded path too. The
  clean cut: when opening fails AND the tool is `mail_search` with a cache,
  fall through to a cache-only handler rather than `json_rpc_error`.
  Implement as: `run_with_connection` keeps failing exactly as today, and the
  `MailSearch` dispatch arm catches the `-32000` open failure by calling a
  `handle_mail_search_cache_only` fallback first-class — i.e. the dispatch
  arm for search does not use `run_with_connection`'s error directly but a
  variant `run_with_connection_or` that takes a fallback closure. Keep the
  variant private and search-only.

- [ ] Steps 2–4: fail → implement → green. Also update the existing search
  JSON assertions if any assert exact hit shapes.
- [ ] **Step 5:** Commit `feat(mcp): mail_search merges the local index and degrades to cache_only`.

---

### Task 8: Batched summary fetch + `--rebuild-cache` / `--trim-cache`

**Files:**
- Modify: `crates/torromail-core/src/imap_provider.rs`,
  `crates/torromail-mcp/src/lib.rs`, `crates/torromail-mcp/src/main.rs`
- Tests: `crates/torromail-core/tests/imap_contract.rs` (batch fetch),
  `crates/torromail-mcp/tests/tool_contract.rs` (trim through the store)

**Interfaces:**
- `ImapClient::uid_fetch_summaries(mailbox, uids: &[u32]) -> CoreResult<Vec<FetchedSummary>>`
  — one `UID FETCH a,b,c (UID BODY.PEEK[HEADER.FIELDS (SUBJECT FROM DATE)])`,
  parsing every untagged FETCH line (reuse the literal collection; the UID in
  each line's text identifies the message: parse `UID <n>` from
  `fetch.text`). 500 messages, one round trip.
- `torromail_mcp::rebuild_cache(account_id: &str, policy_path: Option<PathBuf>, presented_token: Option<&str>, progress: &mut dyn FnMut(&str)) -> Result<(), String>`
  — resolves the document account (reuse the `check_account` plumbing for
  facts + secret), wipes via `CacheStore::trim(Off)`, then per readable
  mailbox: select, `note_uidvalidity`, `uid_search` ALL, newest 500,
  `uid_fetch_summaries` in chunks of 50, `upsert_summary` each, one progress
  line per chunk; then INBOX newest 100 `uid_fetch` bodies when the effective
  level allows. Progress lines are the JSON from the spec; the final line is
  `{"phase":"done","messages":N,"size_bytes":M}`.
- `torromail_mcp::trim_cache(account_id, policy_path) -> Result<(), String>`
  — opens the store, trims to the document's effective level.
- `main.rs`: `--rebuild-cache <id>` (prints progress lines to stdout, exit 0/1)
  and `--trim-cache <id>` flags, both before the stdio loop, same pattern as
  `--check-account`.

- [ ] **Step 1: Failing tests** — imap_contract:

```rust
#[test]
fn a_batched_summary_fetch_is_one_command() {
    let mut script = login_script();
    script.extend([line("* 2 EXISTS"), line("t2 OK SELECT completed")]);
    script.extend([
        line(&format!("* 1 FETCH (UID 8 BODY[HEADER.FIELDS (SUBJECT FROM DATE)] {{{}}}", HEADERS.len())),
        Incoming::Bytes(HEADERS.as_bytes().to_vec()),
        line(")"),
        line(&format!("* 2 FETCH (UID 9 BODY[HEADER.FIELDS (SUBJECT FROM DATE)] {{{}}}", HEADERS.len())),
        Incoming::Bytes(HEADERS.as_bytes().to_vec()),
        line(")"),
        line("t3 OK FETCH completed"),
    ]);
    let log = SentLog::default();
    let mut client = ImapClient::connect(ScriptedTransport::new(script, log.clone()), "work@example.com", "app-secret").expect("login succeeds");
    client.select("INBOX").expect("selected");
    let summaries = client.uid_fetch_summaries("INBOX", &[8, 9]).expect("batch works");
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].uid, 8);
    assert_eq!(summaries[1].uid, 9);
    let fetches: Vec<_> = log.lines().into_iter().filter(|l| l.contains("UID FETCH")).collect();
    assert_eq!(fetches.len(), 1, "one command for the whole batch: {fetches:?}");
    assert!(fetches[0].contains("UID FETCH 8,9 "), "got: {fetches:?}");
}
```

  (`ImapClient::select` is currently private — make it `pub(crate)`? It is
  in another crate for the test. `uid_fetch_summaries` selects internally
  like every other method; drop the explicit `client.select` line and let the
  script include the SELECT exchange — mirror `uid_fetch_summary` tests.)

  tool_contract (trim end-to-end through the public function):

```rust
#[test]
fn trim_cache_drops_material_above_the_effective_level() {
    let path = temp_policy_path("cache-trim");
    std::fs::write(&path, r#"{"version":1,"accounts":[{"id":"work","read":"headers","write":{},"send":false,"per_folder":false,"folder_rules":{},"cache":{"level":"attachments"}}]}"#).expect("policy written");
    let cache_dir = path.parent().expect("dir").join("cache");
    let store = torromail_cache::CacheStore::open(&cache_dir, "work").expect("store opens");
    store.upsert_body(&torromail_cache::SummaryRow { mailbox: "INBOX", uid: 1, subject: "S", sender: "a@example.com", date: "" }, false, false, "Body", &[]).expect("seeded");
    drop(store);

    torromail_mcp::trim_cache("work", Some(path.clone())).expect("trim runs");

    let store = torromail_cache::CacheStore::open(&cache_dir, "work").expect("store reopens");
    let row = store.get_message("INBOX", 1).expect("read").expect("summary survives");
    assert!(row.body_text.is_none(), "headers read permission caps the level, the body goes");
    drop(store);
    std::fs::remove_file(&path).ok();
    std::fs::remove_dir_all(&cache_dir).ok();
}
```

  `rebuild_cache` itself is covered to the extent a scripted transport allows
  — a unit test for its mailbox/window math (`newest_window(uids, 500)`)
  plus the CLI wiring compiling; the live pass is the manual step at the end.

- [ ] Steps 2–4: fail → implement → green.
- [ ] **Step 5:** Commit `feat(mcp): batched prefetch, --rebuild-cache and --trim-cache`.

---

### Task 9: Swift model — one level, legacy decode, new policy block

**Files:**
- Modify: `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`
- Test: `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`

**Interfaces:**

```swift
public enum CacheLevel: String, CaseIterable, Identifiable, Hashable, Sendable, Codable, Comparable {
    case off, headers, bodies, attachments
    public var id: Self { self }
    public static func < (lhs: Self, rhs: Self) -> Bool { /* case order */ }
    /// The most the read permission justifies keeping on disk.
    public static func ceiling(for read: ReadAccess) -> CacheLevel
}
public struct SearchCacheSettings: Hashable, Sendable, Codable {
    public var level: CacheLevel   // sole stored fact; key name "level"
    public init(level: CacheLevel = .headers)
    // Custom init(from:) — new shape first, then the legacy keys
    // (localCacheEnabled/cacheMode/indexBodies/indexAttachments/storage)
    // mapped exactly like the server: enabled=false → .off,
    // metadata|headers → .headers, body|fullText → .bodies.
}
```

- `CacheMode` is deleted; `label(for mode:)` in the app goes with it (Task 10).
- `accountObject(for:)` publishes `"cache": ["level": account.searchCache.level.rawValue]`.
- `MailAccount` keeps the `searchCache` property name and storage key — the
  contract test's exact-key-set assertion stays untouched.

- [ ] **Step 1: Failing contract checks** (append to `main.swift`, following
  its `require(_:_:)` style):

```swift
// Cache settings: the single level replaces the four switches, and a state
// file written by the previous build still decodes to the right level.
let legacyCacheJSON = Data("""
{"localCacheEnabled":true,"cacheMode":"fullText","indexBodies":true,"indexAttachments":false,"storage":"12 MB"}
""".utf8)
let migrated = try? JSONDecoder().decode(SearchCacheSettings.self, from: legacyCacheJSON)
require(migrated?.level == .bodies, "legacy fullText cache decodes to the bodies level")
let disabledCacheJSON = Data("""
{"localCacheEnabled":false,"cacheMode":"metadata","indexBodies":false,"indexAttachments":false,"storage":"0 MB"}
""".utf8)
require((try? JSONDecoder().decode(SearchCacheSettings.self, from: disabledCacheJSON))?.level == .off,
    "a disabled legacy cache decodes to off")
require((try? JSONDecoder().decode(SearchCacheSettings.self, from: Data(#"{"level":"attachments"}"#.utf8)))?.level == .attachments,
    "the new shape decodes directly")
require(CacheLevel.ceiling(for: .headers) == .headers && CacheLevel.ceiling(for: .fullMessage) == .bodies
    && CacheLevel.ceiling(for: .withAttachments) == .attachments && CacheLevel.ceiling(for: .none) == .off,
    "the read permission caps the cache level")
```

  Plus: find the contract's policy-publishing assertion (it checks the
  published JSON contains cache facts) and update it to expect
  `"level"` and the absence of `"local_cache_enabled"`.

- [ ] Steps 2–4: fail (swift run … TorroMailKitContract) → implement →
  contract passes, `swift build` clean. Any compile error listing other
  `searchCache.` field uses points straight at app-side code that Task 10
  rewrites — stub those call sites minimally ONLY if they block the build;
  otherwise do Task 10 immediately after in the same session.
- [ ] **Step 5:** Commit `feat(app): cache settings become one level with legacy decode`.

---

### Task 10: App UI — one picker, real storage, working delete and rebuild

**Files:**
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`,
  `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift` (CacheRebuild
  runner + storage measurement),
  `apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj/Localizable.strings`

**Interfaces (TorroMailKit):**

```swift
/// Runs `torromail-mcp --rebuild-cache <id>`, reporting each JSON progress
/// line; modeled on AccountCheck.run (same locator, policy env, app token,
/// termination handling). `cancel()` terminates the child.
public final class CacheRebuild {
    public struct Progress: Sendable { public var phase: String; public var mailbox: String; public var done: Int; public var total: Int }
    public static func start(accountID: String, executableName: String,
                             onProgress: @escaping @Sendable (Progress) -> Void,
                             onFinish: @escaping @Sendable (Result<Void, Error>) -> Void) -> CacheRebuild
    public func cancel()
}
/// DB file (+ -wal) plus the account's attachments directory, in bytes.
public enum CacheStorage {
    public static func sizeBytes(accountID: String) -> Int64
    public static func delete(accountID: String) throws   // cache/<id>.sqlite* + attachments/<id>/
    public static func label(bytes: Int64) -> String       // ByteCountFormatter, file style
}
/// Invokes `torromail-mcp --trim-cache <id>` fire-and-forget (same locator).
public enum CacheTrim { public static func run(accountID: String, executableName: String) }
```

**UI (`cacheSection` replacement):**

```swift
private var cacheSection: some View {
    Section {
        Picker(L("Cache"), selection: $account.searchCache.level) {
            Text(L("Off")).tag(CacheLevel.off)
            Text(L("Subject & sender")).tag(CacheLevel.headers)
            Text(L("Full messages")).tag(CacheLevel.bodies)
            Text(L("Messages & attachments")).tag(CacheLevel.attachments)
        }
        if effectiveCacheLevel < account.searchCache.level {
            Text(String(format: L("Limited to “%@” by the read permission."), label(for: effectiveCacheLevel)))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        if account.searchCache.level != .off {
            if let rebuild = rebuildProgress {
                LabeledContent(L("Rebuilding…")) {
                    ProgressView(value: rebuild.fraction)
                    Button(L("Cancel")) { cancelRebuild() }
                        .buttonStyle(.bordered)
                }
            } else {
                LabeledContent(L("Storage")) {
                    Text(CacheStorage.label(bytes: storageBytes))
                    Button(L("Rebuild")) { startRebuild() }
                        .buttonStyle(.bordered)
                    Button(L("Delete")) { deleteCache() }
                        .torroButton()
                }
            }
            if let failure = rebuildFailure {
                Text(failure).font(.caption).foregroundStyle(.red)
            }
        }
    } header: {
        Text(L("Search & Cache"))
    } footer: {
        Text(L("More caching makes search faster but stores mail content on this Mac."))
    }
    .onAppear { storageBytes = CacheStorage.sizeBytes(accountID: account.id) }
    .onChange(of: account.searchCache.level) { _, newLevel in
        // Lowering the level trims immediately — the disk follows the decision.
        CacheTrim.run(accountID: account.id, executableName: mcpExecutableName)
        storageBytes = CacheStorage.sizeBytes(accountID: account.id)
    }
}
```

  State: `@State private var storageBytes: Int64 = 0`, `rebuildProgress`,
  `rebuildFailure`, the `CacheRebuild` handle. `effectiveCacheLevel` =
  `min(account.searchCache.level, CacheLevel.ceiling(for: account.permissions.read))`.
  `label(for: CacheLevel)` replaces `label(for: CacheMode)`. `deleteCache()`
  calls `CacheStorage.delete` and refreshes the figure. Permission changes
  also trigger `CacheTrim.run` (add to the permission section's onChange or
  the model's save path — pick the single choke point where permissions
  persist, `model.save()`-adjacent, so folder-rule edits count too).
  `mcpExecutableName` — reuse whatever constant `AccountCheck` callers pass
  today (grep for the existing call site).

**Strings (de.lproj):** `"Cache" = "Cache";`, `"Off" = "Aus";`,
`"Subject & sender" = "Betreff & Absender";`,
`"Full messages" = "Ganze E-Mails";`,
`"Messages & attachments" = "E-Mails & Anhänge";`,
`"Rebuild" = "Neu aufbauen";`, `"Rebuilding…" = "Baue neu auf…";`,
`"Cancel" = "Abbrechen";`,
`"Limited to “%@” by the read permission." = "Durch das Leserecht auf „%@“ begrenzt.";`
— and the four dead strings (`Metadata`, `Headers`, `Bodies`, `Full text`,
`Full text index`, `Attachment index`) leave the file.

- [ ] **Step 1:** No UI test harness exists — the contract test from Task 9
  already covers the model. Verification here is `swift build` + a manual
  smoke run (`scripts/make-app-bundle.sh` if launching).
- [ ] **Step 2:** Implement kit pieces + UI + strings.
- [ ] **Step 3:** `swift run … TorroMailKitContract` and `swift build` green.
- [ ] **Step 4:** Commit `feat(app): the cache section drives a real cache`.

---

### Task 11: Docs, spec sync, full verification

**Files:** `AGENTS.md`, `docs/architecture.md`, spec file as needed.

- [ ] **Step 1:** `docs/architecture.md` — rewrite the "Search And Cache"
  section: one level in the permissions' language, capped by read access;
  SQLite+FTS5 per account under `cache/`; write-through from tool calls;
  search merge with `from_cache`/`source`; `--rebuild-cache`/`--trim-cache`;
  status facts. `AGENTS.md` — the cache bullet in the app-role list stays;
  add one sentence to the tool-surface section that `mail_get_cache_status`
  now reports real counts and the effective level.
- [ ] **Step 2:** Re-read the spec's Part 2 against what landed; where
  implementation deviated (FTS contentful table, id-shape rule for caching,
  chunked prefetch), amend the spec the way Task 6 of phase 1 did.
- [ ] **Step 3:** Full verification: `cargo test`, `cargo build -p
  torromail-mcp`, both Swift commands.
- [ ] **Step 4:** Commit `docs: the cache section of the architecture matches the real cache`.
- [ ] **Step 5:** Manual pass against the live Gmail account (user-driven):
  set level, rebuild, watch progress, search, read, download attachment
  twice (`from_cache: true` the second time), delete. Requires restarting
  MCP clients so they spawn the new binary.

---

## Self-review notes

- Two mid-plan corrections are left in place deliberately, marked
  **Resolution/decision (binding)**: core owns `CacheLevel` (not the cache
  crate), FTS5 runs contentful, and cacheability follows the
  `mailbox/uid` id shape rather than `ConnIdentity` — later tasks are
  written against the corrected versions.
- Spec coverage: level model + coupling (T4/T5/T9/T10), storage (T1–T3),
  write-through (T6), search merge + degraded mode (T7), rebuild/trim
  (T8/T10), real status (T5), app section + migration (T9/T10), docs (T11).
- Known deferred item: `mail_refine_search` gets merged hits via the session
  store automatically (T7 routes merged hits into `sessions.create`); no
  separate FTS path inside refine — the spec's "where it can" allows this.
