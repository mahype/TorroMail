//! Turning a raw IMAP body into readable text.
//!
//! `BODY[TEXT]` hands the body over exactly as it travels the wire: MIME
//! parts fenced by boundaries, each in its own transfer encoding, the words
//! usually buried inside HTML. An assistant needs the words, not the
//! envelope. This module unwraps a message to the best plain-text rendering
//! it can find and leaves anything it does not understand alone — a part it
//! cannot decode contributes nothing rather than raising an error.

/// The readable text of a body, given the raw `BODY[TEXT]` and the top-level
/// `Content-Type` / `Content-Transfer-Encoding` headers.
pub(crate) fn body_to_text(raw: &str, content_type: &str, transfer_encoding: &str) -> String {
    let rendered = render_part(raw, content_type, transfer_encoding);
    collapse_blank_lines(rendered.trim())
}

/// One MIME entity to text. A multipart entity picks its best child; a leaf
/// decodes its transfer encoding and charset; a non-text leaf — an
/// attachment — is not body text and renders empty.
fn render_part(raw: &str, content_type: &str, transfer_encoding: &str) -> String {
    let (mime_type, params) = parse_content_type(content_type);

    if mime_type.starts_with("multipart/") {
        return match param(&params, "boundary") {
            Some(boundary) => render_multipart(raw, &boundary),
            None => String::new(),
        };
    }

    let bytes = decode_transfer(raw, transfer_encoding);
    let charset = param(&params, "charset").unwrap_or_else(|| "utf-8".to_owned());
    let text = decode_charset(&charset, &bytes)
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());

    if mime_type == "text/html" {
        html_to_text(&text)
    } else if mime_type.is_empty() || mime_type.starts_with("text/") {
        // A missing Content-Type means text/plain (RFC 2045 §5.2).
        text
    } else {
        String::new()
    }
}

/// Pick the most readable child of a multipart entity: `text/plain` beats a
/// nested multipart's own choice, which beats `text/html`; an attachment
/// scores nothing. The same ranking serves `multipart/alternative` (best of
/// equals) and `multipart/mixed` (the body, not its attachments).
fn render_multipart(raw: &str, boundary: &str) -> String {
    let mut best_rank = 0;
    let mut best = String::new();

    for segment in parts(raw, boundary) {
        let (headers, body) = split_headers_body(segment);
        let content_type = header_value(headers, "content-type").unwrap_or_default();
        let transfer_encoding =
            header_value(headers, "content-transfer-encoding").unwrap_or_default();
        let (mime_type, _) = parse_content_type(&content_type);

        let rendered = render_part(body, &content_type, &transfer_encoding);
        if rendered.trim().is_empty() {
            continue;
        }

        let rank = match mime_type.as_str() {
            "text/plain" => 3,
            _ if mime_type.starts_with("multipart/") => 2,
            "text/html" => 1,
            _ => continue,
        };
        if rank > best_rank {
            best_rank = rank;
            best = rendered;
        }
    }
    best
}

/// The body segments between `--boundary` fences, without the preamble before
/// the first fence or the epilogue after the closing `--boundary--`.
fn parts<'a>(raw: &'a str, boundary: &str) -> Vec<&'a str> {
    let delimiter = format!("--{boundary}");
    let mut segments = Vec::new();
    let mut pieces = raw.split(delimiter.as_str());
    let _preamble = pieces.next();
    for piece in pieces {
        // The closing fence is `--boundary--`, so the piece right after the
        // last real boundary starts with `--`; everything past it is
        // epilogue.
        if piece.starts_with("--") {
            break;
        }
        segments.push(piece.trim_start_matches(['\r', '\n']));
    }
    segments
}

/// Split one MIME part at its header/body blank line.
fn split_headers_body(segment: &str) -> (&str, &str) {
    if let Some(index) = segment.find("\r\n\r\n") {
        (&segment[..index], &segment[index + 4..])
    } else if let Some(index) = segment.find("\n\n") {
        (&segment[..index], &segment[index + 2..])
    } else {
        ("", segment)
    }
}

/// First value of a part header, unfolded, case-insensitive. Part headers
/// (`Content-Type`, `Content-Transfer-Encoding`) are plain tokens, never
/// RFC 2047 encoded-words, so this does no word decoding.
fn header_value(headers: &str, name: &str) -> Option<String> {
    let mut lines = headers.lines();
    while let Some(line) = lines.next() {
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        if !field.trim().eq_ignore_ascii_case(name) {
            continue;
        }

        let mut value = value.trim().to_owned();
        for continuation in lines.by_ref() {
            if !continuation.starts_with([' ', '\t']) {
                break;
            }
            value.push(' ');
            value.push_str(continuation.trim());
        }
        return Some(value);
    }
    None
}

/// `text/html; charset="utf-8"; boundary=xyz` → the lowercased type and its
/// parameters, quotes stripped.
fn parse_content_type(content_type: &str) -> (String, Vec<(String, String)>) {
    let mut pieces = content_type.split(';');
    let mime_type = pieces.next().unwrap_or("").trim().to_ascii_lowercase();
    let params = pieces
        .filter_map(|piece| {
            let (key, value) = piece.split_once('=')?;
            Some((
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').to_owned(),
            ))
        })
        .collect();
    (mime_type, params)
}

fn param(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

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
        let joined: String = segments.into_iter().map(|(_, value)| value).collect();
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
    if after_charset.get(1..2) != Some("?") {
        return None;
    }
    let text = after_charset.get(2..)?;
    let text_end = text.find("?=")?;
    let payload = &text[..text_end];

    let bytes = match encoding.to_ascii_lowercase() {
        'b' => decode_base64(
            &payload
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect::<String>(),
        )?,
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
    walk_attachments(
        raw,
        content_type,
        "",
        transfer_encoding,
        "",
        &mut |info, (body, encoding)| {
            if info.id == attachment_id && hit.is_none() {
                hit = Some((info, decode_transfer(body, encoding)));
            }
        },
    );
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
    let id = if prefix.is_empty() {
        "1".to_owned()
    } else {
        prefix.to_owned()
    };

    // Disposition parameters carry the filename first; `name=` on the
    // Content-Type is the legacy spelling of the same fact.
    let (disposition, disposition_params) = parse_content_type(content_disposition);
    let mut params = disposition_params;
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

/// Raw part content to bytes, undoing the transfer encoding. `7bit`, `8bit`,
/// `binary` and an absent encoding are already bytes.
fn decode_transfer(raw: &str, transfer_encoding: &str) -> Vec<u8> {
    match transfer_encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => {
            let cleaned: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            decode_base64(&cleaned).unwrap_or_default()
        }
        "quoted-printable" => decode_quoted_printable(raw),
        _ => raw.as_bytes().to_vec(),
    }
}

/// Quoted-printable as bodies use it: `=XX` hex escapes and `=`-at-end-of-line
/// soft breaks that join the wrapped line back together. Unlike the header
/// form, `_` is a literal underscore here.
fn decode_quoted_printable(text: &str) -> Vec<u8> {
    let chars: Vec<char> = text.chars().collect();
    let mut bytes = Vec::with_capacity(text.len());
    let mut index = 0;

    while index < chars.len() {
        if chars[index] != '=' {
            let mut buffer = [0u8; 4];
            bytes.extend_from_slice(chars[index].encode_utf8(&mut buffer).as_bytes());
            index += 1;
            continue;
        }

        match chars.get(index + 1) {
            // Soft line break: the `=` and its line ending vanish.
            Some('\n') => index += 2,
            Some('\r') => index += if chars.get(index + 2) == Some(&'\n') { 3 } else { 2 },
            Some(high) => match (
                high.to_digit(16),
                chars.get(index + 2).and_then(|c| c.to_digit(16)),
            ) {
                (Some(high), Some(low)) => {
                    bytes.push((high * 16 + low) as u8);
                    index += 3;
                }
                // Not a valid escape — keep the `=` as written.
                _ => {
                    bytes.push(b'=');
                    index += 1;
                }
            },
            None => {
                bytes.push(b'=');
                index += 1;
            }
        }
    }
    bytes
}

/// HTML to something an assistant can read: script and style blocks dropped,
/// block-level tags turned into line breaks, every other tag removed,
/// entities resolved. A best effort, not a renderer — but the words survive
/// and the markup does not.
fn html_to_text(html: &str) -> String {
    let cleaned = strip_noise(html);
    let mut out = String::with_capacity(cleaned.len());
    let mut chars = cleaned.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '<' {
            out.push(character);
            continue;
        }
        let mut tag = String::new();
        for inside in chars.by_ref() {
            if inside == '>' {
                break;
            }
            tag.push(inside);
        }
        if breaks_line(&tag) {
            out.push('\n');
        } else if separates_cell(&tag) {
            out.push(' ');
        }
    }

    let decoded = decode_entities(&out);
    decoded
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drop the parts of an HTML document that carry no readable text: script and
/// style blocks, and comments (which often hide whole conditional layouts).
fn strip_noise(html: &str) -> String {
    let without_scripts = remove_ranges(html, "<script", "</script>");
    let without_styles = remove_ranges(&without_scripts, "<style", "</style>");
    remove_ranges(&without_styles, "<!--", "-->")
}

/// Remove every `open … close` span, case-insensitively. An unclosed `open`
/// takes the rest of the input with it — a truncated script is still noise.
fn remove_ranges(input: &str, open: &str, close: &str) -> String {
    // Lowercasing only rewrites ASCII A–Z, so byte lengths are unchanged and
    // indices found here are valid slice points in the original.
    let haystack = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while cursor < input.len() {
        let Some(relative) = haystack[cursor..].find(open) else {
            out.push_str(&input[cursor..]);
            break;
        };
        let start = cursor + relative;
        out.push_str(&input[cursor..start]);
        match haystack[start + open.len()..].find(close) {
            Some(end) => cursor = start + open.len() + end + close.len(),
            None => break,
        }
    }
    out
}

/// Tags that end a line of text: paragraphs, list items, table rows, headings
/// and the like. `<br>` and the closing forms count too.
fn breaks_line(tag: &str) -> bool {
    let name = tag_name(tag);
    matches!(
        name.as_str(),
        "br" | "p"
            | "div"
            | "tr"
            | "li"
            | "ul"
            | "ol"
            | "table"
            | "blockquote"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "footer"
            | "section"
            | "article"
    )
}

/// Table cells sit on one line but should not run their text together.
fn separates_cell(tag: &str) -> bool {
    matches!(tag_name(tag).as_str(), "td" | "th")
}

/// The bare element name: no leading slash, no attributes, lowercased.
fn tag_name(tag: &str) -> String {
    tag.trim()
        .trim_start_matches('/')
        .split([' ', '\t', '\r', '\n', '/'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The handful of HTML entities that actually show up in mail text, plus
/// numeric escapes. An unknown entity is left as written rather than dropped.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(end) = after[..after.len().min(12)].find(';') else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..end];
        match resolve_entity(entity) {
            Some(character) => out.push_str(&character),
            None => out.push_str(&after[..=end]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn resolve_entity(entity: &str) -> Option<String> {
    let named = match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        _ => None,
    };
    if let Some(character) = named {
        return Some(character.to_string());
    }

    let code = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        entity.strip_prefix('#')?.parse().ok()?
    };
    Some(char::from_u32(code)?.to_string())
}

/// Collapse runs of blank lines to a single one and trim the edges, so the
/// paragraph breaks survive but the whitespace desert HTML leaves behind does
/// not.
fn collapse_blank_lines(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if matches!(out.last(), Some(last) if !last.is_empty()) {
                out.push("");
            }
        } else {
            out.push(line);
        }
    }
    while out.last() == Some(&"") {
        out.pop();
    }
    out.join("\n")
}

/// A header value fit for the wire. ASCII passes through untouched; anything
/// else becomes one RFC 2047 base64 encoded-word, so a subject with an umlaut
/// or an emoji survives a header that only carries ASCII.
pub(crate) fn encode_rfc2047(value: &str) -> String {
    if value.is_ascii() {
        return value.to_owned();
    }
    format!("=?UTF-8?B?{}?=", encode_base64(value.as_bytes()))
}

/// Standard padded base64. Small enough to hand-roll rather than take a
/// dependency; the mirror of `decode_base64`.
pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut buffer = [0u8; 3];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let packed =
            (u32::from(buffer[0]) << 16) | (u32::from(buffer[1]) << 8) | u32::from(buffer[2]);

        for index in 0..4 {
            if index <= chunk.len() {
                let value = ((packed >> (18 - index * 6)) & 0x3F) as usize;
                encoded.push(char::from(ALPHABET[value]));
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

/// The charsets we can turn into text without a lookup table. UTF-8 covers
/// almost all mail; Latin-1 maps one byte to one code point exactly. Others
/// return `None` rather than a guess, and the caller falls back to a lossy
/// UTF-8 read.
pub(crate) fn decode_charset(charset: &str, bytes: &[u8]) -> Option<String> {
    if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("us-ascii") {
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    if charset.eq_ignore_ascii_case("iso-8859-1") || charset.eq_ignore_ascii_case("latin1") {
        return Some(bytes.iter().map(|&byte| char::from(byte)).collect());
    }
    None
}

/// Standard padded base64 to bytes. Hand-rolled because torromail-core carries
/// no dependencies; shared by MIME bodies and RFC 2047 header words.
pub(crate) fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for character in text.chars() {
        if character == '=' {
            break;
        }
        let value = base64_value(character)?;
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
        }
    }
    Some(bytes)
}

fn base64_value(character: char) -> Option<u8> {
    match character {
        'A'..='Z' => Some(character as u8 - b'A'),
        'a'..='z' => Some(character as u8 - b'a' + 26),
        '0'..='9' => Some(character as u8 - b'0' + 52),
        '+' => Some(62),
        '/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_printable_undoes_escapes_and_soft_breaks() {
        let raw = "f=C3=BCr dich =E2=80=93 jetzt=\r\n weiter";
        let bytes = decode_quoted_printable(raw);
        let text = String::from_utf8(bytes).expect("decoded bytes are valid UTF-8");
        assert_eq!(text, "für dich – jetzt weiter");
    }

    #[test]
    fn a_plain_leaf_keeps_its_text() {
        let body = body_to_text("Hallo Welt\r\n", "text/plain; charset=utf-8", "7bit");
        assert_eq!(body, "Hallo Welt");
    }

    #[test]
    fn a_base64_leaf_is_decoded() {
        // "Hallo" in UTF-8, base64.
        let body = body_to_text("SGFsbG8=", "text/plain; charset=utf-8", "base64");
        assert_eq!(body, "Hallo");
    }

    #[test]
    fn multipart_alternative_prefers_plain_text_over_html() {
        let raw = concat!(
            "--b\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n\r\n",
            "Nur der Text z=C3=A4hlt\r\n",
            "--b\r\n",
            "Content-Type: text/html; charset=utf-8\r\n\r\n",
            "<p>HTML sollte verlieren</p>\r\n",
            "--b--\r\n",
        );
        let body = body_to_text(raw, "multipart/alternative; boundary=b", "7bit");
        assert_eq!(body, "Nur der Text zählt");
    }

    #[test]
    fn html_becomes_readable_text() {
        let html = concat!(
            "<html><head><style>p{color:red}</style></head><body>",
            "<p>Hey Sven,</p><p>Deine Vorteile:</p>",
            "<ul><li>eins</li><li>zwei</li></ul>",
            "<script>track()</script>",
            "<p>gr&#252;&#223;e&nbsp;&amp; bis bald</p></body></html>",
        );
        let text = body_to_text(html, "text/html; charset=utf-8", "7bit");
        assert!(text.contains("Hey Sven,"));
        assert!(text.contains("eins"));
        assert!(text.contains("zwei"));
        // Numeric entities resolved (ü, ß), &amp; too.
        assert!(text.contains("grüße & bis bald"));
        // Script and style contributed nothing.
        assert!(!text.contains("track()"));
        assert!(!text.contains("color:red"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn plain_and_rfc2047_filenames_decode() {
        let plain = vec![("filename".to_owned(), "angebot.pdf".to_owned())];
        assert_eq!(filename_from_params(&plain).as_deref(), Some("angebot.pdf"));

        // "Größenliste.pdf" as one UTF-8 B word.
        let b_word = vec![(
            "filename".to_owned(),
            "=?UTF-8?B?R3LDtsOfZW5saXN0ZS5wZGY=?=".to_owned(),
        )];
        assert_eq!(
            filename_from_params(&b_word).as_deref(),
            Some("Größenliste.pdf")
        );

        // Q form: underscores are spaces, =XX escapes apply.
        let q_word = vec![(
            "filename".to_owned(),
            "=?utf-8?Q?M=C3=A4rz_Bericht.pdf?=".to_owned(),
        )];
        assert_eq!(
            filename_from_params(&q_word).as_deref(),
            Some("März Bericht.pdf")
        );
    }

    #[test]
    fn rfc2231_extended_filenames_decode_and_win() {
        // filename* beats filename; charset prefix and percent escapes resolve.
        let extended = vec![
            ("filename".to_owned(), "fallback.bin".to_owned()),
            ("filename*".to_owned(), "UTF-8''%E2%82%AC-rechnung.pdf".to_owned()),
        ];
        assert_eq!(
            filename_from_params(&extended).as_deref(),
            Some("€-rechnung.pdf")
        );

        // Continuations join in numeric order; only segment 0 carries the
        // charset.
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

    #[test]
    fn an_attachment_only_part_yields_nothing() {
        let raw = concat!(
            "--b\r\n",
            "Content-Type: application/pdf; name=x.pdf\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\n",
            "AAAA\r\n",
            "--b--\r\n",
        );
        assert_eq!(body_to_text(raw, "multipart/mixed; boundary=b", "7bit"), "");
    }

    /// A realistic multipart/mixed: an alternative body (plain+html), a PDF
    /// beside it, and an inline PNG nested one level deeper in a related
    /// wrapper, so ids cross a boundary ("3.1").
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
        let (info, bytes) =
            extract_attachment_bytes(raw, content_type, "7bit", "2").expect("part 2 exists");
        assert_eq!(info.filename, "Angebot März.pdf");
        // "JVBERi0xLjQKJcOkw7zDtsOf" → "%PDF-1.4\n%" + UTF-8 "äüöß".
        assert_eq!(bytes, b"%PDF-1.4\n%\xc3\xa4\xc3\xbc\xc3\xb6\xc3\x9f".to_vec());
        assert!(extract_attachment_bytes(raw, content_type, "7bit", "9").is_none());
    }

    #[test]
    fn a_single_part_attachment_message_is_part_one() {
        // The whole message *is* the file: no multipart, filename on top.
        let listed = list_attachments("AAAA", "application/pdf; name=direkt.pdf", "base64");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "1");
        assert_eq!(listed[0].filename, "direkt.pdf");

        let (_, bytes) =
            extract_attachment_bytes("AAAA", "application/pdf; name=direkt.pdf", "base64", "1")
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
}
