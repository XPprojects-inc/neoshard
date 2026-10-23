//! Full user profiles (ADR-0020): bio, banner, theme, per-server profile,
//! mutual servers, private notes and verified links.
//!
//! A profile is visible to the user, to people sharing a server with them and
//! to their DM partners. Anyone else gets 404, the same as for an unknown id.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{post, put};
use axum::{Json, Router};
use neoshard_proto::profile;
use neoshard_proto::v1;
use neoshard_server_common::repo;
use serde::Deserialize;

use crate::AppState;
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::{self, Auth};
use crate::routes::previews::{attr, safe_fetch};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users/{uid}/profile", axum::routing::get(get_profile))
        .route("/users/{uid}/note", put(put_note))
        .route("/users/@me/links", put(put_links))
        .route("/users/@me/links/verify", post(verify_links))
}

#[derive(Deserialize)]
struct ProfileQuery {
    #[serde(default, deserialize_with = "extract::opt_id")]
    guild_id: Option<u64>,
}

fn resolve_uid(raw: &str, me: u64) -> ApiResult<u64> {
    if raw == "@me" {
        Ok(me)
    } else {
        raw.parse().map_err(|_| bad("invalid user id"))
    }
}

/// Self, a shared server or a shared DM.
async fn can_see(st: &AppState, viewer: u64, target: u64) -> ApiResult<bool> {
    if viewer == target {
        return Ok(true);
    }
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM members a JOIN members b ON a.guild_id = b.guild_id
                          WHERE a.user_id = $1 AND b.user_id = $2)
               OR EXISTS (SELECT 1 FROM channel_recipients a JOIN channel_recipients b ON a.channel_id = b.channel_id
                          WHERE a.user_id = $1 AND b.user_id = $2)
               OR EXISTS (SELECT 1 FROM relationships WHERE user_id = $1 AND peer_id = $2 AND kind IN (1, 3, 4)) AS "ok!""#,
        viewer as i64,
        target as i64
    )
    .fetch_one(&st.ctx.db)
    .await?)
}

fn colors(v: Vec<i32>) -> Vec<u32> {
    v.into_iter().map(|c| c as u32).collect()
}

async fn get_profile(
    State(st): State<AppState>,
    a: Auth,
    Path(uid): Path<String>,
    Query(q): Query<ProfileQuery>,
) -> ApiResult<Json<v1::UserProfile>> {
    let uid = resolve_uid(&uid, a.id())?;
    if !can_see(&st, a.id(), uid).await? {
        return Err(ApiError::NotFound);
    }
    let user = repo::user(&st.ctx.db, uid)
        .await?
        .ok_or(ApiError::NotFound)?
        .public();
    let p = sqlx::query!(
        "SELECT pronouns, bio, banner, banner_color, theme_colors, profile_effect, created_at FROM users WHERE id = $1",
        uid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    let links = sqlx::query!(
        "SELECT label, url, verified_at FROM user_links WHERE user_id = $1 ORDER BY position",
        uid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?
    .into_iter()
    .map(|l| v1::ProfileLink {
        label: l.label,
        url: l.url,
        verified: l.verified_at.is_some(),
    })
    .collect();
    let mutual_guild_ids = if uid == a.id() {
        vec![]
    } else {
        sqlx::query_scalar!(
            "SELECT a.guild_id FROM members a JOIN members b ON a.guild_id = b.guild_id WHERE a.user_id = $1 AND b.user_id = $2 ORDER BY a.guild_id",
            a.id() as i64,
            uid as i64
        )
        .fetch_all(&st.ctx.db)
        .await?
        .into_iter()
        .map(|g| g as u64)
        .collect()
    };
    // The per-server profile is shown only to members of that server.
    let member = match q.guild_id {
        Some(gid) if uid == a.id() || mutual_guild_ids.contains(&gid) => sqlx::query!(
            r#"SELECT m.nick, m.avatar, m.banner, m.bio, m.pronouns, m.theme_colors, m.joined_at,
                      COALESCE(array_agg(mr.role_id) FILTER (WHERE mr.role_id IS NOT NULL), '{}') AS "role_ids!"
               FROM members m LEFT JOIN member_roles mr ON mr.guild_id = m.guild_id AND mr.user_id = m.user_id
               WHERE m.guild_id = $1 AND m.user_id = $2
               GROUP BY m.guild_id, m.user_id"#,
            gid as i64,
            uid as i64
        )
        .fetch_optional(&st.ctx.db)
        .await?
        .map(|m| v1::MemberProfile {
            guild_id: gid,
            nick: m.nick,
            avatar: m.avatar,
            banner: m.banner,
            bio: m.bio,
            pronouns: m.pronouns,
            theme_colors: colors(m.theme_colors),
            role_ids: repo::ids(&m.role_ids),
            joined_at: m.joined_at.timestamp_millis(),
        }),
        _ => None,
    };
    let (mutual_friend_ids, relationship) = if uid == a.id() {
        (vec![], 0)
    } else {
        let mutual = sqlx::query_scalar!(
            "SELECT a.peer_id FROM relationships a JOIN relationships b ON a.peer_id = b.peer_id
             WHERE a.user_id = $1 AND b.user_id = $2 AND a.kind = 1 AND b.kind = 1 ORDER BY a.peer_id",
            a.id() as i64,
            uid as i64
        )
        .fetch_all(&st.ctx.db)
        .await?
        .into_iter()
        .map(|x| x as u64)
        .collect();
        (
            mutual,
            repo::relationship_kind(&st.ctx.db, a.id(), uid).await?,
        )
    };
    let note = sqlx::query_scalar!(
        "SELECT note FROM user_notes WHERE owner_id = $1 AND target_id = $2",
        a.id() as i64,
        uid as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .unwrap_or_default();
    Ok(Json(v1::UserProfile {
        user: Some(user),
        pronouns: p.pronouns,
        bio: p.bio,
        banner: p.banner,
        banner_color: p.banner_color.map(|c| c as u32),
        theme_colors: colors(p.theme_colors),
        profile_effect: p.profile_effect,
        created_at: p.created_at.timestamp_millis(),
        links,
        member,
        mutual_guild_ids,
        note,
        mutual_friend_ids,
        relationship,
    }))
}

#[derive(Deserialize)]
struct NoteBody {
    note: String,
}

/// Private note about another user; an empty note deletes it.
async fn put_note(
    State(st): State<AppState>,
    a: Auth,
    Path(uid): Path<String>,
    Json(b): Json<NoteBody>,
) -> ApiResult<StatusCode> {
    let uid = resolve_uid(&uid, a.id())?;
    let note = b.note.trim();
    if note.chars().count() > profile::NOTE_MAX {
        return Err(bad(format!("note ≤ {} characters", profile::NOTE_MAX)));
    }
    if !can_see(&st, a.id(), uid).await? {
        return Err(ApiError::NotFound);
    }
    if note.is_empty() {
        sqlx::query!(
            "DELETE FROM user_notes WHERE owner_id = $1 AND target_id = $2",
            a.id() as i64,
            uid as i64
        )
        .execute(&st.ctx.db)
        .await?;
    } else {
        sqlx::query!(
            "INSERT INTO user_notes (owner_id, target_id, note) VALUES ($1, $2, $3)
             ON CONFLICT (owner_id, target_id) DO UPDATE SET note = EXCLUDED.note, updated_at = now()",
            a.id() as i64,
            uid as i64,
            note
        )
        .execute(&st.ctx.db)
        .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct LinkIn {
    label: String,
    url: String,
}

/// Replace the profile links (≤ 6). Changed links lose their verification.
async fn put_links(
    State(st): State<AppState>,
    a: Auth,
    Json(links): Json<Vec<LinkIn>>,
) -> ApiResult<Json<Vec<v1::ProfileLink>>> {
    if links.len() > profile::LINKS_MAX {
        return Err(bad(format!("at most {} links", profile::LINKS_MAX)));
    }
    for l in &links {
        let label = l.label.trim();
        let parsed = url::Url::parse(l.url.trim()).ok();
        if label.is_empty()
            || label.chars().count() > 32
            || l.url.len() > 512
            || !parsed
                .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        {
            return Err(bad(
                "each link needs a label (≤ 32) and an http(s) URL (≤ 512)",
            ));
        }
    }
    let mut tx = st.ctx.db.begin().await?;
    let old = sqlx::query!(
        "SELECT url, verified_at FROM user_links WHERE user_id = $1",
        a.id() as i64
    )
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM user_links WHERE user_id = $1", a.id() as i64)
        .execute(&mut *tx)
        .await?;
    for (i, l) in links.iter().enumerate() {
        let url = l.url.trim();
        // Keep verification for a URL that did not change.
        let verified = old
            .iter()
            .find(|o| o.url == url)
            .and_then(|o| o.verified_at);
        sqlx::query!(
            "INSERT INTO user_links (user_id, position, label, url, verified_at) VALUES ($1, $2, $3, $4, $5)",
            a.id() as i64,
            i as i16,
            l.label.trim(),
            url,
            verified
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(load_links(&st, a.id()).await?))
}

async fn load_links(st: &AppState, uid: u64) -> ApiResult<Vec<v1::ProfileLink>> {
    Ok(sqlx::query!(
        "SELECT label, url, verified_at FROM user_links WHERE user_id = $1 ORDER BY position",
        uid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?
    .into_iter()
    .map(|l| v1::ProfileLink {
        label: l.label,
        url: l.url,
        verified: l.verified_at.is_some(),
    })
    .collect())
}

/// Public profile URL that a linked page must point back to.
pub fn profile_url(st: &AppState, username: &str) -> String {
    format!(
        "{}/u/{username}",
        st.ctx.cfg.public_url.trim_end_matches('/')
    )
}

/// `<a>` or `<link>` with `rel` containing `me` and `href` equal to `target`.
pub fn has_rel_me(html: &str, target: &str) -> bool {
    let norm = |u: &str| u.trim().trim_end_matches('/').to_ascii_lowercase();
    let target = norm(target);
    html.split('<').any(|chunk| {
        let lower = chunk.get(..5).unwrap_or(chunk).to_ascii_lowercase();
        if !(lower.starts_with("a ") || lower.starts_with("link ")) {
            return false;
        }
        let tag = chunk.split('>').next().unwrap_or("");
        let rel_me = attr(tag, "rel")
            .is_some_and(|r| r.split_whitespace().any(|t| t.eq_ignore_ascii_case("me")));
        rel_me && attr(tag, "href").is_some_and(|h| norm(h) == target)
    })
}

/// Fetch every unverified link (through the SSRF-safe fetcher) and look for
/// a rel="me" link back to this profile.
async fn verify_links(
    State(st): State<AppState>,
    a: Auth,
) -> ApiResult<Json<Vec<v1::ProfileLink>>> {
    let me = repo::user(&st.ctx.db, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    let target = profile_url(&st, &me.username);
    let pending = sqlx::query!(
        "SELECT position, url FROM user_links WHERE user_id = $1 AND verified_at IS NULL",
        a.id() as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    for l in pending {
        let ok = match safe_fetch(&l.url).await {
            Some((_, html)) => has_rel_me(&html, &target),
            None => false,
        };
        if ok {
            sqlx::query!(
                "UPDATE user_links SET verified_at = now() WHERE user_id = $1 AND position = $2",
                a.id() as i64,
                l.position
            )
            .execute(&st.ctx.db)
            .await?;
        }
    }
    Ok(Json(load_links(&st, a.id()).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_me_backlinks() {
        let t = "https://chat.example.ru/u/alice";
        assert!(has_rel_me(
            r#"<a rel="me" href="https://chat.example.ru/u/alice">NeoShard</a>"#,
            t
        ));
        assert!(has_rel_me(
            r#"<link href='https://chat.example.ru/u/alice/' rel='me noopener'>"#,
            t
        ));
        assert!(!has_rel_me(
            r#"<a href="https://chat.example.ru/u/alice">no rel</a>"#,
            t
        ));
        assert!(!has_rel_me(
            r#"<a rel="me" href="https://chat.example.ru/u/alicex">"#,
            t
        ));
        assert!(!has_rel_me(
            r#"<a rel="meh" href="https://chat.example.ru/u/alice">"#,
            t
        ));
    }
}
