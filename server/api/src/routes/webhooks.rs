//! Incoming webhooks: `POST /webhooks/{id}/{token}` posts into a channel.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use neoshard_proto::permissions::Permissions as P;
use neoshard_proto::v1;
use neoshard_server_common::{auth, repo};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::authz::{audit, channel_access, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::Auth;
use crate::ratelimit::rates;
use crate::routes::messages::{MAX_CONTENT, NewMessage, create_internal};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{cid}/webhooks", get(list_channel).post(create))
        .route("/guilds/{gid}/webhooks", get(list_guild))
        .route("/webhooks/{wid}", delete(remove))
        .route("/webhooks/{wid}/{token}", post(execute))
}

pub async fn create_webhook_row(
    st: &AppState,
    ch: &v1::Channel,
    creator: u64,
    name: &str,
) -> ApiResult<(u64, String)> {
    let (token, hash) = auth::new_token("nsw_");
    let id = st.ctx.id();
    sqlx::query!(
        "INSERT INTO webhooks (id, guild_id, channel_id, name, token_hash, created_by) VALUES ($1, $2, $3, $4, $5, $6)",
        id as i64,
        ch.guild_id as i64,
        ch.id as i64,
        name.chars().take(80).collect::<String>(),
        hash,
        creator as i64
    )
    .execute(&st.ctx.db)
    .await?;
    Ok((id, token))
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
}

async fn create(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(b): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if ca.is_dm() {
        return Err(bad("webhooks are for guild channels"));
    }
    ca.require(P::MANAGE_WEBHOOKS)?;
    if b.name.trim().is_empty() {
        return Err(bad("name required"));
    }
    let (id, token) = create_webhook_row(&st, &ca.channel, a.id(), b.name.trim()).await?;
    audit(
        &st,
        ca.guild_id(),
        a.id(),
        "webhook.create",
        id,
        json!({ "channel_id": cid.to_string() }),
        "",
    )
    .await?;
    let url = format!(
        "{}/api/v1/webhooks/{id}/{token}",
        st.ctx.cfg.public_url.trim_end_matches('/')
    );
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id.to_string(), "token": token, "url": url, "channel_id": cid.to_string() }),
        ),
    ))
}

async fn list_rows(st: &AppState, rows: Vec<(i64, i64, String)>) -> Json<serde_json::Value> {
    let _ = st;
    Json(json!(rows.into_iter().map(|(id, cid, name)| json!({ "id": id.to_string(), "channel_id": cid.to_string(), "name": name })).collect::<Vec<_>>()))
}

async fn list_channel(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    channel_access(&st, cid, a.id())
        .await?
        .require(P::MANAGE_WEBHOOKS)?;
    let rows = sqlx::query!(
        "SELECT id, channel_id, name FROM webhooks WHERE channel_id = $1",
        cid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    Ok(list_rows(
        &st,
        rows.into_iter()
            .map(|r| (r.id, r.channel_id, r.name))
            .collect(),
    )
    .await)
}

async fn list_guild(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_WEBHOOKS)?;
    let rows = sqlx::query!(
        "SELECT id, channel_id, name FROM webhooks WHERE guild_id = $1",
        gid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    Ok(list_rows(
        &st,
        rows.into_iter()
            .map(|r| (r.id, r.channel_id, r.name))
            .collect(),
    )
    .await)
}

async fn remove(
    State(st): State<AppState>,
    a: Auth,
    Path(wid): Path<u64>,
) -> ApiResult<StatusCode> {
    let w = sqlx::query!(
        "SELECT guild_id, channel_id FROM webhooks WHERE id = $1",
        wid as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    channel_access(&st, w.channel_id as u64, a.id())
        .await?
        .require(P::MANAGE_WEBHOOKS)?;
    sqlx::query!("DELETE FROM webhooks WHERE id = $1", wid as i64)
        .execute(&st.ctx.db)
        .await?;
    audit(
        &st,
        w.guild_id as u64,
        a.id(),
        "webhook.delete",
        wid,
        json!({}),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ExecuteBody {
    content: String,
    /// Optional per-message display name override.
    username: Option<String>,
}

async fn execute(
    State(st): State<AppState>,
    Path((wid, token)): Path<(u64, String)>,
    Json(b): Json<ExecuteBody>,
) -> ApiResult<(StatusCode, Json<v1::Message>)> {
    let w = sqlx::query!(
        "SELECT channel_id, token_hash, name FROM webhooks WHERE id = $1",
        wid as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !auth::ct_eq(&w.token_hash, &auth::hash_token(&token)) {
        return Err(ApiError::NotFound);
    }
    // Only holders of the token consume its quota.
    st.limiter.check(&format!("wh:{wid}"), rates::WEBHOOK)?;
    if b.content.trim().is_empty() || b.content.chars().count() > MAX_CONTENT {
        return Err(bad("content must be 1-4000 characters"));
    }
    let ch = repo::channel(&st.ctx.db, w.channel_id as u64)
        .await?
        .ok_or(ApiError::NotFound)?;
    let nm = NewMessage {
        content: b.content,
        webhook_id: Some(wid),
        ..Default::default()
    };
    let mut msg = create_internal(&st, wid, &ch, nm).await?;
    if let (Some(name), Some(author)) = (b.username, msg.author.as_mut()) {
        author.display_name = name.chars().take(80).collect();
    }
    let _ = w.name;
    Ok((StatusCode::CREATED, Json(msg)))
}
