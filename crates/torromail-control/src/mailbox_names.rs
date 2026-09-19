//! IMAP4rev1 puts mailbox names on the wire in modified UTF-7. A picker shows
//! readable text but keeps the wire name as its value, because that exact
//! name is what later IMAP commands — and the policy document — must carry.

/// The readable form of a wire name. Anything that does not decode is shown
/// as it came: a strange folder name is still better than a missing one.
#[must_use]
pub fn display_name(wire_name: &str) -> String {
    let mut output = String::new();
    let mut rest = wire_name;
    while let Some(ampersand) = rest.find('&') {
        output.push_str(&rest[..ampersand]);
        let shifted = &rest[ampersand + 1..];
        let Some(end) = shifted.find('-') else {
            output.push_str(&rest[ampersand..]);
            return output;
        };
        let encoded = &shifted[..end];
        if encoded.is_empty() {
            output.push('&');
        } else if let Some(decoded) = decode_shifted(encoded) {
            output.push_str(&decoded);
        } else {
            output.push('&');
            output.push_str(encoded);
            output.push('-');
        }
        rest = &shifted[end + 1..];
    }
    output.push_str(rest);
    output
}

/// Modified base64 (`,` for `/`, no padding) over UTF-16BE.
fn decode_shifted(encoded: &str) -> Option<String> {
    let mut bits: u32 = 0;
    let mut bit_count = 0;
    let mut bytes = Vec::new();
    for character in encoded.bytes() {
        let value = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a' + 26,
            b'0'..=b'9' => character - b'0' + 52,
            b'+' => 62,
            b',' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            bytes.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
    String::from_utf16(&units).ok()
}
