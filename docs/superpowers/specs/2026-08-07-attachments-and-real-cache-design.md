# Attachment Download and a Real Search Cache

Status: current. Written 2026-08-07.

## The problem

Assistants cannot get at incoming attachments, at all. The catalog knows
attachments only in the outgoing direction (`mail_create_draft`);
`mail_get_message` does not even *list* them — `StoredMessage` carries subject,
sender and rendered text, and the MIME renderer deliberately drops every part
that is not body text. Worse, the policy layer already advertises a
`download_attachments` right through `mail_get_policy` and the app already
offers the "Message and attachments" read level, so a paired assistant sees a
right it can never exercise. That is exactly the "looks available but is not"
trap the agent guide warns about, and in practice it sends assistants hunting
for phantom causes (the user's assistant blamed the cache settings).

The cache, meanwhile, is a facade. The "Search & Cache" section stores its
switches, publishes them into the policy document, and `mail_get_cache_status`
echoes them back — but no cache exists. Nothing is ever written to disk,
"Storage" shows a hard-coded `"0 MB"` string, the Delete button's action is
empty, and every read runs live over IMAP on each tool call. The level names
make the confusion worse: "Full text" as a *storage* level is indistinguishable
from the separate "Full text index" toggle, because the distinction means
nothing. The controls also speak a different language than the permission
controls right above them (Metadata/Headers/Bodies/Full text vs. Subject and
sender/Full message/Message and attachments), though they describe the same
material.

## What this adds

Two independently shippable parts:

1. **Attachments over MCP.** `mail_get_message` and `mail_get_thread` list a
   message's attachments; a new `mail_get_attachment` tool downloads one to a
   TorroMail-managed folder and returns the path. Gated by the existing
   "Message and attachments" read level, which finally does something.
2. **A real, lean cache.** One SQLite database with an FTS5 index per account,
   filled write-through by whatever the tools fetch anyway, plus an explicit
   rebuild. One control in the language of the permissions, a real storage
   figure, and Delete/Rebuild buttons that do what they say.

Out of scope, deliberately: extracting text from inside attachment files (PDF
contents are not searchable; filenames are), background synchronization or
IMAP IDLE (the cache fills from use and from explicit rebuilds; a health check
must not become a sync loop), and any cross-device story.

## Part 1: Attachments over MCP

### What counts as an attachment

The MIME tree of the fetched message is walked depth-first. A leaf part is an
attachment when it carries a filename — `Content-Disposition` `filename`/
`filename*` (RFC 2231) or, failing that, a `Content-Type` `name` parameter —
and it is not the part the renderer chose as the body text. Parts with
`Content-Disposition: inline` and a filename (embedded images, most signature
logos) are listed too, marked `inline: true`, so an assistant can skip them
without a second question. Each listed part gets an `attachment_id`: its MIME
part path from the walk (`"2"`, `"2.1"`), stable because listing and download
derive it from the same deterministic walk.

No protocol extension is needed. The existing `get_message` fetch already
carries every MIME part (`BODY.PEEK[TEXT]` is the full multipart body) and the
top-level `CONTENT-TYPE` header provides the boundary; single-part messages
are covered by the fetched `CONTENT-TRANSFER-ENCODING`.

### Listing in `mail_get_message` and `mail_get_thread`

The message JSON grows two fields:

```json
{
  "attachments": [
    {
      "attachment_id": "2",
      "filename": "angebot.pdf",
      "media_type": "application/pdf",
      "size_bytes": 182044,
      "inline": false,
      "download_allowed": true
    }
  ],
  "attachment_count": 1
}
```

`size_bytes` is the decoded size, computed from the encoded part.
`download_allowed` answers for the mailbox the message lives in, so an
assistant can say "there is an attachment, but downloading needs the
'Message and attachments' level in TorroMail" instead of guessing. The listing
itself appears from the "Full message" read level: attachment names are part
of reading a message, the bytes are not.

### `mail_get_attachment`

```json
{ "account_id": "gmail", "message_id": "4711", "attachment_id": "2" }
```

The server locates the message (same lookup as `mail_get_message`), extracts
the named part, decodes it (base64 and quoted-printable), writes the file
below the TorroMail application-support directory, and answers:

```json
{
  "message_id": "4711",
  "attachment_id": "2",
  "filename": "angebot.pdf",
  "media_type": "application/pdf",
  "size_bytes": 182044,
  "path": "/Users/…/Library/Application Support/TorroMail/attachments/gmail/4711/2-angebot.pdf",
  "from_cache": false
}
```

The path answer is the point: the server always runs on the same machine as
its stdio client, and a path never floods the assistant's context window the
way a base64 blob would. For clients without filesystem access (a pure chat
app), `"include_content": true` additionally inlines `content_base64` when the
decoded size is at most 2 MiB; larger files answer with the path alone and a
note to that effect.

The tool accepts no destination path, mirror of the rule on the outgoing side:
a client that chooses paths writes anywhere the user can. TorroMail decides
where files land; the client learns the location from the answer.

### Files on disk

`~/Library/Application Support/TorroMail/attachments/<account>/<message>/<attachment_id>-<filename>`.
Account and message directory names are the respective ids, sanitized.
Filenames are sanitized before writing: path separators and NUL stripped,
leading dots removed, length capped at 255 bytes; an empty or fully-consumed
name becomes `attachment-<id>.bin`. The id prefix replaces collision
suffixes: re-downloads land on the same path (idempotent), and two
same-named attachments of one message can never clash. The
decoded size is capped at 50 MiB — above that the tool refuses with the size
in the message, and the assistant can tell the user to fetch it in a mail
client.

These files double as the cache's attachment store. At cache level
"Messages & attachments" a downloaded file is recorded and *retained* — the
next `mail_get_attachment` for it answers `from_cache: true` without touching
IMAP. Below that level the file still lands there (it is the deliverable), but
nothing records it, and a cleanup sweep on server start deletes unrecorded
files older than 24 hours.

### Rights and audit

`mail_get_attachment` asks the existing `Capability::DownloadAttachments`
question, folder-scoped exactly like body reads: the account needs the
"Message and attachments" read level, and the message's mailbox must not have
its read rule switched off. The call is audited and health-logged like every
other mailbox tool call.

### Errors

Unknown `attachment_id` or `message_id`: invalid params, naming the id. Part
too large: tool error carrying the decoded size and the 50 MiB cap. Right
missing: the same policy-denial shape the other read tools use.

## Part 2: The cache becomes real

### One decision: the cache level

The section's four controls (cache toggle, four-value level, two index
toggles) collapse into one picker plus the storage row. Indexing is not a
separate decision — the index always covers exactly what is stored, that is
what a search cache is for — so both index toggles disappear. "Full text" as a
storage level disappears with them. What remains speaks the language of the
permissions above:

| Level | Stores | German label |
| --- | --- | --- |
| `off` | nothing | Aus |
| `headers` | subject, sender, date, flags | Betreff & Absender |
| `bodies` | headers + rendered body text | Ganze E-Mails |
| `attachments` | bodies + downloaded attachment files | E-Mails & Anhänge |

Default for new accounts: `headers` (what the old default flags persisted).
The FTS5 index covers subject and sender always, body text and attachment
*filenames* from `bodies` up (the listing rides along with the body fetch);
file *contents* never (out of scope above).

### Coupled to the read permission

The effective level is the minimum of the chosen level and the account's read
access — nothing is kept on disk that no assistant may read. `headers` needs
"Subject and sender", `bodies` needs "Full message", `attachments` needs
"Message and attachments". When the permission caps the chosen level, the
section says so in place ("Auf ‚Betreff & Absender' begrenzt durch das
Leserecht"). Lowering the read permission or the level triggers a trim: the
app invokes `torromail-mcp --trim-cache <account>`, which drops rows and files
above the new effective level. Cache writes stay server-owned; the app never
opens the database for writing.

### Storage

A new crate `torromail-cache` (rusqlite, bundled SQLite with FTS5) used by
`torromail-mcp`; `torromail-core` stays dependency-light and replaces its five
`CachePolicy` booleans with the `CacheLevel` enum. One database per account at
`~/Library/Application Support/TorroMail/cache/<account>.sqlite`.

Several processes write concurrently — every MCP client spawns its own server,
plus the rebuild command — so the database runs in WAL mode with a busy
timeout, and all writes are idempotent upserts keyed by mailbox and UID.
Tables: `mailboxes` (name, UIDVALIDITY), `messages` (mailbox, uid, message id,
thread id, subject, sender, date, flags, body text nullable, cached-at), an
FTS5 mirror of subject/sender/body/filenames, and `attachments` (message ref,
attachment id, filename, media type, size, path nullable — listing rows
arrive with body fetches, the path fills in on download and marks the file
retained). Cached rows are
only valid for the UIDVALIDITY under which they were written; a changed value
on SELECT invalidates that mailbox's rows, and an *unknown* generation is
never recorded over a known one. Cacheability follows the id shape: only
`mailbox/uid` ids are cached, which keeps fixture data (ids like `m1`) out
by construction.

### Write-through population

The tools feed the cache with what they fetched anyway: search result
summaries upsert header rows, `mail_get_message`/`mail_get_thread` upsert body
text at `bodies` and above, `mail_get_attachment` records and retains the file
at `attachments`. A cache miss never triggers an extra server round-trip; a
hit saves one. A row only serves message reads once it is *complete* — body
fetched, attachment listing included. Summary rows (search write-through)
never answer `mail_get_message`, not even header reads: they carry no
attachment listing, and "no attachments" would be a claim, not a fact.
Attachment downloads hit as described in Part 1.

### Search

The server stays the source of truth. A `mail_search` with a query runs the
provider search as today and, in the same call, queries the local FTS index;
results merge deduplicated by mailbox and message id, server order first,
cache-only hits appended and marked `from_cache: true` (those carry real
snippets from the cached body text; live hits keep their provider shape). The
result set carries `"source": "live"`; when the provider is unreachable the
cache answers alone with `"source": "cache_only"`, which turns a dead network
from an error into a degraded answer. The empty query ("what came in lately")
stays a pure provider listing — recency is the one thing a partial cache
cannot answer honestly. `mail_refine_search` narrows within the previous
result set using the index where it can.

### Rebuild

`torromail-mcp --rebuild-cache <account>` wipes the account's cache and
prefetches, newest first: 500 header rows per readable mailbox, then the
newest 100 INBOX bodies when the effective level allows bodies. Attachments
are never prefetched — they arrive on demand and are retained per level. The
command emits one JSON progress line per batch
(`{"phase":"headers","mailbox":"INBOX","done":120,"total":500}`, final line
`{"phase":"done","messages":1234,"size_bytes":…}`); the app runs it the way it
runs `--check-account`, shows the progress inline with a cancel button
(SIGTERM; batches are transactional, so a canceled rebuild leaves a valid
partial cache), and refuses to start a second rebuild for the same account.

### Delete and status

Delete is app-side and honest: remove `cache/<account>.sqlite*` and
`attachments/<account>/`. The server recreates the schema lazily on the next
write. `mail_get_cache_status` finally reports facts, per account:

```json
{
  "account_id": "gmail",
  "level": "bodies",
  "effective_level": "headers",
  "message_count": 512,
  "attachment_count": 3,
  "size_bytes": 1849344,
  "last_write": 1786430000.0
}
```

### Migration

The policy document's `cache` block becomes `{"level": "headers"}`. The server
maps the legacy shape (`local_cache_enabled: false` → `off`;
`metadata`/`headers` → `headers`; `body`/`fullText` → `bodies`; the index
booleans are ignored), so an old document keeps working until the app next
publishes. The Swift `SearchCacheSettings` shrinks to the single level with
the same decode mapping; the stored `storage` string is dropped — the figure
is computed, not stored.

## App changes

The section keeps its "Search & Cache" header and its footer sentence. Inside:
the level picker (Off / Subject & sender / Full messages / Messages &
attachments — German: Aus / Betreff & Absender / Ganze E-Mails / E-Mails &
Anhänge, deliberately echoing the permission labels), the capping hint when
the read permission limits the choice, and a storage row showing the real
on-disk size (database plus attachment files, computed by the app, refreshed
when the section appears and after rebuild or delete) with working
**Neu aufbauen** and **Löschen** buttons. During a rebuild the row shows
progress and a cancel button instead; failures surface inline, per the
status-only-on-exception principle. New strings land in `de.lproj` alongside
the existing ones.

## Documentation changes

`AGENTS.md` (tool catalog, cache paragraph) and `docs/architecture.md` (MCP
surface, "Search And Cache", the GUI-only list's cache wording) are updated in
the same change that implements each part — the catalog stays honest.

## Phasing

Phase 1 ships the attachment listing and `mail_get_attachment` without any
cache (files land in the attachment directory, the 24-hour sweep keeps it
tidy). Phase 2 ships the cache: crate, level model, write-through, search
merge, rebuild/trim commands, status, app section, migration. Phase 1 ends the
user-visible failure; Phase 2 makes the settings mean something.

## Testing

- Core unit tests: MIME walk and part extraction (RFC 2231 filenames,
  quoted-printable and base64 decoding, nested multiparts, single-part
  messages), filename sanitization, level capping against read access.
- Cache crate tests: upsert idempotence, UIDVALIDITY invalidation, trim,
  FTS matching, concurrent writers (two connections, WAL).
- Scripted-transport IMAP tests: attachment extraction end to end without a
  network, unchanged fetch shapes.
- `tool_contract.rs`: the new tool's schema and answers, the extended message
  JSON, `mail_get_cache_status` facts, policy denials at each read level.
- Swift contract test: settings decode migration, policy publishing of the
  new cache block.
- Manual pass against the real Gmail account: list, download, cache hit,
  rebuild, delete, storage figure.
