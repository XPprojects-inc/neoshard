//! Link previews (OpenGraph), fetched server-side after a message is sent.
//!
//! SSRF protection: only http(s); the host is resolved once, every address is
//! checked against private/loopback/link-local/multicast ranges, and the
//! request is pinned to the checked address (no DNS rebinding). Redirects are
//! followed manually with the same checks. Body is capped at 512 KiB.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use neoshard_proto::v1::{self, Intent, event::Kind};
use neoshard_server_common::repo;
use url::Url;

use crate::AppState;

const MAX_BODY: usize = 512 * 1024;
const MAX_LINKS: usize = 3;

/// URLs in a message, skipping `<suppressed>` ones and code spans.
pub fn extract_urls(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    for tok in content.split_whitespace() {
        if tok.matches('`').count() % 2 == 1 {
            in_code = !in_code;
            continue;
        }
        if in_code || tok.starts_with('<') {
            continue;
        }
        let t = tok.trim_end_matches(['.', ',', ')', '!', '?', ';', ':']);
        if (t.starts_with("https://") || t.starts_with("http://"))
            && Url::parse(t).is_ok()
            && !out.iter().any(|u| u == t)
        {
            out.push(t.to_string());
            if out.len() == MAX_LINKS {
                break;
            }
        }
    }
    out
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xC0) == 64) // CGNAT 100.64/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] & 0xFE) == 18) // benchmarking 198.18/15
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            // IPv4-compatible (::a.b.c.d), NAT64 (64:ff9b::/96) and 6to4 (2002::/16) embed IPv4.
            if s[..6] == [0, 0, 0, 0, 0, 0]
                || (s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0])
            {
                let v4 = std::net::Ipv4Addr::new(
                    (s[6] >> 8) as u8,
                    s[6] as u8,
                    (s[7] >> 8) as u8,
                    s[7] as u8,
                );
                return is_public_ip(IpAddr::V4(v4));
            }
            if s[0] == 0x2002 {
                let v4 = std::net::Ipv4Addr::new(
                    (s[1] >> 8) as u8,
                    s[1] as u8,
                    (s[2] >> 8) as u8,
                    s[2] as u8,
                );
                return is_public_ip(IpAddr::V4(v4));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xFE00) == 0xFC00 // unique local
                || (s[0] & 0xFFC0) == 0xFE80 // link local
                || (s[0] == 0x2001 && s[1] == 0x0DB8)) // documentation
        }
    }
}

pub(crate) async fn safe_fetch(url: &str) -> Option<(String, String)> {
    let mut current = Url::parse(url).ok()?;
    for _ in 0..3 {
        if !matches!(current.scheme(), "http" | "https") {
            return None;
        }
        let host = current.host_str()?.to_string();
        let port = current.port_or_known_default()?;
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
            .await
            .ok()?
            .collect();
        let addr = *addrs.first()?;
        if addrs.iter().any(|a| !is_public_ip(a.ip())) {
            return None;
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve(&host, addr)
            .user_agent(concat!("NeoShardBot/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;
        let resp = client
            .get(current.clone())
            .header("accept", "text/html")
            .send()
            .await
            .ok()?;
        if resp.status().is_redirection() {
            let loc = resp.headers().get("location")?.to_str().ok()?;
            current = current.join(loc).ok()?;
            continue;
        }
        if !resp.status().is_success() {
            return None;
        }
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if !ctype.starts_with("text/html") {
            return None;
        }
        let mut body = Vec::new();
        let mut resp = resp;
        while let Ok(Some(chunk)) = resp.chunk().await {
            body.extend_from_slice(&chunk);
            if body.len() >= MAX_BODY {
                break;
            }
        }
        return Some((
            current.to_string(),
            String::from_utf8_lossy(&body).into_owned(),
        ));
    }
    None
}

pub(crate) fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let i = lower.find(&key)? + key.len();
    let rest = &tag[i..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = rest[1..].find(quote)?;
        Some(&rest[1..1 + end])
    } else {
        Some(
            rest.split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or(""),
        )
    }
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

/// Parse OpenGraph / `<title>` from HTML.
pub fn parse_og(url: &str, html: &str) -> Option<v1::Embed> {
    // Untrusted input: never slice at byte offsets that might split a char.
    let mut cut = html.len().min(MAX_BODY);
    while !html.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &html[..cut];
    let mut e = v1::Embed {
        url: url.to_string(),
        ..Default::default()
    };
    for raw in head
        .split('<')
        .filter(|t| t.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("meta ")))
    {
        let tag = raw.split('>').next().unwrap_or("");
        let key = attr(tag, "property")
            .or_else(|| attr(tag, "name"))
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(content) = attr(tag, "content") else {
            continue;
        };
        let v = unescape(content).chars().take(500).collect::<String>();
        match key.as_str() {
            "og:title" | "twitter:title" if e.title.is_empty() => e.title = v,
            "og:description" | "description" | "twitter:description"
                if e.description.is_empty() =>
            {
                e.description = v
            }
            "og:site_name" => e.site_name = v,
            "og:image" | "twitter:image"
                if e.image_url.is_empty()
                    && (v.starts_with("https://") || v.starts_with("http://")) =>
            {
                e.image_url = v
            }
            "theme-color" => {
                e.color =
                    u32::from_str_radix(v.trim_start_matches('#'), 16).unwrap_or(0) & 0xFF_FFFF
            }
            _ => {}
        }
    }
    if e.title.is_empty() {
        let lower = head.to_ascii_lowercase();
        // ASCII lowercasing keeps byte offsets identical, so indices map onto `head`.
        if let (Some(s), Some(t)) = (lower.find("<title"), lower.find("</title>"))
            && let Some(gt) = lower[s..].find('>')
            && let Some(inner) = head.get(s + gt + 1..t)
        {
            e.title = unescape(inner.trim()).chars().take(256).collect();
        }
    }
    (!e.title.is_empty() || !e.description.is_empty()).then_some(e)
}

pub fn spawn_for_message(st: AppState, channel: v1::Channel, msg: v1::Message) {
    let urls = extract_urls(&msg.content);
    if urls.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let mut embeds = Vec::new();
        for u in urls {
            if let Some((final_url, html)) = safe_fetch(&u).await
                && let Some(e) = parse_og(&final_url, &html)
            {
                embeds.push(e);
            }
        }
        if embeds.is_empty() {
            return;
        }
        let Ok(json) = serde_json::to_value(&embeds) else {
            return;
        };
        let ok = sqlx::query!(
            "UPDATE messages SET embeds = $3 WHERE channel_id = $1 AND id = $2",
            msg.channel_id as i64,
            msg.id as i64,
            json
        )
        .execute(&st.ctx.db)
        .await
        .is_ok_and(|r| r.rows_affected() > 0);
        if ok && let Ok(Some(m)) = repo::message(&st.ctx.db, msg.channel_id, msg.id, 0).await {
            st.publish_channel_event(&channel, Intent::GuildMessages, Kind::MessageUpdate(m))
                .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            extract_urls("see https://a.ru/x, and <https://b.ru> `https://c.ru`"),
            vec!["https://a.ru/x"]
        );
        assert!(extract_urls("ftp://x.ru javascript:alert(1)").is_empty());
        assert_eq!(extract_urls("https://a.ru https://a.ru").len(), 1);
    }

    #[test]
    fn ip_filter() {
        for bad in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_ip(bad.parse().unwrap()), "{bad} must be blocked");
        }
        for bad in ["64:ff9b::7f00:1", "2002:7f00:1::", "::127.0.0.1"] {
            assert!(!is_public_ip(bad.parse().unwrap()), "{bad} must be blocked");
        }
        for good in ["77.88.8.8", "2a02:6b8::feed:0ff"] {
            assert!(
                is_public_ip(good.parse().unwrap()),
                "{good} must be allowed"
            );
        }
    }

    #[test]
    fn og() {
        let html = r##"<html><head><title>Fallback</title>
            <meta property="og:title" content="NeoShard &amp; друзья">
            <meta name="description" content='Открытый мессенджер'>
            <meta property="og:image" content="https://neoshard.ru/i.png">
            <meta name="theme-color" content="#5865f2"></head></html>"##;
        let e = parse_og("https://neoshard.ru", html).unwrap();
        assert_eq!(e.title, "NeoShard & друзья");
        assert_eq!(e.description, "Открытый мессенджер");
        assert_eq!(e.image_url, "https://neoshard.ru/i.png");
        assert_eq!(e.color, 0x5865f2);
        let e2 = parse_og("https://x.ru", "<title>Only title</title>").unwrap();
        assert_eq!(e2.title, "Only title");
        assert!(parse_og("https://x.ru", "<p>nothing</p>").is_none());
        // Multi-byte characters right after '<' must not panic.
        assert!(parse_og("https://x.ru", "<мета><title>ок</title>").is_some());
        assert!(parse_og("https://x.ru", "<я").is_none());
    }
}
