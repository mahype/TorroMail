# Attachment Download over MCP — Implementation Plan (Spec Phase 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Assistants can list a message's attachments and download one to a
TorroMail-managed folder through a new `mail_get_attachment` tool, gated by the
existing "Message and attachments" read level.

**Architecture:** The MIME walk lives in `torromail-core/src/mime.rs` next to
the existing body renderer and reuses its parsers. `MailProvider` grows a
`get_attachment` method (fixture: seeded payloads; IMAP: the existing
`uid_fetch`, whose raw body is now kept). `MailAccessService.get_attachment`
does the two-step policy check. The MCP layer adds the tool, writes the file
under `<policy dir>/attachments/`, and sweeps stale files on server start.
Spec: `docs/superpowers/specs/2026-08-07-attachments-and-real-cache-design.md`.

**Tech Stack:** Rust (no new dependencies — base64/QP stay hand-rolled), Swift
only for log labels and localization.

## Global Constraints

- `torromail-core` takes no external dependencies; unsafe code is forbidden
  workspace-wide.
- Attachment listing appears from read level `full_message`; downloading bytes
  needs `with_attachments` (`Capability::DownloadAttachments`), folder-scoped.
- Download cap: 50 MiB decoded (`MAX_ATTACHMENT_DOWNLOAD_BYTES`); inline
  content cap: 2 MiB decoded (`MAX_INLINE_CONTENT_BYTES`).
- Files land at `attachments/<account>/<message>/<attachment_id>-<filename>`
  (id prefix instead of the spec's numeric collision suffix: deterministic,
  idempotent re-downloads, no collisions — amend the spec in Task 6).
- The tool accepts no destination paths from clients, ever.
- Verification before "done": `cargo test`, `cargo build -p torromail-mcp`,
  both Swift commands from AGENTS.md.
- Commit style: `feat(core): …` / `feat(mcp): …` / `docs: …`, each ending with
  the `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>` line.

---

### Task 1: Filename decoding for MIME part headers (`decode_filename_value`)

Attachment filenames arrive three ways: plain (`filename="a.pdf"`), RFC 2047
encoded-words (`filename="=?UTF-8?B?…?="`, B and Q forms), and RFC 2231
extended syntax (`filename*=UTF-8''%E2%82%AC.pdf`, possibly split into
`filename*0*`/`filename*1*` continuations). This task adds the pure decoding
helpers; the walk in Task 2 consumes them.

**Files:**
- Modify: `crates/torromail-core/src/mime.rs`

**Interfaces:**
- Produces: `pub(crate) fn filename_from_params(params: &[(String, String)]) -> Option<String>`
  — resolves `filename*`/`filename*N*`/`filename` (in that priority) from
  `parse_content_type`-shaped parameter lists, fully decoded.
- Produces (internal): `fn decode_rfc2047_words(value: &str) -> String`,
  `fn decode_rfc2231_value(value: &str) -> String`.
- Consumes: existing `decode_base64`, `decode_quoted_printable`,
  `decode_charset`, `param`.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` of `mime.rs`)

```rust
#[test]
fn plain_and_rfc2047_filenames_decode() {
    let plain = vec![("filename".to_owned(), "angebot.pdf".to_owned())];
    assert_eq!(filename_from_params(&plain).as_deref(), Some("angebot.pdf"));

    // "Größenliste.pdf" as one UTF-8 B word.
    let b_word = vec![(
        "filename".to_owned(),
        "=?UTF-8?B?R3LDtsOfZW5saXN0ZS5wZGY=?=".to_owned(),
    )];
    assert_eq!(filename_from_params(&b_word).as_deref(), Some("Größenliste.pdf"));

    // Q form: underscores are spaces, =XX escapes apply.
    let q_word = vec![(
        "filename".to_owned(),
        "=?utf-8?Q?M=C3=A4rz_Bericht.pdf?=".to_owned(),
    )];
    assert_eq!(filename_from_params(&q_word).as_deref(), Some("März Bericht.pdf"));
}

#[test]
fn rfc2231_extended_filenames_decode_and_win() {
    // filename* beats filename; charset prefix and percent escapes resolve.
    let extended = vec![
        ("filename".to_owned(), "fallback.bin".to_owned()),
        ("filename*".to_owned(), "UTF-8''%E2%82%AC-rechnung.pdf".to_owned()),
    ];
    assert_eq!(filename_from_params(&extended).as_deref(), Some("€-rechnung.pdf"));

    // Continuations join in numeric order; only segment 0 carries the charset.
    let split = vec![
        ("filename*0*".to_owned(), "UTF-8''ver%20".to_owned()),
        ("filename*1*".to_owned(), "trag.pdf".to_owned()),
    ];
    assert_eq!(filename_from_params(&split).as_deref(), Some("ver trag.pdf"));
}

#[test]
fn missing_filenames_stay_none() {
    assert_eq!(filename_from_params(&[]), None);
    let unrelated = vec![("charset".to_owned(), "utf-8".to_owned())];
    assert_eq!(filename_from_params(&unrelated), None);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p torromail-core filename`
Expected: compile error — `filename_from_params` not found.

- [ ] **Step 3: Implement** (in `mime.rs`, above the tests)

```rust
/// The filename a part declares, decoded. RFC 2231 `filename*` (and its
/// `filename*0*`… continuations) wins over plain `filename`, which may carry
/// RFC 2047 encoded-words. `None` when the part declares nothing.
pub(crate) fn filename_from_params(params: &[(String, String)]) -> Option<String> {
    if let Some(extended) = param(params, "filename*") {
        return Some(decode_rfc2231_value(&extended));
    }

    // Continuations: filename*0*, filename*1*, … — joined in numeric order.
    // Only the first segment carries the charset'language' prefix.
    let mut segments: Vec<(u32, String)> = params
        .iter()
        .filter_map(|(key, value)| {
            let index = key
                .strip_prefix("filename*")?
                .trim_end_matches('*')
                .parse()
                .ok()?;
            Some((index, value.clone()))
        })
        .collect();
    if !segments.is_empty() {
        segments.sort_by_key(|(index, _)| *index);
        let joined: String = segments
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        return Some(decode_rfc2231_value(&joined));
    }

    param(params, "filename").map(|plain| decode_rfc2047_words(&plain))
}

/// RFC 2231: `charset'language'percent-escaped-bytes`. A value without the
/// two apostrophes is taken as already-plain text.
fn decode_rfc2231_value(value: &str) -> String {
    let mut pieces = value.splitn(3, '\'');
    let (charset, escaped) = match (pieces.next(), pieces.next(), pieces.next()) {
        (Some(charset), Some(_language), Some(rest)) => (charset.to_owned(), rest),
        _ => (String::from("utf-8"), value),
    };

    let mut bytes = Vec::with_capacity(escaped.len());
    let mut rest = escaped;
    while let Some(index) = rest.find('%') {
        bytes.extend_from_slice(rest[..index].as_bytes());
        let escape = rest.get(index + 1..index + 3);
        match escape.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
            Some(byte) => {
                bytes.push(byte);
                rest = &rest[index + 3..];
            }
            None => {
                bytes.push(b'%');
                rest = &rest[index + 1..];
            }
        }
    }
    bytes.extend_from_slice(rest.as_bytes());

    decode_charset(&charset, &bytes)
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// RFC 2047 encoded-words (`=?charset?B|Q?text?=`) anywhere in the value;
/// everything between words passes through untouched.
fn decode_rfc2047_words(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("=?") {
        out.push_str(&rest[..start]);
        let word = &rest[start..];
        match decode_one_rfc2047_word(word) {
            Some((decoded, consumed)) => {
                out.push_str(&decoded);
                rest = &word[consumed..];
            }
            None => {
                out.push_str("=?");
                rest = &word[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One `=?charset?encoding?text?=` word at the start of `word`; answers the
/// decoded text and how many bytes the word consumed.
fn decode_one_rfc2047_word(word: &str) -> Option<(String, usize)> {
    let inner = word.strip_prefix("=?")?;
    let charset_end = inner.find('?')?;
    let charset = &inner[..charset_end];
    let after_charset = &inner[charset_end + 1..];
    let encoding = after_charset.chars().next()?;
    let text = after_charset.get(2..)?;
    let text_end = text.find("?=")?;
    let payload = &text[..text_end];

    let bytes = match encoding.to_ascii_lowercase() {
        'b' => decode_base64(&payload.chars().filter(|c| !c.is_ascii_whitespace()).collect::<String>())?,
        // Header Q form: `_` is a space, `=XX` escapes as in bodies.
        'q' => decode_quoted_printable(&payload.replace('_', " ")),
        _ => return None,
    };
    let decoded = decode_charset(charset, &bytes)
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
    // =? + charset + ? + encoding + ? + payload + ?=
    let consumed = 2 + charset_end + 1 + 2 + text_end + 2;
    Some((decoded, consumed))
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p torromail-core filename`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/torromail-core/src/mime.rs
git commit -m "feat(core): decode MIME part filenames (plain, RFC 2047, RFC 2231)"
```

---

### Task 2: MIME attachment walk (`list_attachments`, `extract_attachment_bytes`)

**Files:**
- Modify: `crates/torromail-core/src/mime.rs`

**Interfaces:**
- Produces:

```rust
pub(crate) struct MimeAttachment {
    pub id: String,          // MIME part path from the walk: "2", "2.1"; "1" for a single-part message
    pub filename: String,    // decoded, never empty (fallback: "attachment-<id>.bin")
    pub media_type: String,  // lowercased; "application/octet-stream" when absent
    pub size_bytes: usize,   // decoded estimate: base64 chars * 3/4 minus padding, else raw length
    pub inline: bool,        // Content-Disposition: inline
}
pub(crate) fn list_attachments(raw: &str, content_type: &str, transfer_encoding: &str) -> Vec<MimeAttachment>;
pub(crate) fn extract_attachment_bytes(raw: &str, content_type: &str, transfer_encoding: &str, attachment_id: &str) -> Option<(MimeAttachment, Vec<u8>)>;
```

- Consumes: Task 1's `filename_from_params`, existing `parts`,
  `split_headers_body`, `header_value`, `parse_content_type`, `param`,
  `decode_transfer`.
- Rule (from spec): a leaf part with a declared filename is an attachment;
  parts without filenames (the body text) are not.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`)

```rust
/// A realistic multipart/mixed: alternative body (plain+html), a PDF, and an
/// inline PNG inside the alternative? No — keep the nesting honest: the PDF
/// and PNG sit beside the alternative, the PNG nested one level deeper in a
/// related wrapper, so ids cross a boundary ("3.1").
fn mixed_message() -> (&'static str, &'static str) {
    let raw = concat!(
        "--outer\r\n",
        "Content-Type: multipart/alternative; boundary=inner\r\n\r\n",
        "--inner\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n\r\n",
        "Der Text\r\n",
        "--inner\r\n",
        "Content-Type: text/html; charset=utf-8\r\n\r\n",
        "<p>Der Text</p>\r\n",
        "--inner--\r\n",
        "--outer\r\n",
        "Content-Type: application/pdf\r\n",
        "Content-Disposition: attachment; filename*=UTF-8''Angebot%20M%C3%A4rz.pdf\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\n",
        "JVBERi0xLjQKJcOkw7zDtsOf\r\n",
        "--outer\r\n",
        "Content-Type: multipart/related; boundary=rel\r\n\r\n",
        "--rel\r\n",
        "Content-Type: image/png; name=logo.png\r\n",
        "Content-Disposition: inline; filename=logo.png\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\n",
        "iVBORw0KGgo=\r\n",
        "--rel--\r\n",
        "--outer--\r\n",
    );
    (raw, "multipart/mixed; boundary=outer")
}

#[test]
fn attachments_are_listed_with_stable_part_ids() {
    let (raw, content_type) = mixed_message();
    let listed = list_attachments(raw, content_type, "7bit");

    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "2");
    assert_eq!(listed[0].filename, "Angebot März.pdf");
    assert_eq!(listed[0].media_type, "application/pdf");
    assert!(!listed[0].inline);
    // "JVBERi0xLjQKJcOkw7zDtsOf" is 24 base64 chars, no padding: 18 bytes.
    assert_eq!(listed[0].size_bytes, 18);

    assert_eq!(listed[1].id, "3.1");
    assert_eq!(listed[1].filename, "logo.png");
    assert!(listed[1].inline);
}

#[test]
fn extraction_decodes_the_named_part() {
    let (raw, content_type) = mixed_message();
    let (info, bytes) = extract_attachment_bytes(raw, content_type, "7bit", "2")
        .expect("part 2 exists");
    assert_eq!(info.filename, "Angebot März.pdf");
    // "JVBERi0xLjQKJcOkw7zDtsOf" → "%PDF-1.4\n%" + UTF-8 "äüöß".
    assert_eq!(bytes, b"%PDF-1.4\n%\xc3\xa4\xc3\xbc\xc3\xb6\xc3\x9f".to_vec());
    assert!(extract_attachment_bytes(raw, content_type, "7bit", "9").is_none());
}

#[test]
fn a_single_part_attachment_message_is_part_one() {
    // The whole message *is* the file: no multipart, filename on the top level.
    let listed = list_attachments("AAAA", "application/pdf; name=direkt.pdf", "base64");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "1");
    assert_eq!(listed[0].filename, "direkt.pdf");

    let (_, bytes) = extract_attachment_bytes("AAAA", "application/pdf; name=direkt.pdf", "base64", "1")
        .expect("the single part");
    assert_eq!(bytes, vec![0, 0, 0]);
}

#[test]
fn a_body_without_filenames_lists_nothing() {
    assert!(list_attachments("Hallo", "text/plain; charset=utf-8", "7bit").is_empty());
}

#[test]
fn a_nameless_binary_part_gets_the_fallback_name() {
    let raw = concat!(
        "--b\r\n",
        "Content-Type: application/octet-stream\r\n",
        "Content-Disposition: attachment\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\n",
        "AAAA\r\n",
        "--b--\r\n",
    );
    let listed = list_attachments(raw, "multipart/mixed; boundary=b", "7bit");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "1");
    assert_eq!(listed[0].filename, "attachment-1.bin");
}
```

Note the last test: `Content-Disposition: attachment` *without* a filename is
still an attachment (the disposition says so explicitly); only then does the
fallback name apply. The walk therefore treats a part as an attachment when it
has a filename **or** an explicit `attachment` disposition.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p torromail-core attachment`
Expected: compile error — `list_attachments` not found. (The pre-existing
`an_attachment_only_part_yields_nothing` test keeps passing.)

- [ ] **Step 3: Implement** (in `mime.rs`)

```rust
/// One part of a message that is a file rather than body text: it declares a
/// filename, or says `Content-Disposition: attachment` outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MimeAttachment {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: usize,
    pub inline: bool,
}

/// Every attachment of a message, ids assigned by a depth-first walk of the
/// MIME tree — "2", "3.1" — stable because the walk is deterministic.
pub(crate) fn list_attachments(
    raw: &str,
    content_type: &str,
    transfer_encoding: &str,
) -> Vec<MimeAttachment> {
    let mut found = Vec::new();
    walk_attachments(raw, content_type, "", transfer_encoding, "", &mut |info, _| {
        found.push(info);
    });
    found
}

/// The decoded bytes of the attachment `attachment_id` names, with its
/// listing entry. `None` when no part carries that id.
pub(crate) fn extract_attachment_bytes(
    raw: &str,
    content_type: &str,
    transfer_encoding: &str,
    attachment_id: &str,
) -> Option<(MimeAttachment, Vec<u8>)> {
    let mut hit = None;
    walk_attachments(raw, content_type, "", transfer_encoding, "", &mut |info, body_and_encoding| {
        if info.id == attachment_id && hit.is_none() {
            let (body, encoding) = body_and_encoding;
            hit = Some((info, decode_transfer(body, encoding)));
        }
    });
    hit
}

/// Depth-first over the MIME tree. Multipart nodes recurse with their child
/// index appended to `prefix`; attachment leaves are reported with their raw
/// body and transfer encoding so a caller can decode exactly the part it
/// wants. A non-multipart top level walks as the single part "1".
fn walk_attachments(
    raw: &str,
    content_type: &str,
    content_disposition: &str,
    transfer_encoding: &str,
    prefix: &str,
    visit: &mut dyn FnMut(MimeAttachment, (&str, &str)),
) {
    let (mime_type, type_params) = parse_content_type(content_type);

    if mime_type.starts_with("multipart/") {
        let Some(boundary) = param(&type_params, "boundary") else {
            return;
        };
        for (index, segment) in parts(raw, &boundary).iter().enumerate() {
            let (headers, body) = split_headers_body(segment);
            let child_type = header_value(headers, "content-type").unwrap_or_default();
            let child_disposition =
                header_value(headers, "content-disposition").unwrap_or_default();
            let child_encoding =
                header_value(headers, "content-transfer-encoding").unwrap_or_default();
            let child_id = if prefix.is_empty() {
                format!("{}", index + 1)
            } else {
                format!("{prefix}.{}", index + 1)
            };
            walk_attachments(
                body,
                &child_type,
                &child_disposition,
                &child_encoding,
                &child_id,
                visit,
            );
        }
        return;
    }

    // A leaf. The top level itself is a leaf when the message is not
    // multipart; it walks under the id "1".
    let id = if prefix.is_empty() { "1".to_owned() } else { prefix.to_owned() };

    let (disposition, disposition_params) = parse_content_type(content_disposition);
    let mut params = disposition_params;
    params.extend(type_params.clone());
    // `name=` on Content-Type is the legacy spelling of the same fact.
    if filename_from_params(&params).is_none() {
        if let Some(name) = param(&type_params, "name") {
            params.push(("filename".to_owned(), name));
        }
    }

    let filename = filename_from_params(&params);
    let is_attachment = filename.is_some() || disposition == "attachment";
    if !is_attachment {
        return;
    }

    let media_type = if mime_type.is_empty() {
        "application/octet-stream".to_owned()
    } else {
        mime_type
    };
    let info = MimeAttachment {
        filename: filename.unwrap_or_else(|| format!("attachment-{id}.bin")),
        media_type,
        size_bytes: decoded_size_estimate(raw, transfer_encoding),
        inline: disposition == "inline",
        id,
    };
    visit(info, (raw, transfer_encoding));
}

/// Decoded size without decoding: exact for base64 (count the alphabet
/// characters), the raw length otherwise.
fn decoded_size_estimate(raw: &str, transfer_encoding: &str) -> usize {
    if transfer_encoding.trim().eq_ignore_ascii_case("base64") {
        let meaningful = raw
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '=')
            .count();
        meaningful * 3 / 4
    } else {
        raw.len()
    }
}
```

Note: `parse_content_type` also parses `Content-Disposition` (same
`token; key=value` syntax) — that reuse is deliberate. `filename*` parameters
keep their `*` in the key because `parse_content_type` lowercases but does not
strip it, which is exactly what `filename_from_params` expects.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p torromail-core`
Expected: all tests PASS, including the five new ones and every pre-existing
mime test (the renderer is untouched).

- [ ] **Step 5: Commit**

```bash
git add crates/torromail-core/src/mime.rs
git commit -m "feat(core): walk MIME trees for attachment listing and extraction"
```

---

### Task 3: Domain model — attachments on `StoredMessage`, provider method, service gate

**Files:**
- Modify: `crates/torromail-core/src/lib.rs`
- Test: `crates/torromail-core/tests/core_contract.rs`

**Interfaces:**
- Produces (all in `torromail_core`, all `pub`):

```rust
pub struct AttachmentInfo { … }  // new(id, filename, media_type, size_bytes, inline) + getters id(), filename(), media_type(), size_bytes(), inline()
impl StoredMessage {
    pub fn with_attachment_infos(self, attachments: Vec<AttachmentInfo>) -> Self;
    pub fn attachments(&self) -> &[AttachmentInfo];
}
pub struct AttachmentPayload { … } // new(mailbox, filename, media_type, content) + getters mailbox(), filename(), media_type(), content() -> &[u8]
pub enum CoreError { …, AttachmentNotFound { message_id: String, attachment_id: String } }
pub trait MailProvider { …, fn get_attachment(&self, account_id: &AccountId, message_id: &str, attachment_id: &str) -> CoreResult<AttachmentPayload>; }
impl FixtureMailProvider {
    pub fn with_attachment(self, message_id: &str, attachment_id: &str, filename: &str, media_type: &str, content: Vec<u8>) -> Self;
}
impl MailAccessService {
    pub fn get_attachment(&self, account_id: &AccountId, message_id: &str, attachment_id: &str) -> CoreResult<AttachmentPayload>;
    pub fn download_allowed(&self, account_id: &AccountId, mailbox: &str) -> bool;
}
pub use mime::encode_base64;  // the MCP layer inlines small files as base64
```

- Consumes: existing `Capability::DownloadAttachments`, `PolicyEngine`,
  `CoreError` display conventions.

- [ ] **Step 1: Write the failing tests** (append to `core_contract.rs`;
  match the file's existing helpers for building policies — read its head
  first and reuse its `PolicyEngine`/`Policy` construction style)

```rust
#[test]
fn attachment_download_needs_the_attachment_read_level() {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account.clone(), "INBOX", "m1", "t1", "Mit Anhang", "a@example.com", "s", "b",
    )
    .with_attachment_infos(vec![AttachmentInfo::new(
        "2", "angebot.pdf", "application/pdf", 4, false,
    )]);
    let provider = FixtureMailProvider::new([message])
        .with_attachment("m1", "2", "angebot.pdf", "application/pdf", b"%PDF".to_vec());

    // full_message may list but not download.
    let mut permissions = PermissionSet::default();
    permissions.read = ReadAccess::FullMessage;
    let engine = PolicyEngine::new([Policy::new(account.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(&provider, engine, &mut sessions);

    let listed = service.get_message(&account, "m1", false).expect("readable");
    assert_eq!(listed.attachments().len(), 1);
    assert_eq!(listed.attachments()[0].filename(), "angebot.pdf");
    assert!(!service.download_allowed(&account, "INBOX"));
    assert!(service.get_attachment(&account, "m1", "2").is_err());

    // with_attachments downloads.
    let mut permissions = PermissionSet::default();
    permissions.read = ReadAccess::WithAttachments;
    let engine = PolicyEngine::new([Policy::new(account.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(&provider, engine, &mut sessions);

    assert!(service.download_allowed(&account, "INBOX"));
    let payload = service.get_attachment(&account, "m1", "2").expect("allowed");
    assert_eq!(payload.mailbox(), "INBOX");
    assert_eq!(payload.filename(), "angebot.pdf");
    assert_eq!(payload.content(), b"%PDF");
}

#[test]
fn a_blocked_folder_blocks_the_download_too() {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account.clone(), "Geheim", "m1", "t1", "S", "a@example.com", "s", "b",
    );
    let provider = FixtureMailProvider::new([message])
        .with_attachment("m1", "1", "x.bin", "application/octet-stream", vec![0]);

    let mut permissions = PermissionSet::default();
    permissions.read = ReadAccess::WithAttachments;
    permissions.per_folder = true;
    permissions.folder_rules.insert(
        "Geheim".to_owned(),
        FolderRule { read: false, write: true },
    );
    let engine = PolicyEngine::new([Policy::new(account.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(&provider, engine, &mut sessions);

    assert!(service.get_attachment(&account, "m1", "1").is_err());
}

#[test]
fn an_unknown_attachment_id_is_a_named_error() {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account.clone(), "INBOX", "m1", "t1", "S", "a@example.com", "s", "b",
    );
    let provider = FixtureMailProvider::new([message]);

    let error = provider
        .get_attachment(&account, "m1", "7")
        .expect_err("nothing seeded");
    assert!(error.to_string().contains('7'));
    assert!(error.to_string().contains("m1"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p torromail-core --test core_contract attachment`
Expected: compile errors (`AttachmentInfo` etc. not found).

- [ ] **Step 3: Implement in `lib.rs`**

1. `AttachmentInfo` next to `StoredMessage` (private fields, `new` +
   getters, `#[derive(Debug, Clone, PartialEq, Eq)]`); `StoredMessage`
   gains `attachments: Vec<AttachmentInfo>` (initialized empty in `new`),
   the `with_attachment_infos` builder, and the `attachments()` getter.
2. `AttachmentPayload` (same derive/getter style; `content() -> &[u8]`).
3. `CoreError::AttachmentNotFound { message_id, attachment_id }` with the
   Display arm `attachment {attachment_id} not found on message {message_id}`
   — place both alongside `MessageNotFound`.
4. Trait method on `MailProvider` (doc comment: "The decoded bytes of one
   attachment, id as listed on the stored message.") plus the forwarding arm
   in the blanket `impl<T: MailProvider + ?Sized> MailProvider for &mut T`.
5. `FixtureMailProvider`: add field
   `attachment_payloads: BTreeMap<(String, String), (String, String, Vec<u8>)>`
   (key: message id + attachment id; value: filename, media type, bytes),
   default empty in `new`; the `with_attachment` builder inserts. Its
   `get_attachment` finds the message first (for account guard + mailbox;
   `MessageNotFound` otherwise), then the payload (`AttachmentNotFound`
   otherwise), and builds `AttachmentPayload::new(message.mailbox(), …)`.
6. `MailAccessService::get_attachment`: `authorize(account_id,
   Capability::DownloadAttachments)` first (the cheap account-wide refusal),
   then `provider.get_attachment(…)`, then `authorize_in(account_id,
   payload.mailbox(), Capability::DownloadAttachments)` before returning —
   bytes never leave the service unauthorized. `download_allowed` is
   `self.policy_engine.authorize_in(account_id, mailbox,
   Capability::DownloadAttachments).is_ok()`.
7. `pub use mime::encode_base64;` near the existing mime re-exports (make
   `encode_base64` `pub(crate)` → `pub` inside `mime.rs` only if the
   re-export requires it; keep the narrower visibility if `pub use` of a
   `pub(crate)` item fails to compile — it will, so widen to `pub` in
   `mime.rs` and export).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p torromail-core`
Expected: PASS. Then `cargo test` (workspace) — expect compile failures only
in `torromail-mcp` if its test doubles implement `MailProvider`; fix the
`FailingOnce` double in `crates/torromail-mcp/tests/tool_contract.rs` by
delegating:

```rust
    fn get_attachment(
        &self,
        account_id: &AccountId,
        message_id: &str,
        attachment_id: &str,
    ) -> CoreResult<AttachmentPayload> {
        self.inner.get_attachment(account_id, message_id, attachment_id)
    }
```

(import `AttachmentPayload` in that file's `torromail_core` use list). Run
`cargo test` again: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/torromail-core/src/lib.rs crates/torromail-core/tests/core_contract.rs crates/torromail-mcp/tests/tool_contract.rs
git commit -m "feat(core): attachment listing on messages and a policy-gated download path"
```

---

### Task 4: IMAP provider — keep the raw body, list and extract attachments

**Files:**
- Modify: `crates/torromail-core/src/imap_provider.rs`
- Test: `crates/torromail-core/tests/imap_contract.rs`

**Interfaces:**
- Consumes: Task 2's `mime::list_attachments` / `mime::extract_attachment_bytes`
  (they are `pub(crate)`, reachable from this module), Task 3's
  `AttachmentInfo` / `AttachmentPayload` / `CoreError::AttachmentNotFound`.
- Produces: `FetchedMessage` gains `raw_body: String`, `content_type: String`,
  `transfer_encoding: String`; `ImapMailProvider` implements
  `get_attachment`. The FETCH command strings do not change — existing
  scripted tests keep matching.

- [ ] **Step 1: Write the failing test** (append to `imap_contract.rs`;
  reuse the file's scripted-transport helper — read its existing `uid_fetch`
  test first and copy its script/answer structure, replacing the body literal
  with this multipart)

The scripted FETCH answer's second literal (the `BODY[TEXT]` one) becomes:

```text
--outer\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBitte den Anhang pruefen.\r\n--outer\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=angebot.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n--outer--\r\n
```

and the first literal (the headers) carries
`Content-Type: multipart/mixed; boundary=outer` alongside Subject/From/Date.

```rust
#[test]
fn a_fetched_message_lists_and_serves_its_attachments() {
    // Build the provider exactly as the existing fetch test does, with the
    // multipart literals above, for message id "INBOX/7".
    let provider = scripted_provider_with_multipart_fetch();

    let message = provider
        .get_message(&AccountId::new("work"), "INBOX/7")
        .expect("fetch works");
    assert_eq!(message.body(), "Bitte den Anhang pruefen.");
    assert_eq!(message.attachments().len(), 1);
    let info = &message.attachments()[0];
    assert_eq!(info.id(), "2");
    assert_eq!(info.filename(), "angebot.pdf");
    assert_eq!(info.media_type(), "application/pdf");
    assert_eq!(info.size_bytes(), 8); // "JVBERi0xLjQ=" → "%PDF-1.4", 8 bytes

    let payload = provider
        .get_attachment(&AccountId::new("work"), "INBOX/7", "2")
        .expect("extraction works");
    assert_eq!(payload.mailbox(), "INBOX");
    assert_eq!(payload.content(), b"%PDF-1.4");

    let missing = provider
        .get_attachment(&AccountId::new("work"), "INBOX/7", "5")
        .expect_err("no such part");
    assert!(matches!(missing, CoreError::AttachmentNotFound { .. }));
}
```

(`scripted_provider_with_multipart_fetch` is a local helper in the test file:
the same transport script as the existing fetch test, with the two literals
replaced and enough scripted responses that `get_attachment`'s second
`uid_fetch` — SELECT plus UID FETCH — is answered too.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p torromail-core --test imap_contract attachments`
Expected: compile error (`attachments()` exists but `get_attachment` is
unimplemented for `ImapMailProvider` — the trait obligation from Task 3 makes
this file fail to compile even before the new test runs; that compile error IS
the failure signal here, and it already blocks `cargo test` at the end of
Task 3 — implement Task 4 immediately after).

*Correction for honest sequencing:* Task 3's Step 4 workspace run would
already fail to compile because `ImapMailProvider` misses the new trait
method. To keep every commit green, Task 3's implementation Step includes a
minimal `ImapMailProvider::get_attachment` stub that returns
`Err(CoreError::AttachmentNotFound { … })`, and this task replaces the stub
with the real implementation. The stub is honest (no fetch path exists yet)
and the Task 4 test drives the real one.

- [ ] **Step 3: Implement**

In `uid_fetch`, keep what today is dropped:

```rust
        Ok(FetchedMessage {
            uid,
            subject: header_field(&headers, "subject").unwrap_or_default(),
            sender: header_field(&headers, "from").unwrap_or_default(),
            date: header_field(&headers, "date").unwrap_or_default(),
            body,
            raw_body,
            content_type,
            transfer_encoding,
            seen: fetch.text.contains("\\Seen"),
            flagged: fetch.text.contains("\\Flagged"),
        })
```

(move the `content_type`/`transfer_encoding` locals up so they can be stored;
`raw_body` is the second literal already read). Add the three fields to the
`FetchedMessage` struct.

In `ImapMailProvider::get_message`, after building the message:

```rust
        let attachments = crate::mime::list_attachments(
            &fetched.raw_body,
            &fetched.content_type,
            &fetched.transfer_encoding,
        )
        .into_iter()
        .map(|part| {
            AttachmentInfo::new(part.id, part.filename, part.media_type, part.size_bytes, part.inline)
        })
        .collect();
        let message = message.with_attachment_infos(attachments);
```

Replace the Task 3 stub of `get_attachment`:

```rust
    fn get_attachment(
        &self,
        account_id: &AccountId,
        message_id: &str,
        attachment_id: &str,
    ) -> CoreResult<AttachmentPayload> {
        self.guard(account_id)?;
        let (mailbox, uid) = self.split_message_id(message_id)?;
        let fetched = self.client.borrow_mut().uid_fetch(mailbox, uid)?;
        let (info, bytes) = crate::mime::extract_attachment_bytes(
            &fetched.raw_body,
            &fetched.content_type,
            &fetched.transfer_encoding,
            attachment_id,
        )
        .ok_or_else(|| CoreError::AttachmentNotFound {
            message_id: message_id.to_owned(),
            attachment_id: attachment_id.to_owned(),
        })?;
        Ok(AttachmentPayload::new(mailbox, info.filename, info.media_type, bytes))
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p torromail-core`
Expected: PASS including the new scripted test and all pre-existing IMAP
contract tests (unchanged FETCH strings).

- [ ] **Step 5: Commit**

```bash
git add crates/torromail-core/src/imap_provider.rs crates/torromail-core/tests/imap_contract.rs
git commit -m "feat(core): IMAP attachment listing and extraction from the kept raw body"
```

---

### Task 5: MCP tool `mail_get_attachment` + attachment listing in message JSON

**Files:**
- Modify: `crates/torromail-mcp/src/lib.rs`
- Test: `crates/torromail-mcp/tests/tool_contract.rs`

**Interfaces:**
- Consumes: Task 3's service API (`get_attachment`, `download_allowed`),
  `torromail_core::encode_base64`, `AttachmentPayload`, `AttachmentInfo`.
- Produces:
  - `ToolName::MailGetAttachment` ↔ `"mail_get_attachment"`, catalog entry
    with `AccessLevel::Read`, `requires_gui_confirmation: false`, description
    "Download one attachment to a TorroMail-managed folder and answer with
    its absolute path. Needs the 'Message and attachments' read level."
  - Input schema:
    `{"type":"object","properties":{"account_id":{"type":"string"},"message_id":{"type":"string"},"attachment_id":{"type":"string","description":"From the attachments list of mail_get_message."},"include_content":{"type":"boolean","description":"Also inline the bytes as base64 when the file is 2 MiB or smaller."}},"required":["account_id","message_id","attachment_id"]}`
  - `message_json(message, download_allowed)` — every message answer gains
    `attachments` (array of `{attachment_id, filename, media_type,
    size_bytes, inline, download_allowed}`) and `attachment_count`.
  - Constants `MAX_ATTACHMENT_DOWNLOAD_BYTES: usize = 50 * 1024 * 1024`,
    `MAX_INLINE_CONTENT_BYTES: usize = 2 * 1024 * 1024`.
  - `ToolFailure::Io(String)` → `-32000`, no health outcome, not a
    connection failure.
  - `LineMcpServer.attachments_dir: Option<PathBuf>`, derived in
    `with_policy_path` as `path.parent().map(|dir| dir.join("attachments"))`,
    `None` in `fixture()`.
  - `fn sanitize_component(raw: &str) -> String` (for account/message dir
    names: path separators, NUL and leading dots replaced by `-`, 255-byte
    cap, empty → `"item"`), reused for filenames.

- [ ] **Step 1: Write the failing tests** (append to `tool_contract.rs`)

```rust
/// A message with one PDF attachment, plus its payload bytes.
fn mailbox_with_attachment() -> FixtureMailProvider {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account, "INBOX", "m1", "thread-1", "Mit Anhang", "a@example.com", "s", "body",
    )
    .with_attachment_infos(vec![torromail_core::AttachmentInfo::new(
        "2", "angebot.pdf", "application/pdf", 8, false,
    )]);
    FixtureMailProvider::new([message]).with_attachment(
        "m1", "2", "angebot.pdf", "application/pdf", b"%PDF-1.4".to_vec(),
    )
}

fn attachment_document(path: &std::path::Path, read: &str) {
    std::fs::write(
        path,
        format!(
            r#"{{"version":1,"accounts":[{{"id":"work","read":"{read}","write":{{"drafts":true}},"send":false,"per_folder":false,"folder_rules":{{}}}}]}}"#
        ),
    )
    .expect("policy document written");
}

const GET_MESSAGE_WITH_BODY: &str = r#"{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"mail_get_message","arguments":{"account_id":"work","message_id":"m1","include_body":true}}}"#;
const GET_ATTACHMENT: &str = r#"{"jsonrpc":"2.0","id":61,"method":"tools/call","params":{"name":"mail_get_attachment","arguments":{"account_id":"work","message_id":"m1","attachment_id":"2"}}}"#;
const GET_ATTACHMENT_INLINE: &str = r#"{"jsonrpc":"2.0","id":62,"method":"tools/call","params":{"name":"mail_get_attachment","arguments":{"account_id":"work","message_id":"m1","attachment_id":"2","include_content":true}}}"#;

#[test]
fn get_message_lists_attachments_and_their_download_right() {
    let path = temp_policy_path("attachment-listing");
    attachment_document(&path, "full_message");
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account| {
        Ok(Box::new(mailbox_with_attachment()) as Box<dyn MailProvider>)
    });

    let response = server.handle_line(GET_MESSAGE_WITH_BODY).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#"\"attachment_count\":1"#), "got: {response}");
    assert!(response.contains("angebot.pdf"), "got: {response}");
    assert!(response.contains(r#"\"download_allowed\":false"#), "got: {response}");
}

#[test]
fn downloading_needs_the_attachment_read_level() {
    let path = temp_policy_path("attachment-denied");
    attachment_document(&path, "full_message");
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account| {
        Ok(Box::new(mailbox_with_attachment()) as Box<dyn MailProvider>)
    });

    let response = server.handle_line(GET_ATTACHMENT).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains("error"), "got: {response}");
    assert!(response.contains("-32000"), "got: {response}");
}

#[test]
fn downloading_writes_the_file_and_answers_its_path() {
    let path = temp_policy_path("attachment-download");
    attachment_document(&path, "with_attachments");
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account| {
        Ok(Box::new(mailbox_with_attachment()) as Box<dyn MailProvider>)
    });

    let response = server.handle_line(GET_ATTACHMENT).expect("a response");
    let inline = server.handle_line(GET_ATTACHMENT_INLINE).expect("a response");

    let expected_file = path
        .parent()
        .expect("temp dir")
        .join("attachments/work/m1/2-angebot.pdf");
    let written = std::fs::read(&expected_file).expect("file written");
    std::fs::remove_file(&path).ok();
    std::fs::remove_dir_all(path.parent().expect("temp dir").join("attachments")).ok();

    assert_eq!(written, b"%PDF-1.4");
    assert!(response.contains(r#"\"filename\":\"angebot.pdf\""#), "got: {response}");
    assert!(response.contains("2-angebot.pdf"), "got: {response}");
    assert!(!response.contains("content_base64"), "got: {response}");
    // include_content inlines the bytes: "%PDF-1.4" → JVBERi0xLjQ=
    assert!(inline.contains("JVBERi0xLjQ="), "got: {inline}");
}

#[test]
fn an_unknown_attachment_id_answers_a_named_error() {
    let path = temp_policy_path("attachment-missing");
    attachment_document(&path, "with_attachments");
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account| {
        Ok(Box::new(mailbox_with_attachment()) as Box<dyn MailProvider>)
    });

    let request = r#"{"jsonrpc":"2.0","id":63,"method":"tools/call","params":{"name":"mail_get_attachment","arguments":{"account_id":"work","message_id":"m1","attachment_id":"9"}}}"#;
    let response = server.handle_line(request).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains("error"), "got: {response}");
    assert!(response.contains('9'), "got: {response}");
}
```

Also update the existing catalog-shape test in this file (the one asserting
the tool list / count — search for `fourteen`, `14`, or a full names list) to
include `mail_get_attachment`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p torromail-mcp --test tool_contract attachment`
Expected: compile error (`with_attachment_infos` import fine, but
`mail_get_attachment` unknown → the catalog test and handlers fail; the tool
answers "tool not found").

- [ ] **Step 3: Implement in `crates/torromail-mcp/src/lib.rs`**

1. `ToolName::MailGetAttachment` (enum, `as_str`, and its arm in
   `canonical_tools()` right after `MailGetThread`), schema and description
   as specified above.
2. Server field `attachments_dir` (see Interfaces) — set in
   `with_policy_path`, `None` in `fixture()`.
3. Dispatch arm after the `MailGetThread` one:

```rust
        if name == ToolName::MailGetAttachment.as_str() {
            return self.run_with_connection(&account_id, id, &|provider, engine| {
                handle_mail_get_attachment(
                    arguments,
                    &account_id,
                    provider,
                    engine,
                    self.attachments_dir.as_deref(),
                )
            });
        }
```

4. The handler:

```rust
/// Download one attachment into the TorroMail-managed store and answer with
/// the absolute path — never a client-chosen destination. Bytes ride along
/// as base64 only on request and only for small files.
fn handle_mail_get_attachment(
    arguments: &Value,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
    attachments_dir: Option<&std::path::Path>,
) -> ToolResult {
    let message_id = arguments["message_id"].as_str().unwrap_or_default();
    let attachment_id = arguments["attachment_id"].as_str().unwrap_or_default();
    let include_content = arguments["include_content"].as_bool().unwrap_or(false);
    if message_id.is_empty() || attachment_id.is_empty() {
        return Err(ToolFailure::InvalidParams(
            "message_id and attachment_id are required".to_owned(),
        ));
    }
    let Some(base_dir) = attachments_dir else {
        return Err(ToolFailure::Io(
            "no attachment store: the server is running without a policy path".to_owned(),
        ));
    };

    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(provider, engine, &mut sessions);
    let payload = service
        .get_attachment(account_id, message_id, attachment_id)
        .map_err(ToolFailure::Core)?;

    if payload.content().len() > MAX_ATTACHMENT_DOWNLOAD_BYTES {
        return Err(ToolFailure::InvalidParams(format!(
            "attachment is {} bytes; the limit is {MAX_ATTACHMENT_DOWNLOAD_BYTES} — fetch it in a mail client instead",
            payload.content().len()
        )));
    }

    let file_name = format!(
        "{}-{}",
        sanitize_component(attachment_id),
        sanitize_component(payload.filename())
    );
    let directory = base_dir
        .join(sanitize_component(account_id.as_str()))
        .join(sanitize_component(message_id));
    std::fs::create_dir_all(&directory)
        .map_err(|error| ToolFailure::Io(format!("cannot create attachment store: {error}")))?;
    let target = directory.join(&file_name);
    std::fs::write(&target, payload.content())
        .map_err(|error| ToolFailure::Io(format!("cannot write attachment: {error}")))?;

    let mut answer = json!({
        "message_id": message_id,
        "attachment_id": attachment_id,
        "filename": payload.filename(),
        "media_type": payload.media_type(),
        "size_bytes": payload.content().len(),
        "path": target.to_string_lossy(),
        "from_cache": false
    });
    if include_content {
        if payload.content().len() <= MAX_INLINE_CONTENT_BYTES {
            answer["content_base64"] = json!(torromail_core::encode_base64(payload.content()));
        } else {
            answer["content_note"] = json!(format!(
                "file exceeds the {MAX_INLINE_CONTENT_BYTES}-byte inline limit; read it from `path`"
            ));
        }
    }
    Ok(answer)
}

/// One path component, defused: separators, NUL and leading dots cannot
/// escape the store or hide the file; 255 bytes is the filesystem's own cap.
fn sanitize_component(raw: &str) -> String {
    let mut cleaned: String = raw
        .chars()
        .map(|character| match character {
            '/' | '\\' | '\0' => '-',
            _ => character,
        })
        .collect();
    while cleaned.starts_with('.') {
        cleaned.remove(0);
    }
    while cleaned.len() > 255 {
        cleaned.pop();
    }
    if cleaned.is_empty() {
        cleaned.push_str("item");
    }
    cleaned
}
```

5. `ToolFailure::Io(String)`: extend the enum plus `is_connection` (false —
   the existing match already answers false via its pattern), a `None` arm in
   `health_outcome`, the message in `message()`, and `-32000` in
   `into_response`.
6. `message_json(message: &StoredMessage, download_allowed: bool)` — extend
   with:

```rust
        "attachments": message.attachments().iter().map(|info| json!({
            "attachment_id": info.id(),
            "filename": info.filename(),
            "media_type": info.media_type(),
            "size_bytes": info.size_bytes(),
            "inline": info.inline(),
            "download_allowed": download_allowed
        })).collect::<Vec<_>>(),
        "attachment_count": message.attachments().len(),
```

   Callers: `handle_mail_get_message` computes
   `let download_allowed = service.download_allowed(account_id, message.mailbox());`
   after the fetch; `handle_mail_get_thread` computes it per message the same
   way.
7. Constants and imports (`AttachmentPayload` is not needed by name here —
   the payload stays local; import `SearchSessionStore` is already there).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p torromail-mcp`
Expected: PASS — the four new tests, the adjusted catalog test, and every
pre-existing test (message JSON gained fields; no test asserts an exact full
message object, but if one does, extend its expectation).

- [ ] **Step 5: Commit**

```bash
git add crates/torromail-mcp/src/lib.rs crates/torromail-mcp/tests/tool_contract.rs
git commit -m "feat(mcp): mail_get_attachment downloads into the managed store"
```

---

### Task 6: Startup sweep for stale files + main wiring + spec amendment

**Files:**
- Modify: `crates/torromail-mcp/src/lib.rs`, `crates/torromail-mcp/src/main.rs`,
  `docs/superpowers/specs/2026-08-07-attachments-and-real-cache-design.md`
- Test: `crates/torromail-mcp/tests/tool_contract.rs` (unit-style, same file)

**Interfaces:**
- Produces: `pub fn sweep_attachment_files(policy_path: Option<std::path::PathBuf>)`
  (derives `<parent>/attachments`, 24 h TTL) and the testable
  `pub fn sweep_attachment_dir(dir: &std::path::Path, ttl: std::time::Duration)`.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_attachment_sweep_removes_old_files_and_prunes_empty_dirs() {
    let root = std::env::temp_dir().join(format!("torromail-sweep-{}", std::process::id()));
    let nested = root.join("work").join("m1");
    std::fs::create_dir_all(&nested).expect("store created");
    let file = nested.join("2-angebot.pdf");
    std::fs::write(&file, b"x").expect("file written");

    // A generous TTL keeps a fresh file.
    torromail_mcp::sweep_attachment_dir(&root, std::time::Duration::from_secs(60 * 60));
    assert!(file.exists());

    // TTL zero: everything is old; files go, empty directories follow.
    torromail_mcp::sweep_attachment_dir(&root, std::time::Duration::ZERO);
    assert!(!file.exists());
    assert!(!nested.exists());

    std::fs::remove_dir_all(&root).ok();
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p torromail-mcp --test tool_contract sweep`
Expected: compile error — `sweep_attachment_dir` not found.

- [ ] **Step 3: Implement**

In `lib.rs` (near `sweep_account_health`):

```rust
/// Phase-1 housekeeping for the attachment store: downloaded files are the
/// client's deliverable, not yet a cache, so anything older than the TTL is
/// deleted on server start and emptied directories go with it. Best-effort
/// throughout — a file the sweep cannot stat or remove is left for the next
/// start.
pub fn sweep_attachment_files(policy_path: Option<std::path::PathBuf>) {
    let Some(dir) = policy_path
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(|parent| parent.join("attachments"))
    else {
        return;
    };
    sweep_attachment_dir(&dir, std::time::Duration::from_secs(24 * 60 * 60));
}

/// The sweep itself, TTL injected so tests need not fake file ages.
pub fn sweep_attachment_dir(dir: &std::path::Path, ttl: std::time::Duration) {
    let now = std::time::SystemTime::now();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sweep_attachment_dir(&path, ttl);
            // Empty after its own sweep? Then it only held stale files.
            if std::fs::read_dir(&path).map(|mut rest| rest.next().is_none()).unwrap_or(false) {
                let _ = std::fs::remove_dir(&path);
            }
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= ttl);
        if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}
```

In `main.rs`, extend the existing sweep thread body:

```rust
    std::thread::spawn(move || {
        torromail_mcp::sweep_attachment_files(sweep_policy_path.clone());
        torromail_mcp::sweep_account_health(sweep_policy_path, sweep_token.as_deref());
    });
```

In the spec, replace the collision sentence ("a collision appends a numeric
suffix") in **Files on disk** with the implemented rule: files are stored as
`<attachment_id>-<filename>` inside the message directory, which makes
re-downloads idempotent and same-named attachments collision-free; adjust the
example path to `…/attachments/gmail/4711/2-angebot.pdf`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p torromail-mcp && cargo build -p torromail-mcp`
Expected: PASS / builds.

- [ ] **Step 5: Commit**

```bash
git add crates/torromail-mcp/src/lib.rs crates/torromail-mcp/src/main.rs docs/superpowers/specs/2026-08-07-attachments-and-real-cache-design.md
git commit -m "feat(mcp): sweep stale attachment files on server start"
```

---

### Task 7: Docs, app log label, localization, full verification

**Files:**
- Modify: `AGENTS.md`, `docs/architecture.md`,
  `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`,
  `apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj/Localizable.strings`

**Interfaces:** none — documentation and labels only.

- [ ] **Step 1: AGENTS.md** — in "Current MCP Tool Surface", add under the
  read/search list:

```markdown
- `mail_get_attachment` — downloads one attachment (listed by
  `mail_get_message`) into `~/Library/Application Support/TorroMail/attachments/`
  and answers with the absolute path; gated by the "Message and attachments"
  read level. Clients never choose destination paths.
```

  and change "All fourteen catalog tools are implemented." to "All fifteen
  catalog tools are implemented."

- [ ] **Step 2: docs/architecture.md** — add `mail_get_attachment` to the
  "MCP Surface" read list, and append a short section after **Outgoing
  Attachments**:

```markdown
## Incoming Attachments

`mail_get_message` lists a message's attachments (id, filename, media type,
decoded size, inline flag) from the "Full message" read level; the bytes
need "Message and attachments". `mail_get_attachment` writes the decoded
file to `attachments/<account>/<message>/<id>-<filename>` under the shared
Application Support directory and answers with the absolute path — clients
never pass destination paths, mirroring the rule on the outgoing side.
`include_content: true` additionally inlines base64 up to 2 MiB; downloads
are capped at 50 MiB. Files older than 24 hours are swept on server start
until the cache (spec phase 2) starts retaining them.
```

- [ ] **Step 3: App label** — in `TorroMailApp.swift`, the audit-log label
  switch (near `case "mail_get_cache_status"`), add:

```swift
    case "mail_get_attachment": L("Downloaded an attachment")
```

  and in `de.lproj/Localizable.strings` (alphabetical near the other tool
  labels):

```text
"Downloaded an attachment" = "Anhang heruntergeladen";
```

- [ ] **Step 4: Full verification** (all four commands from AGENTS.md)

```bash
cargo test
cargo build -p torromail-mcp
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: everything green.

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md docs/architecture.md apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj/Localizable.strings
git commit -m "docs(app): document mail_get_attachment and label it in the log"
```

---

## Self-review notes

- Spec coverage: listing (Task 4/5), tool + path answer + `include_content`
  (Task 5), sanitization + caps (Task 5), rights + folder scope (Task 3),
  audit/health ride on `run_with_connection` for free (no task needed),
  sweep (Task 6), docs (Task 7). `from_cache` is emitted as `false` — the
  cache arrives in phase 2.
- Deviation from spec, recorded in Task 6: `<id>-<filename>` storage names
  instead of numeric collision suffixes.
- Task 3/4 sequencing: the trait method lands with an honest IMAP stub in
  Task 3 so every commit compiles; Task 4 replaces it test-first.
