//! Friends and blocks (ADR-0021). Friend requests go by username, without a
//! shared server. A block hides presence, stops DMs and friend requests both
//! ways; it is never revealed to the blocked user (they get a generic error).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use neoshard_proto::v1::{self, Intent, RelationshipKind as K, event::Kind};
use neoshard_server_common::repo;
use serde::Deserialize;
use sqlx::PgPool;

use crate::AppState;
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::Auth;
use crate::ratelimit::rates;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users/@me/relationships", get(list).post(send_request))
        .route(
            "/users/@me/relationships/{uid}",
            put(set_relationship).delete(remove),
        )
}

async fn list(State(st): State<AppState>, a: Auth) -> ApiResult<Json<Vec<v1::Relationship>>> {
    Ok(Json(repo::relationships(&st.ctx.db, a.id()).await?))
}

async fn set_kind(db: &PgPool, user: u64, peer: u64, kind: K) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO relationships (user_id, peer_id, kind) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, peer_id) DO UPDATE SET kind = EXCLUDED.kind, created_at = now()",
        user as i64,
        peer as i64,
        kind as i16
    )
    .execute(db)
    .await?;
    Ok(())
}

async fn delete_row(db: &PgPool, user: u64, peer: u64) -> sqlx::Result<bool> {
    Ok(sqlx::query!(
        "DELETE FROM relationships WHERE user_id = $1 AND peer_id = $2",
        user as i64,
        peer as i64
    )
    .execute(db)
    .await?
    .rows_affected()
        > 0)
}

/// Tell `to` about its relationship with `peer` (or that it is gone).
async fn notify(st: &AppState, to: u64, peer: u64) -> ApiResult<()> {
    let kind = repo::relationship_kind(&st.ctx.db, to, peer).await?;
    if kind == 0 {
        st.publish_user(
            to,
            Intent::None,
            Kind::RelationshipRemove(v1::RelationshipRemove { user_id: peer }),
        );
        return Ok(());
    }
    let user = repo::user(&st.ctx.db, peer)
        .await?
        .map(repo::UserRow::public);
    st.publish_user(
        to,
        Intent::None,
        Kind::RelationshipAdd(v1::Relationship {
            user,
            kind,
            since: repo::now_ms(),
        }),
    );
    Ok(())
}

#[derive(Deserialize)]
struct RequestBody {
    username: String,
}

/// Send a friend request by username (accepts it if they already asked you).
async fn send_request(
    State(st): State<AppState>,
    a: Auth,
    Json(b): Json<RequestBody>,
) -> ApiResult<(StatusCode, Json<v1::Relationship>)> {
    let me = a.id();
    let name = b.username.trim().trim_start_matches('@').to_lowercase();
    if name.is_empty() || name.len() > 32 {
        return Err(bad("username required"));
    }
    let peer = sqlx::query_scalar!(
        "SELECT id FROM users WHERE instance = '' AND username = $1 AND deleted_at IS NULL AND NOT bot",
        name
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .map(|x| x as u64)
    .ok_or(ApiError::NotFound)?;
    if peer == me {
        return Err(bad("you cannot add yourself"));
    }
    // A block (either way) looks like any other refusal to the sender.
    if repo::blocked_between(&st.ctx.db, me, peer).await? {
        return Err(bad("this user does not accept friend requests from you"));
    }
    let created = match K::try_from(repo::relationship_kind(&st.ctx.db, me, peer).await?)
        .unwrap_or(K::None)
    {
        K::Friend => return Err(ApiError::Conflict("already friends")),
        K::Outgoing => return Err(ApiError::Conflict("request already sent")),
        K::Incoming => {
            set_kind(&st.ctx.db, me, peer, K::Friend).await?;
            set_kind(&st.ctx.db, peer, me, K::Friend).await?;
            false
        }
        _ => {
            st.limiter
                .check(&format!("friend:{me}"), rates::FRIEND_REQUEST)?;
            set_kind(&st.ctx.db, me, peer, K::Outgoing).await?;
            set_kind(&st.ctx.db, peer, me, K::Incoming).await?;
            true
        }
    };
    notify(&st, me, peer).await?;
    notify(&st, peer, me).await?;
    let rel = mine(&st, me, peer).await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(rel),
    ))
}

async fn mine(st: &AppState, me: u64, peer: u64) -> ApiResult<v1::Relationship> {
    Ok(v1::Relationship {
        user: repo::user(&st.ctx.db, peer)
            .await?
            .map(repo::UserRow::public),
        kind: repo::relationship_kind(&st.ctx.db, me, peer).await?,
        since: repo::now_ms(),
    })
}

#[derive(Deserialize)]
struct SetBody {
    /// 1 = accept a friend request, 2 = block
    #[serde(rename = "type")]
    kind: i32,
}

async fn set_relationship(
    State(st): State<AppState>,
    a: Auth,
    Path(uid): Path<u64>,
    Json(b): Json<SetBody>,
) -> ApiResult<Json<v1::Relationship>> {
    let me = a.id();
    if uid == me {
        return Err(bad("not with yourself"));
    }
    repo::user(&st.ctx.db, uid)
        .await?
        .ok_or(ApiError::NotFound)?;
    match K::try_from(b.kind).unwrap_or(K::None) {
        K::Friend => {
            if repo::relationship_kind(&st.ctx.db, me, uid).await? != K::Incoming as i32 {
                return Err(bad("no friend request from this user"));
            }
            set_kind(&st.ctx.db, me, uid, K::Friend).await?;
            set_kind(&st.ctx.db, uid, me, K::Friend).await?;
        }
        K::Blocked => {
            set_kind(&st.ctx.db, me, uid, K::Blocked).await?;
            // Their side forgets friendship/requests; their own block of me stays.
            if repo::relationship_kind(&st.ctx.db, uid, me).await? != K::Blocked as i32 {
                delete_row(&st.ctx.db, uid, me).await?;
            }
        }
        _ => return Err(bad("type must be 1 (friend) or 2 (block)")),
    }
    notify(&st, me, uid).await?;
    notify(&st, uid, me).await?;
    Ok(Json(mine(&st, me, uid).await?))
}

/// Remove a friend, cancel or decline a request, or unblock.
async fn remove(
    State(st): State<AppState>,
    a: Auth,
    Path(uid): Path<u64>,
) -> ApiResult<StatusCode> {
    let me = a.id();
    let was = repo::relationship_kind(&st.ctx.db, me, uid).await?;
    if !delete_row(&st.ctx.db, me, uid).await? {
        return Err(ApiError::NotFound);
    }
    if was != K::Blocked as i32
        && repo::relationship_kind(&st.ctx.db, uid, me).await? != K::Blocked as i32
    {
        delete_row(&st.ctx.db, uid, me).await?;
    }
    notify(&st, me, uid).await?;
    notify(&st, uid, me).await?;
    Ok(StatusCode::NO_CONTENT)
}
