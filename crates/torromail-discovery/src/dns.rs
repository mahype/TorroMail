//! A deliberately small DNS client: one question, three record types, the
//! system's own resolvers. It exists because discovery needs MX, TXT and SRV
//! — which the standard library cannot ask for — and nothing else about DNS.
//!
//! The answers come from the network, so the parser trusts nothing: every
//! read is bounds-checked, and compression pointers are followed a limited
//! number of times, because a packet may point a name at itself.

use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::io::{Read, Write};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);
const TYPE_TXT: u16 = 16;
const TYPE_MX: u16 = 15;
const TYPE_SRV: u16 = 33;
const TYPE_OPT: u16 = 41;
/// What we tell the server we can take over UDP (EDNS0). SPF records
/// routinely outgrow the classic 512 bytes.
const UDP_PAYLOAD: u16 = 1232;
const MAX_POINTER_JUMPS: usize = 32;

/// One answer record, reduced to what discovery reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Mx { preference: u16, host: String },
    Txt(String),
    Srv { priority: u16, port: u16, target: String },
}

/// The `nameserver` lines of `/etc/resolv.conf`.
#[must_use]
pub fn system_resolvers() -> Vec<SocketAddr> {
    std::fs::read_to_string("/etc/resolv.conf").map(|text| parse_resolv_conf(&text)).unwrap_or_default()
}

#[must_use]
pub fn parse_resolv_conf(text: &str) -> Vec<SocketAddr> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        // A scoped IPv6 address (`fe80::1%eth0`) does not parse; it is skipped
        // rather than guessed at.
        .filter_map(|rest| rest.trim().parse::<IpAddr>().ok())
        .map(|address| SocketAddr::new(address, 53))
        .collect()
}

/// The wire form of one question, or `None` for a name DNS cannot carry.
/// Non-ASCII names are refused rather than sent raw: they would need
/// punycode, and a wrong question is worse than none.
#[must_use]
pub fn build_query(id: u16, name: &str, record_type: u16) -> Option<Vec<u8>> {
    let name = name.trim_end_matches('.');
    if name.is_empty() || name.len() > 253 || !name.is_ascii() {
        return None;
    }
    let mut packet = Vec::with_capacity(name.len() + 32);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00]); // recursion desired
    packet.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1]); // one question, one additional (OPT)
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&record_type.to_be_bytes());
    packet.extend_from_slice(&[0, 1]); // class IN
    // OPT pseudo-record: root name, type, our UDP size as the "class".
    packet.push(0);
    packet.extend_from_slice(&TYPE_OPT.to_be_bytes());
    packet.extend_from_slice(&UDP_PAYLOAD.to_be_bytes());
    packet.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // extended rcode/flags, no options
    Some(packet)
}

struct Reader<'a> {
    packet: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        let slice = self.packet.get(self.position..self.position.checked_add(count)?)?;
        self.position += count;
        Some(slice)
    }

    fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// A possibly compressed name. The reader ends up after the name as it
    /// appears *here*, wherever its pointers led.
    fn name(&mut self) -> Option<String> {
        let mut labels: Vec<String> = Vec::new();
        let mut cursor = self.position;
        let mut resume: Option<usize> = None;
        let mut jumps = 0;
        loop {
            let length = usize::from(*self.packet.get(cursor)?);
            match length {
                0 => {
                    cursor += 1;
                    break;
                }
                length if length & 0xC0 == 0xC0 => {
                    let low = usize::from(*self.packet.get(cursor + 1)?);
                    resume.get_or_insert(cursor + 2);
                    cursor = ((length & 0x3F) << 8) | low;
                    jumps += 1;
                    if jumps > MAX_POINTER_JUMPS {
                        return None;
                    }
                }
                length if length & 0xC0 != 0 => return None,
                length => {
                    let label = self.packet.get(cursor + 1..cursor + 1 + length)?;
                    labels.push(String::from_utf8_lossy(label).to_ascii_lowercase());
                    cursor += 1 + length;
                    if labels.len() > 127 {
                        return None;
                    }
                }
            }
        }
        self.position = resume.unwrap_or(cursor);
        Some(labels.join("."))
    }
}

/// The answers of one response, or `None` when it is not a usable answer to
/// the question with this id. `Some(truncated = true)` asks for a retry over
/// TCP.
#[must_use]
pub fn parse_response(packet: &[u8], id: u16) -> Option<(Vec<Record>, bool)> {
    let mut reader = Reader { packet, position: 0 };
    if reader.u16()? != id {
        return None;
    }
    let flags = reader.u16()?;
    let is_response = flags & 0x8000 != 0;
    let truncated = flags & 0x0200 != 0;
    let rcode = flags & 0x000F;
    if !is_response || rcode != 0 {
        return None;
    }
    let questions = reader.u16()?;
    let answers = reader.u16()?;
    reader.bytes(4)?; // authority and additional counts
    for _ in 0..questions {
        reader.name()?;
        reader.bytes(4)?;
    }
    let mut records = Vec::new();
    for _ in 0..answers {
        reader.name()?;
        let record_type = reader.u16()?;
        reader.bytes(6)?; // class, ttl
        let length = usize::from(reader.u16()?);
        let end = reader.position.checked_add(length)?;
        if end > packet.len() {
            return None;
        }
        match record_type {
            TYPE_MX => {
                let preference = reader.u16()?;
                records.push(Record::Mx { preference, host: reader.name()? });
            }
            TYPE_SRV => {
                let priority = reader.u16()?;
                reader.u16()?; // weight
                let port = reader.u16()?;
                records.push(Record::Srv { priority, port, target: reader.name()? });
            }
            TYPE_TXT => {
                // One record is a run of length-prefixed strings that belong
                // together; SPF in particular is split this way.
                let mut text = Vec::new();
                while reader.position < end {
                    let piece = usize::from(*reader.bytes(1)?.first()?);
                    if reader.position + piece > end {
                        return None;
                    }
                    text.extend_from_slice(reader.bytes(piece)?);
                }
                records.push(Record::Txt(String::from_utf8_lossy(&text).into_owned()));
            }
            _ => {}
        }
        // Whatever the record said about its own length wins over where
        // parsing it happened to stop.
        reader.position = end;
    }
    Some((records, truncated))
}

fn query_id() -> u16 {
    let mut bytes = [0_u8; 2];
    // A predictable id only makes spoofing marginally easier on a path that
    // is already the local resolver; still, ask for a random one.
    let _ = getrandom::getrandom(&mut bytes);
    u16::from_be_bytes(bytes)
}

fn ask(resolver: SocketAddr, name: &str, record_type: u16) -> Option<Vec<Record>> {
    let id = query_id();
    let query = build_query(id, name, record_type)?;
    let socket = UdpSocket::bind(if resolver.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).ok()?;
    socket.set_read_timeout(Some(TIMEOUT)).ok()?;
    socket.connect(resolver).ok()?;
    socket.send(&query).ok()?;
    let mut buffer = [0_u8; 4096];
    let received = socket.recv(&mut buffer).ok()?;
    let (records, truncated) = parse_response(&buffer[..received], id)?;
    if !truncated {
        return Some(records);
    }

    // Too big for UDP: the same question over TCP, length-prefixed.
    let mut stream = TcpStream::connect_timeout(&resolver, TIMEOUT).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    stream.write_all(&(query.len() as u16).to_be_bytes()).ok()?;
    stream.write_all(&query).ok()?;
    let mut length = [0_u8; 2];
    stream.read_exact(&mut length).ok()?;
    let mut response = vec![0_u8; usize::from(u16::from_be_bytes(length))];
    stream.read_exact(&mut response).ok()?;
    parse_response(&response, id).map(|(records, _)| records)
}

/// The first resolver that answers decides; one that does not answer is
/// skipped. At most two are tried, so a dead resolver list costs seconds, not
/// the whole discovery budget.
fn lookup(resolvers: &[SocketAddr], name: &str, record_type: u16) -> Vec<Record> {
    resolvers.iter().take(2).find_map(|resolver| ask(*resolver, name, record_type)).unwrap_or_default()
}

/// MX hosts, most preferred first.
#[must_use]
pub fn mail_exchangers(resolvers: &[SocketAddr], domain: &str) -> Vec<String> {
    let mut hosts: Vec<(u16, String)> = lookup(resolvers, domain, TYPE_MX)
        .into_iter()
        .filter_map(|record| match record {
            // A lone "." is the null MX: the domain takes no mail.
            Record::Mx { preference, host } if !host.is_empty() => Some((preference, host)),
            _ => None,
        })
        .collect();
    hosts.sort();
    hosts.into_iter().map(|(_, host)| host).collect()
}

#[must_use]
pub fn text_records(resolvers: &[SocketAddr], domain: &str) -> Vec<String> {
    lookup(resolvers, domain, TYPE_TXT)
        .into_iter()
        .filter_map(|record| if let Record::Txt(text) = record { Some(text) } else { None })
        .collect()
}

/// RFC 6186 `_imaps._tcp.<domain>`: the most preferred target and its port.
#[must_use]
pub fn imap_service(resolvers: &[SocketAddr], domain: &str) -> Option<(String, u16)> {
    lookup(resolvers, &format!("_imaps._tcp.{domain}"), TYPE_SRV)
        .into_iter()
        .filter_map(|record| match record {
            Record::Srv { priority, port, target } if !target.is_empty() => Some((priority, target, port)),
            _ => None,
        })
        .min()
        .map(|(_, target, port)| (target, port))
}
