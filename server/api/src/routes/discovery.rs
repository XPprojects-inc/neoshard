//! Public servers (ADR-0022): owners can let a guild be found by name and
//! joined without an invite. Search covers only guilds that opted in.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use neoshard_proto::permissions::Permissions as P;
use neoshard_proto::v1;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::authz::{audit, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::Auth;
use crate::ratelimit::Rate;
use crate::routes::channels::add_member;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discovery/guilds", get(search))
        .route("/discovery/guilds/{gid}/join", post(join))
        .route("/guilds/{gid}/discovery", get(settings).put(set_settings))
}

const SEARCH: Rate = Rate::new(30, std::time::Duration::from_secs(60));

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

/// `%`, `_` and `\` are literal in the user's text.
fn like_pattern(q: &str) -> String {
    let mut p = String::with_capacity(q.len() + 2);
    p.push('%');
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

/// Public guilds whose name contains `q` (all of them, by size, when `q` is empty).
async fn search(
    State(st): State<AppState>,
    a: Auth,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<Value>>> {
    st.limiter.check(&format!("discovery:{}", a.id()), SEARCH)?;
    let q = q.q.trim();
    if q.chars().count() > 100 {
        return Err(bad("query is at most 100 characters"));
    }
    // Unicode lowercase on both sides: the default C collation would not fold Cyrillic.
    let pattern = like_pattern(&q.to_lowercase());
    let rows = sqlx::query!(
        r#"SELECT g.id, g.name, g.icon, g.discovery_description,
                (SELECT count(*) FROM members m WHERE m.guild_id = g.id) AS "members!",
                EXISTS (SELECT 1 FROM members m WHERE m.guild_id = g.id AND m.user_id = $2) AS "joined!"
         FROM guilds g
         WHERE g.discoverable AND ($1 = '%%' OR lower(g.name COLLATE pg_c_utf8) LIKE $1)
         ORDER BY 5 DESC, g.id
         LIMIT 25"#,
        pattern,
        a.id() as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                json!({
                    "id": r.id.to_string(),
                    "name": r.name,
                    "icon": r.icon,
                    "description": r.discovery_description,
                    "member_count": r.members,
                    "joined": r.joined,
                })
            })
            .collect(),
    ))
}

/// Join a public guild without an invite (bans still apply).
async fn join(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<v1::Guild>> {
    st.limiter.check(
        &format!("discovery-join:{}", a.id()),
        crate::ratelimit::rates::INVITE_ACCEPT,
    )?;
    let public = sqlx::query_scalar!("SELECT discoverable FROM guilds WHERE id = $1", gid as i64)
        .fetch_optional(&st.ctx.db)
        .await?;
    if public != Some(true) {
        // Private guilds look the same as missing ones.
        return Err(ApiError::NotFound);
    }
    Ok(Json(add_member(&st, gid, a.id()).await?))
}

async fn settings(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<Value>> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_GUILD)?;
    let r = sqlx::query!(
        "SELECT discoverable, discovery_description FROM guilds WHERE id = $1",
        gid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    Ok(Json(json!({
        "enabled": r.discoverable,
        "description": r.discovery_description,
    })))
}

#[derive(Deserialize)]
struct Settings {
    enabled: bool,
    #[serde(default)]
    description: String,
}

async fn set_settings(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(s): Json<Settings>,
) -> ApiResult<Json<Value>> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_GUILD)?;
    let description = s.description.trim();
    if description.chars().count() > 300 {
        return Err(bad("description is at most 300 characters"));
    }
    sqlx::query!(
        "UPDATE guilds SET discoverable = $2, discovery_description = $3, updated_at = now() WHERE id = $1",
        gid as i64,
        s.enabled,
        description
    )
    .execute(&st.ctx.db)
    .await?;
    audit(
        &st,
        gid,
        a.id(),
        "guild.discovery",
        gid,
        json!({ "enabled": s.enabled }),
        "",
    )
    .await?;
    Ok(Json(
        json!({ "enabled": s.enabled, "description": description }),
    ))
}

#[cfg(test)]
mod tests {
    use super::like_pattern;

    #[test]
    fn like_wildcards_are_literal() {
        assert_eq!(like_pattern(""), "%%");
        assert_eq!(like_pattern("Мастерская"), "%Мастерская%");
        assert_eq!(like_pattern("100%_x\\"), "%100\\%\\_x\\\\%");
    }
}
