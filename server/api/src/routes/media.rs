//! Voice/video/screen-share tokens (LiveKit) and TURN credentials, MLS key
//! distribution and relay, Web Push subscriptions.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use neoshard_media_auth as media;
use neoshard_proto::v1::{self, ChannelType, Intent, event::Kind};
use neoshard_server_common::repo;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::authz::channel_access;
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::Auth;
use crate::ratelimit::{Rate, rates};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{cid}/voice", post(voice_token))
        .route("/turn", get(turn))
        .route("/mls/key-packages", post(upload_key_packages))
        .route("/users/{uid}/mls/key-package", post(claim_key_package))
        .route("/channels/{cid}/mls", post(relay_mls))
        .route("/push/subscriptions", post(push_subscribe))
        .route("/push/subscriptions/{id}", delete(push_unsubscribe))
}

fn now_secs() -> u64 {
    (repo::now_ms() / 1000) as u64
}

async fn voice_token(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    let ok_type = ca.channel.r#type == ChannelType::Voice as i32 || ca.is_dm();
    if !ok_type {
        return Err(bad("not a voice channel"));
    }
    if ca.is_dm()
        && ca.channel.recipient_ids.len() == 2
        && let Some(peer) = ca
            .channel
            .recipient_ids
            .iter()
            .copied()
            .find(|&u| u != a.id())
        && repo::blocked_between(&st.ctx.db, a.id(), peer).await?
    {
        return Err(ApiError::Forbidden("blocked"));
    }
    let grant = media::grants(cid, ca.perms).ok_or(ApiError::MissingPermissions(
        neoshard_proto::permissions::Permissions::CONNECT,
    ))?;
    let c = &st.ctx.cfg;
    let (Some(url), Some(key), Some(secret)) =
        (&c.livekit_url, &c.livekit_api_key, &c.livekit_api_secret)
    else {
        return Err(ApiError::Unavailable(
            "voice is not configured on this instance (LIVEKIT_*)",
        ));
    };
    let user = repo::user(&st.ctx.db, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    let display = if user.display_name.is_empty() {
        user.username.clone()
    } else {
        user.display_name.clone()
    };
    let token = media::livekit_token(
        &media::LiveKitKeys {
            api_key: key,
            api_secret: secret,
        },
        a.id(),
        &display,
        grant.clone(),
        now_secs(),
        6 * 3600,
    )
    .map_err(anyhow::Error::from)?;
    let turn = c
        .turn_secret
        .as_ref()
        .map(|s| media::turn_credentials(s, &c.turn_urls, a.id(), now_secs(), 24 * 3600));
    Ok(Json(
        json!({ "url": url, "token": token, "room": grant.room, "grants": grant, "turn": turn }),
    ))
}

async fn turn(State(st): State<AppState>, a: Auth) -> ApiResult<Json<media::TurnCredentials>> {
    let c = &st.ctx.cfg;
    let secret = c
        .turn_secret
        .as_ref()
        .ok_or(ApiError::Unavailable("TURN is not configured"))?;
    Ok(Json(media::turn_credentials(
        secret,
        &c.turn_urls,
        a.id(),
        now_secs(),
        24 * 3600,
    )))
}

// ------------------------------------------------------------------ MLS (ADR-0013)

#[derive(Deserialize)]
struct KeyPackages {
    device_id: String,
    /// base64 MLS KeyPackages
    key_packages: Vec<String>,
}

async fn upload_key_packages(
    State(st): State<AppState>,
    a: Auth,
    Json(b): Json<KeyPackages>,
) -> ApiResult<Json<serde_json::Value>> {
    if b.device_id.is_empty() || b.device_id.len() > 64 || b.key_packages.len() > 100 {
        return Err(bad("device_id 1-64 chars, ≤ 100 key packages"));
    }
    let available = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM mls_key_packages WHERE user_id = $1 AND consumed_at IS NULL"#,
        a.id() as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if available + b.key_packages.len() as i64 > 500 {
        return Err(bad("too many unused key packages (500)"));
    }
    for kp in &b.key_packages {
        let raw = B64
            .decode(kp)
            .map_err(|_| bad("key packages must be base64"))?;
        if raw.is_empty() || raw.len() > 16 * 1024 {
            return Err(bad("key package size 1..16384"));
        }
        sqlx::query!(
            "INSERT INTO mls_key_packages (id, user_id, device_id, key_package) VALUES ($1, $2, $3, $4)",
            st.ctx.id() as i64,
            a.id() as i64,
            b.device_id,
            raw
        )
        .execute(&st.ctx.db)
        .await?;
    }
    Ok(Json(
        json!({ "available": available + b.key_packages.len() as i64 }),
    ))
}

/// Claim (consume) one KeyPackage per device of a user, to add them to a group.
async fn claim_key_package(
    State(st): State<AppState>,
    a: Auth,
    Path(uid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    st.limiter.check(
        &format!("kp:{}", a.id()),
        Rate::new(60, std::time::Duration::from_secs(60)),
    )?;
    let rows = sqlx::query!(
        "UPDATE mls_key_packages SET consumed_at = now()
         WHERE consumed_at IS NULL AND id IN (
            SELECT id FROM (
                SELECT id, row_number() OVER (PARTITION BY device_id ORDER BY id) AS rn
                FROM mls_key_packages WHERE user_id = $1 AND consumed_at IS NULL) t
            WHERE rn = 1)
         RETURNING device_id, key_package",
        uid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({ "device_id": r.device_id, "key_package": B64.encode(r.key_package) }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
struct MlsRelay {
    /// "commit" | "welcome" | "proposal"
    kind: String,
    payload: String,
    /// For welcome messages: only these recipients.
    #[serde(default, deserialize_with = "crate::extract::ids")]
    to: Vec<u64>,
}

async fn relay_mls(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(b): Json<MlsRelay>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if !ca.channel.e2ee {
        return Err(bad("channel is not end-to-end encrypted"));
    }
    if !matches!(b.kind.as_str(), "commit" | "welcome" | "proposal") {
        return Err(bad("kind must be commit, welcome or proposal"));
    }
    let payload = B64
        .decode(&b.payload)
        .map_err(|_| bad("payload must be base64"))?;
    if payload.len() > 256 * 1024 {
        return Err(bad("payload too large"));
    }
    let ev = v1::MlsEvent {
        channel_id: cid,
        sender_id: a.id(),
        payload,
        kind: b.kind,
    };
    let targets: Vec<u64> = if b.to.is_empty() {
        ca.channel
            .recipient_ids
            .iter()
            .copied()
            .filter(|&u| u != a.id())
            .collect()
    } else {
        b.to.iter()
            .copied()
            .filter(|u| ca.channel.recipient_ids.contains(u))
            .collect()
    };
    for u in targets {
        st.publish_user(u, Intent::DirectMessages, Kind::Mls(ev.clone()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ Web Push

#[derive(Deserialize)]
struct PushKeys {
    p256dh: String,
    auth: String,
}

#[derive(Deserialize)]
struct PushSub {
    endpoint: String,
    keys: PushKeys,
}

async fn push_subscribe(
    State(st): State<AppState>,
    a: Auth,
    Json(b): Json<PushSub>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    st.limiter
        .check(&format!("push:{}", a.id()), rates::TYPING)?;
    if !b.endpoint.starts_with("https://") || b.endpoint.len() > 2048 {
        return Err(bad("endpoint must be an https URL"));
    }
    let id = st.ctx.id();
    sqlx::query!(
        "INSERT INTO push_subscriptions (id, user_id, endpoint, p256dh, auth) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (endpoint) DO UPDATE SET user_id = EXCLUDED.user_id, p256dh = EXCLUDED.p256dh, auth = EXCLUDED.auth",
        id as i64,
        a.id() as i64,
        b.endpoint,
        b.keys.p256dh,
        b.keys.auth
    )
    .execute(&st.ctx.db)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id.to_string() }))))
}

async fn push_unsubscribe(
    State(st): State<AppState>,
    a: Auth,
    Path(id): Path<u64>,
) -> ApiResult<StatusCode> {
    sqlx::query!(
        "DELETE FROM push_subscriptions WHERE id = $1 AND user_id = $2",
        id as i64,
        a.id() as i64
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
