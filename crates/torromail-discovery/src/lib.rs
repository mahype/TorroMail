//! The network half of account discovery. `torromail-control` decides what to
//! believe and in which order; this crate is only how the questions get
//! asked: MX, TXT and SRV over DNS, autoconfig documents over HTTPS, and a
//! plain TCP connect to see whether a guessed host exists at all.
//!
//! Everything here is bounded in time and best-effort. A lookup that fails —
//! no resolver, no route, a server that talks nonsense — is an empty answer,
//! and the chain moves on.

pub mod dns;

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use torromail_control::providers::{self, DiscoveredConfig, Network};

/// How long the whole chain may take. Past this the wizard shows manual
/// fields; waiting longer would be worse than asking.
pub const BUDGET: Duration = Duration::from_secs(8);
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// An autoconfig document is a few kilobytes; anything much larger is not one.
const MAX_DOCUMENT: u64 = 256 * 1024;

/// The real network: the system's resolvers and the public internet.
pub struct SystemNetwork {
    resolvers: Vec<SocketAddr>,
    agent: ureq::Agent,
}

impl Default for SystemNetwork {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .user_agent("TorroMail")
            // A redirect to plain http would hand the address to anyone on
            // the path; an autoconfig host that needs one is not trusted.
            .https_only(true)
            .build()
            .into();
        Self { resolvers: dns::system_resolvers(), agent }
    }
}

impl Network for SystemNetwork {
    fn mail_exchangers(&self, domain: &str) -> Vec<String> {
        dns::mail_exchangers(&self.resolvers, domain)
    }

    fn text_records(&self, domain: &str) -> Vec<String> {
        dns::text_records(&self.resolvers, domain)
    }

    fn imap_service(&self, domain: &str) -> Option<(String, u16)> {
        dns::imap_service(&self.resolvers, domain)
    }

    fn fetch(&self, url: &str) -> Option<String> {
        let mut response = self.agent.get(url).call().ok()?;
        if response.status() != 200 {
            return None;
        }
        response.body_mut().with_config().limit(MAX_DOCUMENT).read_to_string().ok()
    }

    fn answers(&self, host: &str, port: u16) -> bool {
        let Ok(addresses) = (host, port).to_socket_addrs() else {
            return false;
        };
        addresses.take(2).any(|address| TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok())
    }
}

/// Runs the chain against the real network, under the budget.
#[must_use]
pub fn discover(email: &str) -> Option<DiscoveredConfig> {
    let deadline = Instant::now() + BUDGET;
    providers::discover(email, &SystemNetwork::default(), &|| Instant::now() >= deadline)
}

/// The newest published release of a GitHub repository, as its tag
/// (`v0.9.0`). Read-only: nothing is downloaded or installed from here — on
/// Linux updates arrive through a package or a checkout.
pub fn latest_release_tag(repository: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .user_agent("TorroMail")
        .https_only(true)
        .build()
        .into();
    let mut response = agent
        .get(&format!("https://api.github.com/repos/{repository}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| error.to_string())?;
    let body = response.body_mut().with_config().limit(MAX_DOCUMENT).read_to_string().map_err(|error| error.to_string())?;
    // `"tag_name": "v0.9.0"` — one field of a large document, so it is picked
    // out rather than the whole thing being modelled.
    let after = body.split_once("\"tag_name\"").ok_or("the answer names no release")?.1;
    let start = after.find('"').ok_or("the answer names no release")? + 1;
    let end = after[start..].find('"').ok_or("the answer names no release")? + start;
    Ok(after[start..end].to_owned())
}

/// Whether `candidate` (`v1.2.3` or `1.2.3`) is newer than `installed`.
/// Compared number by number; anything that is not a number counts as zero.
#[must_use]
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    let numbers = |version: &str| -> Vec<u64> {
        version.trim_start_matches(['v', 'V']).split(['.', '-']).take(3).map(|part| part.parse().unwrap_or(0)).collect()
    };
    numbers(candidate) > numbers(installed)
}
