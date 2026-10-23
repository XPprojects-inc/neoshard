//! Federation endpoints (ADR-0014, spec/federation.md).
//!
//! Implemented slice: federated identities (`user@instance`) and direct
//! messages between instances. Remote users appear locally as "shadow"
//! accounts (`users.instance = <domain>`) and can be DMed like local users;
//! messages to them are delivered to their home instance's signed inbox.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use neoshard_federation::{self as fed, InstanceKey, SignedParts};
use neoshard_proto::v1;
use neoshard_server_common::repo;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::Auth;
use crate::routes::messages::{NewMessage, create_internal};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/federation/v1/users/{username}", get(profile))
        .route("/federation/v1/inbox", post(inbox))
}

/// Mounted under /api/v1: resolve a remote address into a local shadow user.
pub fn api_router() -> Router<AppState> {
    Router::new().route("/federation/resolve/{address}", post(resolve))
}

fn key(st: &AppState) -> ApiResult<InstanceKey> {
    if !st.ctx.cfg.federation_enabled {
        return Err(ApiError::NotFound);
    }
    let seed = st
        .ctx
        .cfg
        .federation_key
        .as_deref()
        .ok_or(ApiError::Unavailable("NEOSHARD_FEDERATION_KEY not set"))?;
    InstanceKey::from_seed_b64(seed)
        .map_err(|_| anyhow::anyhow!("invalid NEOSHARD_FEDERATION_KEY").into())
}

/// `host[:port]` with a DNS-name or IPv4 host; nothing that a URL parser could
/// reinterpret (`@`, `/`, `?`, `#`, backslashes, brackets).
pub fn valid_domain(d: &str) -> bool {
    let (host, port) = match d.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (d, None),
    };
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !l.starts_with('-')
        });
    let port_ok =
        port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    host_ok && port_ok
}

fn base_url(st: &AppState, domain: &str) -> String {
    // Dev/test instances run on plain http with a port.
    if st.ctx.cfg.dev && (domain.starts_with("localhost") || domain.starts_with("127.0.0.1")) {
        format!("http://{domain}")
    } else {
        format!("https://{domain}")
    }
}

/// GET a small JSON document from another instance: strict domain, the
/// resolved address is checked and pinned (no DNS rebinding), no redirects,
/// 64 KiB cap. Private addresses are only allowed in dev mode.
/// HTTP client pinned to the checked address of `domain`, without redirects.
async fn pinned_client(st: &AppState, domain: &str) -> ApiResult<reqwest::Client> {
    if !valid_domain(domain) {
        return Err(bad("invalid instance domain"));
    }
    let (host, port) = match domain.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| bad("invalid port"))?,
        ),
        None => (
            domain.to_string(),
            if base_url(st, domain).starts_with("https") {
                443
            } else {
                80
            },
        ),
    };
    let addr = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| bad("instance unreachable"))?
        .next()
        .ok_or(bad("instance unreachable"))?;
    if !st.ctx.cfg.dev && !crate::routes::previews::is_public_ip(addr.ip()) {
        return Err(bad("instance unreachable"));
    }
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve(&host, addr)
        .build()
        .map_err(anyhow::Error::from)?)
}

async fn fetch_json(st: &AppState, domain: &str, path: &str) -> ApiResult<serde_json::Value> {
    if !valid_domain(domain) {
        return Err(bad("invalid instance domain"));
    }
    let (host, port) = match domain.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| bad("invalid port"))?,
        ),
        None => (
            domain.to_string(),
            if base_url(st, domain).starts_with("https") {
                443
            } else {
                80
            },
        ),
    };
    let addr = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| bad("instance unreachable"))?
        .next()
        .ok_or(bad("instance unreachable"))?;
    if !st.ctx.cfg.dev && !crate::routes::previews::is_public_ip(addr.ip()) {
        return Err(bad("instance unreachable"));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve(&host, addr)
        .build()
        .map_err(anyhow::Error::from)?;
    let mut resp = client
        .get(format!("{}{path}", base_url(st, domain)))
        .send()
        .await
        .map_err(|_| bad("instance unreachable"))?;
    if !resp.status().is_success() {
        return Err(ApiError::NotFound);
    }
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > 64 * 1024 {
            return Err(bad("instance response too large"));
        }
    }
    serde_json::from_slice(&body).map_err(|_| bad("instance unreachable"))
}

/// Public key of a remote instance (pinned on first use).
async fn instance_key(st: &AppState, domain: &str) -> ApiResult<String> {
    if let Some(r) = sqlx::query!(
        "SELECT public_key, blocked FROM federation_instances WHERE domain = $1",
        domain
    )
    .fetch_optional(&st.ctx.db)
    .await?
    {
        if r.blocked {
            return Err(ApiError::Forbidden("instance is blocked"));
        }
        return Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            r.public_key,
        ));
    }
    st.limiter.check(
        "fed-keyfetch",
        crate::ratelimit::Rate::new(30, std::time::Duration::from_secs(60)),
    )?;
    let wk = fetch_json(st, domain, "/.well-known/neoshard").await?;
    let pk = wk["federation"]["public_key"]
        .as_str()
        .ok_or(bad("instance has no federation key"))?
        .to_string();
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &pk)
        .map_err(|_| bad("bad key"))?;
    if raw.len() != 32 {
        return Err(bad("bad key"));
    }
    sqlx::query!("INSERT INTO federation_instances (domain, public_key) VALUES ($1, $2) ON CONFLICT (domain) DO NOTHING", domain, raw)
        .execute(&st.ctx.db)
        .await?;
    Ok(pk)
}

async fn profile(
    State(st): State<AppState>,
    Path(username): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    key(&st)?;
    let u = sqlx::query!("SELECT id, username, display_name, avatar FROM users WHERE instance = '' AND username = $1 AND deleted_at IS NULL AND NOT bot", username)
        .fetch_optional(&st.ctx.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(
        json!({ "username": u.username, "display_name": u.display_name, "avatar": u.avatar, "instance": st.ctx.cfg.domain }),
    ))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum InboxEvent {
    #[serde(rename = "dm.message")]
    DmMessage {
        id: String,
        from: String,
        to: String,
        content: String,
    },
}

fn header<'a>(h: &'a HeaderMap, name: &'static str) -> Result<&'a str, ApiError> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::BadRequest(format!("missing {name}")))
}

async fn inbox(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    key(&st)?;
    let origin = header(&headers, "neoshard-origin")?.to_lowercase();
    if !valid_domain(&origin) {
        return Err(bad("invalid origin"));
    }
    let destination = header(&headers, "neoshard-destination")?;
    let date: u64 = header(&headers, "neoshard-date")?
        .parse()
        .map_err(|_| bad("bad date"))?;
    let nonce = header(&headers, "neoshard-nonce")?;
    let sig = header(&headers, "neoshard-signature")?;
    if destination != st.ctx.cfg.domain || origin == st.ctx.cfg.domain || nonce.len() > 64 {
        return Err(bad("wrong destination"));
    }
    let pk = instance_key(&st, &origin).await?;
    let parts = SignedParts {
        method: "POST",
        path: "/federation/v1/inbox",
        origin: &origin,
        destination,
        date,
        nonce,
        body: &body,
    };
    fed::verify(&pk, &parts, sig, (repo::now_ms() / 1000) as u64)
        .map_err(|_| ApiError::Unauthorized)?;
    let fresh = sqlx::query!(
        "INSERT INTO federation_nonces (origin, nonce) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        origin,
        nonce
    )
    .execute(&st.ctx.db)
    .await?
    .rows_affected();
    if fresh == 0 {
        return Err(ApiError::Conflict("replayed request"));
    }
    sqlx::query!(
        "UPDATE federation_instances SET last_seen = now() WHERE domain = $1",
        origin
    )
    .execute(&st.ctx.db)
    .await?;
    // Nonces only matter within the allowed clock skew.
    if rand::random::<u8>() == 0 {
        sqlx::query!("DELETE FROM federation_nonces WHERE seen_at < now() - interval '1 hour'")
            .execute(&st.ctx.db)
            .await?;
    }
    let ev: InboxEvent =
        serde_json::from_slice(&body).map_err(|e| bad(format!("bad event: {e}")))?;
    match ev {
        InboxEvent::DmMessage {
            from,
            to,
            content,
            id,
        } => {
            if content.chars().count() > crate::routes::messages::MAX_CONTENT
                || content.trim().is_empty()
            {
                return Err(bad("bad content"));
            }
            let local = sqlx::query_scalar!(
                "SELECT id FROM users WHERE instance = '' AND username = $1 AND deleted_at IS NULL",
                to
            )
            .fetch_optional(&st.ctx.db)
            .await?
            .ok_or(ApiError::NotFound)?;
            let shadow = shadow_user(&st, &from, &origin).await?;
            let ch = dm_channel(&st, local as u64, shadow).await?;
            let nm = NewMessage {
                content,
                nonce: Some(format!("fed:{}", id.chars().take(56).collect::<String>())),
                ..Default::default()
            };
            create_internal(&st, shadow, &ch, nm).await?;
        }
    }
    Ok(StatusCode::ACCEPTED)
}

async fn shadow_user(st: &AppState, username: &str, instance: &str) -> ApiResult<u64> {
    if fed::parse_address(&format!("{username}@{instance}")).is_none() {
        return Err(bad("invalid remote address"));
    }
    if let Some(id) = sqlx::query_scalar!(
        "SELECT id FROM users WHERE instance = $1 AND username = $2",
        instance,
        username
    )
    .fetch_optional(&st.ctx.db)
    .await?
    {
        return Ok(id as u64);
    }
    let id = st.ctx.id();
    sqlx::query!(
        "INSERT INTO users (id, username, display_name, instance) VALUES ($1, $2, $3, $4) ON CONFLICT (instance, username) DO NOTHING",
        id as i64,
        username,
        format!("{username}@{instance}"),
        instance
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(sqlx::query_scalar!(
        "SELECT id FROM users WHERE instance = $1 AND username = $2",
        instance,
        username
    )
    .fetch_one(&st.ctx.db)
    .await? as u64)
}

async fn dm_channel(st: &AppState, a: u64, b: u64) -> ApiResult<v1::Channel> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    if let Some(cid) = sqlx::query_scalar!(
        "SELECT channel_id FROM dm_pairs WHERE user_a = $1 AND user_b = $2 AND NOT e2ee",
        lo as i64,
        hi as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    {
        return repo::channel(&st.ctx.db, cid as u64)
            .await?
            .ok_or(ApiError::NotFound);
    }
    let cid = st.ctx.id();
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!("INSERT INTO channels (id, type) VALUES ($1, 7)", cid as i64)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "INSERT INTO channel_recipients (channel_id, user_id) VALUES ($1, $2), ($1, $3)",
        cid as i64,
        a as i64,
        b as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO dm_pairs (user_a, user_b, channel_id) VALUES ($1, $2, $3)",
        lo as i64,
        hi as i64,
        cid as i64
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(
        &ch,
        v1::Intent::DirectMessages,
        v1::event::Kind::ChannelCreate(ch.clone()),
    )
    .await;
    Ok(ch)
}

/// `POST /api/v1/federation/resolve/{user@instance}` → local shadow user (to open a DM with).
async fn resolve(
    State(st): State<AppState>,
    _a: Auth,
    Path(address): Path<String>,
) -> ApiResult<Json<v1::User>> {
    key(&st)?;
    let (user, domain) = fed::parse_address(&address).ok_or(bad("expected user@instance"))?;
    if domain == st.ctx.cfg.domain {
        return Err(bad("that is a local address"));
    }
    fetch_json(&st, domain, &format!("/federation/v1/users/{user}")).await?;
    instance_key(&st, domain).await?;
    let id = shadow_user(&st, user, domain).await?;
    Ok(Json(
        repo::user(&st.ctx.db, id)
            .await?
            .ok_or(ApiError::NotFound)?
            .public(),
    ))
}

/// Deliver a DM written by a local user to remote recipients' home instances.
pub fn deliver_dm(st: AppState, author: u64, channel: v1::Channel, msg: v1::Message) {
    if !st.ctx.cfg.federation_enabled || channel.guild_id != 0 {
        return;
    }
    tokio::spawn(async move {
        let Ok(k) = key(&st) else { return };
        let Ok(Some(me)) = repo::user(&st.ctx.db, author).await else {
            return;
        };
        if !me.instance.is_empty() {
            return; // never re-federate messages from shadow users
        }
        for uid in channel.recipient_ids.iter().filter(|&&u| u != author) {
            let Ok(Some(r)) = repo::user(&st.ctx.db, *uid).await else {
                continue;
            };
            if r.instance.is_empty() {
                continue;
            }
            let ev = InboxEvent::DmMessage {
                id: msg.id.to_string(),
                from: me.username.clone(),
                to: r.username.clone(),
                content: msg.content.clone(),
            };
            let Ok(body) = serde_json::to_vec(&ev) else {
                continue;
            };
            let date = (repo::now_ms() / 1000) as u64;
            let nonce = fed::new_nonce();
            let parts = SignedParts {
                method: "POST",
                path: "/federation/v1/inbox",
                origin: &st.ctx.cfg.domain,
                destination: &r.instance,
                date,
                nonce: &nonce,
                body: &body,
            };
            let sig = k.sign(&parts);
            let Ok(client) = pinned_client(&st, &r.instance).await else {
                continue;
            };
            let res = client
                .post(format!(
                    "{}/federation/v1/inbox",
                    base_url(&st, &r.instance)
                ))
                .header("content-type", "application/json")
                .header("neoshard-origin", &st.ctx.cfg.domain)
                .header("neoshard-destination", &r.instance)
                .header("neoshard-date", date.to_string())
                .header("neoshard-nonce", &nonce)
                .header("neoshard-signature", sig)
                .body(body)
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => tracing::warn!(status = %r.status(), "federated delivery rejected"),
                Err(e) => tracing::warn!(error = %e, "federated delivery failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::valid_domain;

    #[test]
    fn domains() {
        for ok in [
            "neoshard.ru",
            "a-b.example.org",
            "127.0.0.1:8080",
            "localhost:3000",
        ] {
            assert!(valid_domain(ok), "{ok}");
        }
        for bad in [
            "evil.com:80@169.254.169.254",
            "a/b",
            "x.ru?y",
            "x.ru#y",
            "[::1]:80",
            "-a.ru",
            "a..ru",
            "",
            "x.ru:",
            "x.ru:123456",
            "a\\b",
        ] {
            assert!(!valid_domain(bad), "{bad}");
        }
    }
}
