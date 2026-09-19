//! The DNS parser against packets built by hand — including ones no honest
//! server would send. The answers come off the network; the parser may be
//! wrong about them, but it may never read out of bounds or spin.

use torromail_discovery::dns::{self, Record};

const ID: u16 = 0xBEEF;

fn name(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for label in text.split('.').filter(|label| !label.is_empty()) {
        bytes.push(label.len() as u8);
        bytes.extend_from_slice(label.as_bytes());
    }
    bytes.push(0);
    bytes
}

/// A response to one question with the given answer records (type, rdata).
/// Every answer's owner name is a pointer to the question at offset 12.
fn response(flags: u16, question: &str, answers: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&ID.to_be_bytes());
    packet.extend_from_slice(&flags.to_be_bytes());
    packet.extend_from_slice(&[0, 1]);
    packet.extend_from_slice(&(answers.len() as u16).to_be_bytes());
    packet.extend_from_slice(&[0, 0, 0, 0]);
    packet.extend_from_slice(&name(question));
    packet.extend_from_slice(&[0, 15, 0, 1]);
    for (record_type, rdata) in answers {
        packet.extend_from_slice(&[0xC0, 12]);
        packet.extend_from_slice(&record_type.to_be_bytes());
        packet.extend_from_slice(&[0, 1, 0, 0, 1, 44]);
        packet.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        packet.extend_from_slice(rdata);
    }
    packet
}

fn mx(preference: u16, host: Vec<u8>) -> (u16, Vec<u8>) {
    let mut rdata = preference.to_be_bytes().to_vec();
    rdata.extend(host);
    (15, rdata)
}

#[test]
fn a_query_carries_the_name_the_type_and_an_edns_record() {
    let query = dns::build_query(ID, "Torro.dev.", 15).expect("a valid name");
    assert_eq!(&query[..2], &ID.to_be_bytes());
    assert_eq!(&query[2..4], &[0x01, 0x00], "recursion desired, nothing else");
    assert_eq!(&query[12..23], b"\x05Torro\x03dev\x00");
    assert_eq!(&query[23..27], &[0, 15, 0, 1]);
    assert_eq!(&query[27..32], &[0, 0, 41, 0x04, 0xD0], "OPT with a 1232-byte payload");

    assert!(dns::build_query(ID, "", 15).is_none());
    assert!(dns::build_query(ID, "a..b", 15).is_none());
    assert!(dns::build_query(ID, &"a".repeat(64), 15).is_none(), "a label over 63 bytes");
    assert!(dns::build_query(ID, "müller.example", 15).is_none(), "no raw non-ASCII on the wire");
}

#[test]
fn mx_answers_are_read_with_compressed_names() {
    // The second host is "mx2" + a pointer back to the question's "firma.example".
    let mut compressed = vec![3];
    compressed.extend_from_slice(b"mx2");
    compressed.extend_from_slice(&[0xC0, 12]);
    let packet = response(0x8180, "firma.example", &[mx(20, compressed), mx(10, name("ASPMX.L.Google.com"))]);

    let (records, truncated) = dns::parse_response(&packet, ID).expect("a usable answer");
    assert!(!truncated);
    assert_eq!(
        records,
        [
            Record::Mx { preference: 20, host: "mx2.firma.example".to_owned() },
            Record::Mx { preference: 10, host: "aspmx.l.google.com".to_owned() },
        ]
    );
}

#[test]
fn a_txt_record_split_into_pieces_is_one_string() {
    let mut rdata = vec![7];
    rdata.extend_from_slice(b"v=spf1 ");
    rdata.push(31);
    rdata.extend_from_slice(b"include:_spf.google.com ~all!!!");
    let packet = response(0x8180, "firma.example", &[(16, rdata)]);
    let (records, _) = dns::parse_response(&packet, ID).expect("a usable answer");
    assert_eq!(records, [Record::Txt("v=spf1 include:_spf.google.com ~all!!!".to_owned())]);
}

#[test]
fn srv_answers_carry_port_and_target_and_unknown_types_are_skipped() {
    let mut srv = vec![0, 5, 0, 0, 0x03, 0xE1];
    srv.extend(name("imap.uni.example"));
    let packet = response(0x8180, "_imaps._tcp.uni.example", &[(1, vec![192, 0, 2, 1]), (33, srv)]);
    let (records, _) = dns::parse_response(&packet, ID).expect("a usable answer");
    assert_eq!(records, [Record::Srv { priority: 5, port: 993, target: "imap.uni.example".to_owned() }]);
}

#[test]
fn what_is_not_an_answer_to_our_question_is_refused() {
    let good = response(0x8180, "firma.example", &[mx(10, name("mx.firma.example"))]);
    assert!(dns::parse_response(&good, ID + 1).is_none(), "someone else's id");
    assert!(dns::parse_response(&response(0x0100, "firma.example", &[]), ID).is_none(), "a query, not a response");
    assert!(dns::parse_response(&response(0x8183, "firma.example", &[]), ID).is_none(), "NXDOMAIN");
    let (_, truncated) = dns::parse_response(&response(0x8380, "firma.example", &[]), ID).expect("usable");
    assert!(truncated, "the TC bit asks for TCP");
}

#[test]
fn hostile_packets_end_in_none_never_in_a_panic_or_a_loop() {
    // A name that points at itself.
    let looping = response(0x8180, "firma.example", &[mx(10, vec![0xC0, 12 + 15 + 4 + 12])]);
    let self_pointer_at = looping.len() - 2;
    let mut looping = looping;
    looping[self_pointer_at] = 0xC0;
    looping[self_pointer_at + 1] = self_pointer_at as u8;
    assert!(dns::parse_response(&looping, ID).is_none());

    // A record that claims to be longer than the packet.
    let mut overlong = response(0x8180, "firma.example", &[mx(10, name("mx.firma.example"))]);
    let length_at = 12 + 15 + 4 + 10;
    overlong[length_at] = 0xFF;
    assert!(dns::parse_response(&overlong, ID).is_none());

    // A TXT piece that runs past its record.
    assert!(dns::parse_response(&response(0x8180, "firma.example", &[(16, vec![200, b'x'])]), ID).is_none());

    // Every prefix of a good packet, and a few bytes of noise.
    let good = response(0x8180, "firma.example", &[mx(10, name("mx.firma.example")), (16, vec![1, b'x'])]);
    for cut in 0..good.len() {
        let _ = dns::parse_response(&good[..cut], ID);
    }
    for noise in [&[][..], &[0xBE][..], &[0xBE, 0xEF, 0x81, 0x80, 0xFF, 0xFF, 0xFF, 0xFF][..]] {
        assert!(dns::parse_response(noise, ID).is_none_or(|(records, _)| records.is_empty()));
    }
}

#[test]
fn resolv_conf_yields_the_nameservers_and_skips_what_it_cannot_use() {
    let resolvers = dns::parse_resolv_conf(
        "# generated\nsearch fritz.box\nnameserver 192.168.178.1\nnameserver 2001:db8::1\nnameserver fe80::1%eth0\noptions edns0\n",
    );
    assert_eq!(
        resolvers.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["192.168.178.1:53", "[2001:db8::1]:53"]
    );
    assert!(dns::parse_resolv_conf("").is_empty());
}

/// `cargo test -p torromail-discovery -- --ignored`
#[test]
#[ignore = "asks the real DNS and the real internet"]
fn the_real_network_finds_a_workspace_domain_and_a_published_autoconfig() {
    let resolvers = dns::system_resolvers();
    assert!(!resolvers.is_empty(), "this machine has a resolver");
    let exchangers = dns::mail_exchangers(&resolvers, "google.com");
    assert!(exchangers.iter().any(|host| host.ends_with("google.com")), "got: {exchangers:?}");
    assert!(dns::text_records(&resolvers, "google.com").iter().any(|record| record.starts_with("v=spf1")));

    let found = torromail_discovery::discover("someone@google.com").expect("found through its MX");
    assert_eq!(found.source, "mx");
}
