//! Link previews for the web version, which cannot read other sites itself:
//! `GET /preview?url=<address>` answers with the head of that page (title, description,
//! site name are in it). Off unless `PB_PREVIEWS` is set.
//!
//! A server that fetches what it is asked to could be turned against others, so: only
//! `http`/`https` on ports 80 and 443, only public addresses (checked where names are
//! resolved, so redirects are held to it too), only HTML, only up to the end of the head,
//! a few at a time and a limited number per minute.

use std::io::Read;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ureq::config::Config;
use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// The most of a page read looking for the end of its head.
const MAX_BYTES: u64 = 512 * 1024;
/// Previews fetched at once; more wait their turn.
const AT_ONCE: usize = 4;
/// Previews per minute for the whole server.
const PER_MINUTE: u32 = 60;

/// Fetches heads of pages for previews.
pub struct Previews {
    agent: ureq::Agent,
    at_once: tokio::sync::Semaphore,
    /// The start of the current minute and the previews in it.
    minute: Mutex<(Instant, u32)>,
}

impl Previews {
    pub fn new() -> Self {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(Duration::from_secs(8)))
            .max_redirects(5)
            .user_agent("PaintBoard link preview")
            .build();
        Self {
            agent: ureq::Agent::with_parts(config, DefaultConnector::default(), PublicOnly),
            at_once: tokio::sync::Semaphore::new(AT_ONCE),
            minute: Mutex::new((Instant::now(), 0)),
        }
    }

    /// The head of the page at `url`, or why not.
    pub async fn head_of(&self, url: &str) -> Result<String, String> {
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err("only web addresses".into());
        }
        {
            let mut minute = self.minute.lock().expect("minute lock");
            if minute.0.elapsed() >= Duration::from_secs(60) {
                *minute = (Instant::now(), 0);
            }
            if minute.1 >= PER_MINUTE {
                return Err("too many previews, try again in a minute".into());
            }
            minute.1 += 1;
        }
        let _turn = self.at_once.acquire().await.map_err(|e| e.to_string())?;
        let (agent, url) = (self.agent.clone(), url.to_owned());
        tokio::task::spawn_blocking(move || fetch_head(&agent, &url))
            .await
            .map_err(|e| e.to_string())?
    }
}

fn fetch_head(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let response = agent.get(url).call().map_err(|e| e.to_string())?;
    let html = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("html"));
    if !html {
        return Err("not a web page".into());
    }
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(head(&String::from_utf8_lossy(&bytes)).to_owned())
}

/// `html` up to the end of its head, or all of it read if the head does not end.
fn head(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    match lower.find("</head>") {
        Some(end) => &html[..end + "</head>".len()],
        None => html,
    }
}

/// Resolves names as usual, keeping public addresses on the web's ports only.
#[derive(Debug)]
struct PublicOnly;

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = DefaultResolver::default().resolve(uri, config, timeout)?;
        let mut kept = self.empty();
        for addr in all.iter() {
            if matches!(addr.port(), 80 | 443) && is_public(addr.ip()) {
                kept.push(*addr);
            }
        }
        if kept.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(kept)
    }
}

/// Whether `ip` is an address on the internet, not one of a private network, the
/// machine itself or a range set aside.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                // Shared address space (carrier NAT), and the IETF's own.
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                // Benchmarking, and the reserved rest.
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local, link-local, documentation.
                || (first & 0xFE00) == 0xFC00
                || (first & 0xFFC0) == 0xFE80
                || (first == 0x2001 && v6.segments()[1] == 0x0DB8))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_internet_addresses_are_fetched() {
        for public in ["93.184.216.34", "1.1.1.1", "2606:4700::1111"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
        for inner in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
        ] {
            assert!(!is_public(inner.parse().unwrap()), "{inner}");
        }
    }

    #[test]
    fn only_the_head_is_handed_on() {
        let page = "<html><HEAD><title>T</title></HEAD><body>secret</body></html>";
        assert_eq!(head(page), "<html><HEAD><title>T</title></HEAD>");
        assert_eq!(head("<title>no end"), "<title>no end");
    }

    #[test]
    fn inner_addresses_are_refused() {
        let previews = Previews::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for url in [
            "http://127.0.0.1/",
            "http://localhost:8080/",
            "file:///etc/passwd",
        ] {
            assert!(runtime.block_on(previews.head_of(url)).is_err(), "{url}");
        }
    }
}
