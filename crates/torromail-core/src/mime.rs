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
fn encode_base64(bytes: &[u8]) -> String {
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
}
