//! Reading web pages for research and checks. Every URL here comes from a model answer or from
//! a model reply, so it is untrusted: the reader refuses addresses inside the network, follows
//! few redirects, reads a limited body, and pins the connection to the address it checked.

use async_trait::async_trait;
use openjam_core::actions::Fetched;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BODY: usize = 1_000_000;
const MAX_REDIRECTS: usize = 3;

#[async_trait]
pub trait PageReader: Send + Sync {
    /// Fetch one URL. A failure is `status: None`; this never returns an error.
    async fn read(&self, url: &str) -> Fetched;
}

pub struct HttpPages {
    /// Allow addresses inside the network, for a site that runs on the same host.
    allow_private: bool,
}

impl HttpPages {
    pub fn new(allow_private: bool) -> Self {
        Self { allow_private }
    }

    pub fn from_env() -> Self {
        let allow = std::env::var("OPENJAM_ALLOW_PRIVATE_FETCH")
            .is_ok_and(|v| matches!(v.trim(), "1" | "true"));
        Self::new(allow)
    }

    async fn fetch(&self, url: &str) -> Result<(u16, String), String> {
        let mut url = url::Url::parse(url).map_err(|e| e.to_string())?;
        for _ in 0..=MAX_REDIRECTS {
            if !matches!(url.scheme(), "http" | "https") {
                return Err("only http and https".into());
            }
            let host = url.host_str().ok_or("no host")?.to_owned();
            let port = url.port_or_known_default().ok_or("no port")?;
            let addr = self.resolve(&host, port).await?;
            let client = reqwest::Client::builder()
                .timeout(TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("openJam-research/0.1")
                .resolve(&host, addr)
                .build()
                .map_err(|e| e.to_string())?;
            let mut response = client
                .get(url.clone())
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?;
            if response.status().is_redirection() {
                let next = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|l| url.join(l).ok())
                    .ok_or("redirect without a usable location")?;
                url = next;
                continue;
            }
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| e.without_url().to_string())?
            {
                body.extend_from_slice(&chunk);
                if body.len() >= MAX_BODY {
                    body.truncate(MAX_BODY);
                    break;
                }
            }
            return Ok((status, String::from_utf8_lossy(&body).into_owned()));
        }
        Err("too many redirects".into())
    }

    /// The first address of `host` that may be fetched.
    async fn resolve(&self, host: &str, port: u16) -> Result<SocketAddr, String> {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| e.to_string())?
            .collect();
        addrs
            .into_iter()
            .find(|a| self.allow_private || is_public(a.ip()))
            .ok_or_else(|| format!("{host} has no public address"))
    }
}

#[async_trait]
impl PageReader for HttpPages {
    async fn read(&self, url: &str) -> Fetched {
        match self.fetch(url).await {
            Ok((status, text)) => Fetched {
                url: url.to_owned(),
                status: Some(status),
                text,
            },
            Err(error) => {
                tracing::debug!(url, %error, "page not read");
                Fetched {
                    url: url.to_owned(),
                    status: None,
                    text: String::new(),
                }
            }
        }
    }
}

/// True when `ip` is an address on the public internet.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || o[0] == 0)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || first & 0xfe00 == 0xfc00
                    || first & 0xffc0 == 0xfe80)
            }
        },
    }
}

/// The readable text of an HTML page: no scripts, styles, comments or tags, entities decoded,
/// white space collapsed. Text that is not HTML passes through with its white space collapsed.
pub fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    while i < html.len() {
        let Some(open) = html[i..].find('<').map(|n| n + i) else {
            out.push_str(&html[i..]);
            break;
        };
        out.push_str(&html[i..open]);
        out.push(' ');
        let skip_to = if lower[open..].starts_with("<!--") {
            lower[open..].find("-->").map(|n| open + n + 3)
        } else {
            ["script", "style"]
                .iter()
                .find(|t| lower[open..].starts_with(&format!("<{t}")))
                .and_then(|t| lower[open..].find(&format!("</{t}")).map(|n| open + n))
                .and_then(|close| html[close..].find('>').map(|n| close + n + 1))
        };
        i = match skip_to {
            Some(end) => end,
            None => html[open..].find('>').map_or(html.len(), |n| open + n + 1),
        };
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_read() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.9",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip} is not public");
        }
        for ip in [
            "8.8.8.8",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip} is public");
        }
    }

    #[test]
    fn html_becomes_plain_text() {
        let html = "<html><head><title>Acme &amp; Co</title><style>p{color:red}</style>\
                    <script>var x = '<b>';</script></head><body><!-- hidden --><h1>Hello</h1>\n\n<p>a&nbsp;b &lt;3</p></body></html>";
        assert_eq!(html_to_text(html), "Acme & Co Hello a b <3");
        assert_eq!(html_to_text("plain   text\nhere"), "plain text here");
        assert_eq!(html_to_text("broken <b"), "broken");
    }
}
