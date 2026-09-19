//! The order of the discovery chain, against a network that answers from a
//! table. The order is the point: it encodes which source is believed over
//! which, and each test names the disagreement it settles.

use std::cell::RefCell;
use std::collections::HashMap;

use torromail_control::providers::{self, AuthPath, Network};
use torromail_control::OAuthIssuer;

#[derive(Default)]
struct Table {
    mx: HashMap<&'static str, Vec<&'static str>>,
    txt: HashMap<&'static str, Vec<&'static str>>,
    srv: HashMap<&'static str, (&'static str, u16)>,
    pages: HashMap<String, &'static str>,
    listening: Vec<&'static str>,
    asked: RefCell<Vec<String>>,
}

impl Network for Table {
    fn mail_exchangers(&self, domain: &str) -> Vec<String> {
        self.asked.borrow_mut().push(format!("mx {domain}"));
        self.mx.get(domain).map(|hosts| hosts.iter().map(|host| (*host).to_owned()).collect()).unwrap_or_default()
    }
    fn text_records(&self, domain: &str) -> Vec<String> {
        self.asked.borrow_mut().push(format!("txt {domain}"));
        self.txt.get(domain).map(|records| records.iter().map(|record| (*record).to_owned()).collect()).unwrap_or_default()
    }
    fn imap_service(&self, domain: &str) -> Option<(String, u16)> {
        self.asked.borrow_mut().push(format!("srv {domain}"));
        self.srv.get(domain).map(|(target, port)| ((*target).to_owned(), *port))
    }
    fn fetch(&self, url: &str) -> Option<String> {
        self.asked.borrow_mut().push(format!("get {url}"));
        self.pages.get(url).map(|body| (*body).to_owned())
    }
    fn answers(&self, host: &str, _: u16) -> bool {
        self.asked.borrow_mut().push(format!("probe {host}"));
        self.listening.contains(&host)
    }
}

const HOSTER_AUTOCONFIG: &str = r#"<clientConfig version="1.1"><emailProvider id="hoster"><displayName>Hoster</displayName>
<incomingServer type="imap"><hostname>imap.hoster.example</hostname><port>993</port><socketType>SSL</socketType></incomingServer>
<outgoingServer type="smtp"><hostname>smtp.hoster.example</hostname><port>465</port><socketType>SSL</socketType></outgoingServer>
</emailProvider></clientConfig>"#;

fn never() -> bool {
    false
}

#[test]
fn a_known_domain_asks_the_network_nothing() {
    let network = Table::default();
    let found = providers::discover("sven@posteo.de", &network, &never).expect("in the table");
    assert_eq!(found.imap_host, "posteo.de");
    assert!(network.asked.borrow().is_empty());
}

#[test]
fn the_mx_is_believed_over_a_stale_autoconfig() {
    // The shared hoster still serves its generated file; the mail moved to
    // Microsoft 365 long ago. A password field here could never work.
    let mut network = Table::default();
    network.mx.insert("firma.example", vec!["firma-example.mail.protection.outlook.com"]);
    network.pages.insert("https://autoconfig.firma.example/mail/config-v1.1.xml?emailaddress=a@firma.example".to_owned(), HOSTER_AUTOCONFIG);

    let found = providers::discover("a@firma.example", &network, &never).expect("found");
    assert_eq!(found.auth, AuthPath::OAuth(OAuthIssuer::Microsoft));
    assert_eq!((found.provider_label.as_str(), found.source.as_str()), ("Microsoft 365", "mx"));
    assert_eq!(*network.asked.borrow(), ["mx firma.example"], "and nothing else was even fetched");
}

#[test]
fn a_domains_own_autoconfig_beats_the_shared_table_and_spf() {
    let mut network = Table::default();
    network.mx.insert("verein.example", vec!["mx.hoster.example"]);
    network.txt.insert("verein.example", vec!["v=spf1 include:_spf.google.com ~all"]);
    network.pages.insert(
        "https://verein.example/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=a@verein.example".to_owned(),
        HOSTER_AUTOCONFIG,
    );
    let found = providers::discover("a@verein.example", &network, &never).expect("found");
    assert_eq!((found.imap_host.as_str(), found.source.as_str()), ("imap.hoster.example", "autoconfig"));
    assert_eq!(found.smtp_port, 465);
}

#[test]
fn spf_rescues_a_tenant_hidden_behind_a_filtering_gateway() {
    let mut network = Table::default();
    network.mx.insert("kanzlei.example", vec!["mx01.hornetsecurity.com"]);
    network.txt.insert("kanzlei.example", vec!["some-verification=abc", "v=spf1 include:spf.protection.outlook.com -all"]);
    let found = providers::discover("a@kanzlei.example", &network, &never).expect("found");
    assert_eq!((found.auth, found.source.as_str()), (AuthPath::OAuth(OAuthIssuer::Microsoft), "spf"));
}

#[test]
fn a_third_party_mx_leads_back_to_the_table() {
    let mut network = Table::default();
    network.mx.insert("familie.example", vec!["mxext1.mailbox.org", "mxext2.mailbox.org"]);
    let found = providers::discover("a@familie.example", &network, &never).expect("found");
    assert_eq!((found.imap_host.as_str(), found.provider_label.as_str()), ("imap.mailbox.org", "mailbox.org"));
}

#[test]
fn srv_then_a_host_that_actually_answers_and_otherwise_nothing() {
    let mut network = Table::default();
    network.srv.insert("uni.example", ("mailhost.uni.example", 993));
    let found = providers::discover("a@uni.example", &network, &never).expect("found");
    assert_eq!((found.imap_host.as_str(), found.smtp_host.as_str(), found.source.as_str()), ("mailhost.uni.example", "smtp.uni.example", "srv"));

    let mut network = Table::default();
    network.listening.push("mail.klein.example");
    let found = providers::discover("a@klein.example", &network, &never).expect("found");
    assert_eq!((found.imap_host.as_str(), found.source.as_str()), ("mail.klein.example", "probe"));

    let network = Table::default();
    assert_eq!(providers::discover("a@nichts.example", &network, &never), None, "an empty field beats a hopeful hostname");
    assert_eq!(network.asked.borrow().last().map(String::as_str), Some("probe mail.nichts.example"));
}

#[test]
fn an_expired_budget_stops_the_chain_where_it_stands() {
    let network = Table::default();
    let calls = RefCell::new(0);
    let after_two = || {
        *calls.borrow_mut() += 1;
        *calls.borrow() > 2
    };
    assert_eq!(providers::discover("a@langsam.example", &network, &after_two), None);
    let asked = network.asked.borrow();
    assert!(asked.iter().any(|step| step.starts_with("get ")), "it got as far as the first fetch");
    assert!(!asked.iter().any(|step| step.starts_with("txt ") || step.starts_with("probe ")), "and no further: {asked:?}");
}
