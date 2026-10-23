//! Guilds, roles, members, bans, audit log.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use neoshard_proto::permissions::{Permissions as P, can_grant};
use neoshard_proto::v1::{self, Intent, event::Kind};
use neoshard_server_common::repo;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::authz::{audit, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::{self, Auth};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/guilds", post(create_guild))
        .route("/users/@me/guilds", get(my_guilds))
        .route("/users/@me/guilds/{gid}", delete(leave_guild))
        .route(
            "/guilds/{gid}",
            get(get_guild).patch(update_guild).delete(delete_guild),
        )
        .route("/guilds/{gid}/transfer", post(transfer))
        .route(
            "/guilds/{gid}/roles",
            get(list_roles).post(create_role).patch(reorder_roles),
        )
        .route(
            "/guilds/{gid}/roles/{rid}",
            patch(update_role).delete(delete_role),
        )
        .route("/guilds/{gid}/members", get(list_members))
        .route(
            "/guilds/{gid}/members/{uid}",
            get(get_member).patch(update_member).delete(kick),
        )
        .route(
            "/guilds/{gid}/members/{uid}/roles/{rid}",
            put(add_member_role).delete(remove_member_role),
        )
        .route("/guilds/{gid}/bans", get(list_bans))
        .route("/guilds/{gid}/bans/{uid}", put(ban).delete(unban))
        .route("/guilds/{gid}/audit-log", get(audit_log))
}

#[derive(Deserialize)]
struct CreateGuild {
    name: String,
    #[serde(default)]
    icon: String,
}

async fn create_guild(
    State(st): State<AppState>,
    a: Auth,
    Json(r): Json<CreateGuild>,
) -> ApiResult<(StatusCode, Json<v1::Guild>)> {
    if !r.icon.is_empty() {
        return Err(bad(
            "set the icon after creation via /guilds/{gid}/uploads/icon",
        ));
    }
    let name = r.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(bad("name must be 1-100 characters"));
    }
    let owned = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM members WHERE user_id = $1"#,
        a.id() as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if owned >= 200 {
        return Err(bad("guild limit reached (200)"));
    }
    let gid = st.ctx.id();
    let (cat, text, voice) = (st.ctx.id(), st.ctx.id(), st.ctx.id());
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "INSERT INTO guilds (id, name, icon, owner_id) VALUES ($1, $2, $3, $4)",
        gid as i64,
        name,
        r.icon,
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (id, guild_id, name, position, permissions) VALUES ($1, $1, '@everyone', 0, $2)",
        gid as i64,
        P::DEFAULT_EVERYONE.bits() as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO members (guild_id, user_id) VALUES ($1, $2)",
        gid as i64,
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO channels (id, guild_id, type, name, position) VALUES
           ($1, $4, 3, 'Текстовые каналы', 0), ($2, $4, 1, 'общий', 0), ($3, $4, 2, 'Голосовой', 1)",
        cat as i64,
        text as i64,
        voice as i64,
        gid as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE channels SET parent_id = $1 WHERE id IN ($2, $3)",
        cat as i64,
        text as i64,
        voice as i64
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let guild = repo::guild_full(&st.ctx.db, gid, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_user(a.id(), Intent::Guilds, Kind::GuildCreate(guild.clone()));
    Ok((StatusCode::CREATED, Json(guild)))
}

async fn my_guilds(State(st): State<AppState>, a: Auth) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query!(
        "SELECT g.id, g.name, g.icon, g.owner_id FROM guilds g JOIN members m ON m.guild_id = g.id WHERE m.user_id = $1 ORDER BY m.joined_at",
        a.id() as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    Ok(Json(json!(rows.into_iter().map(|g| json!({
        "id": g.id.to_string(), "name": g.name, "icon": g.icon, "owner_id": g.owner_id.to_string()
    })).collect::<Vec<_>>())))
}

async fn get_guild(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<v1::Guild>> {
    guild_access(&st, gid, a.id()).await?;
    Ok(Json(
        repo::guild_full(&st.ctx.db, gid, a.id())
            .await?
            .ok_or(ApiError::NotFound)?,
    ))
}

#[derive(Deserialize)]
struct UpdateGuild {
    name: Option<String>,
    icon: Option<String>,
    /// Server tag ("" removes it) and its badge.
    tag: Option<String>,
    tag_badge: Option<String>,
}

async fn update_guild(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(r): Json<UpdateGuild>,
) -> ApiResult<Json<v1::Guild>> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_GUILD)?;
    if let Some(icon) = &r.icon
        && !icon.is_empty()
        && !icon.starts_with(&format!("icons/{gid}/"))
    {
        return Err(bad("icon must be a key from /guilds/{gid}/uploads/icon"));
    }
    if let Some(n) = &r.name
        && (n.trim().is_empty() || n.chars().count() > 100)
    {
        return Err(bad("name must be 1-100 characters"));
    }
    if let Some(t) = r.tag.as_deref().map(str::trim)
        && !t.is_empty()
        && !neoshard_proto::profile::valid_tag(t)
    {
        return Err(bad("tag: 1-4 letters or digits"));
    }
    if let Some(b) = &r.tag_badge
        && !neoshard_proto::profile::valid(neoshard_proto::profile::TAG_BADGES, b)
    {
        return Err(bad("unknown tag badge"));
    }
    sqlx::query!(
        "UPDATE guilds SET name = COALESCE($2, name), icon = COALESCE($3, icon), tag = COALESCE($4, tag),
                tag_badge = COALESCE($5, tag_badge), updated_at = now() WHERE id = $1",
        gid as i64,
        r.name.as_deref().map(str::trim),
        r.icon,
        r.tag.as_deref().map(str::trim),
        r.tag_badge
    )
    .execute(&st.ctx.db)
    .await?;
    audit(
        &st,
        gid,
        a.id(),
        "guild.update",
        gid,
        json!({ "name": r.name, "icon": r.icon, "tag": r.tag, "tag_badge": r.tag_badge }),
        "",
    )
    .await?;
    let mut g = repo::guild_full(&st.ctx.db, gid, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut lite = g.clone();
    lite.channels.clear();
    lite.roles.clear();
    lite.self_member = None;
    st.publish_guild(gid, Intent::Guilds, Kind::GuildUpdate(lite));
    g.voice_states.clear();
    Ok(Json(g))
}

async fn delete_guild(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    if !ga.is_owner() {
        return Err(ApiError::Forbidden("only the owner can delete a guild"));
    }
    st.publish_guild(
        gid,
        Intent::Guilds,
        Kind::GuildDelete(v1::GuildDelete {
            id: gid,
            unavailable: false,
        }),
    );
    sqlx::query!("DELETE FROM guilds WHERE id = $1", gid as i64)
        .execute(&st.ctx.db)
        .await?;
    sqlx::query!("DELETE FROM messages WHERE guild_id = $1", gid as i64)
        .execute(&st.ctx.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Transfer {
    #[serde(deserialize_with = "extract::id")]
    user_id: u64,
}

async fn transfer(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(r): Json<Transfer>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    if !ga.is_owner() {
        return Err(ApiError::Forbidden("only the owner can transfer a guild"));
    }
    repo::member(&st.ctx.db, gid, r.user_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    sqlx::query!(
        "UPDATE guilds SET owner_id = $2, updated_at = now() WHERE id = $1",
        gid as i64,
        r.user_id as i64
    )
    .execute(&st.ctx.db)
    .await?;
    audit(&st, gid, a.id(), "guild.transfer", r.user_id, json!({}), "").await?;
    let mut lite = repo::guild_full(&st.ctx.db, gid, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    lite.channels.clear();
    lite.roles.clear();
    lite.self_member = None;
    st.publish_guild(gid, Intent::Guilds, Kind::GuildUpdate(lite));
    Ok(StatusCode::NO_CONTENT)
}

async fn leave_guild(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    if ga.is_owner() {
        return Err(ApiError::Conflict(
            "the owner cannot leave; transfer or delete the guild",
        ));
    }
    remove_member(&st, gid, a.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_member(st: &AppState, gid: u64, uid: u64) -> ApiResult<()> {
    sqlx::query!(
        "DELETE FROM members WHERE guild_id = $1 AND user_id = $2",
        gid as i64,
        uid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    st.publish_guild(
        gid,
        Intent::GuildMembers,
        Kind::MemberRemove(v1::MemberRemove {
            guild_id: gid,
            user_id: uid,
        }),
    );
    st.publish_user(
        uid,
        Intent::Guilds,
        Kind::GuildDelete(v1::GuildDelete {
            id: gid,
            unavailable: false,
        }),
    );
    Ok(())
}

// ------------------------------------------------------------------ roles

async fn list_roles(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<Vec<v1::Role>>> {
    guild_access(&st, gid, a.id()).await?;
    Ok(Json(repo::roles(&st.ctx.db, gid).await?))
}

#[derive(Deserialize, Default)]
struct RoleBody {
    name: Option<String>,
    #[serde(default, deserialize_with = "extract::opt_bits")]
    permissions: Option<u64>,
    color: Option<u32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
}

async fn role_by_id(st: &AppState, gid: u64, rid: u64) -> ApiResult<v1::Role> {
    repo::roles(&st.ctx.db, gid)
        .await?
        .into_iter()
        .find(|r| r.id == rid)
        .ok_or(ApiError::NotFound)
}

async fn create_role(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(r): Json<RoleBody>,
) -> ApiResult<(StatusCode, Json<v1::Role>)> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_ROLES)?;
    let perms = r.permissions.unwrap_or(0);
    if !can_grant(ga.perms, P::raw(perms)) {
        return Err(ApiError::Forbidden(
            "cannot grant permissions you do not have",
        ));
    }
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM roles WHERE guild_id = $1"#,
        gid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if count >= 250 {
        return Err(bad("role limit reached (250)"));
    }
    let id = st.ctx.id();
    let name = r.name.unwrap_or_else(|| "новая роль".into());
    // New roles go right above @everyone; existing roles shift up.
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "UPDATE roles SET position = position + 1 WHERE guild_id = $1 AND id <> $1",
        gid as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (id, guild_id, name, color, position, permissions, hoist, mentionable) VALUES ($1, $2, $3, $4, 1, $5, $6, $7)",
        id as i64,
        gid as i64,
        name.trim(),
        r.color.unwrap_or(0) as i32,
        perms as i64,
        r.hoist.unwrap_or(false),
        r.mentionable.unwrap_or(false)
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    audit(
        &st,
        gid,
        a.id(),
        "role.create",
        id,
        json!({ "name": name, "permissions": perms.to_string() }),
        "",
    )
    .await?;
    for role in repo::roles(&st.ctx.db, gid).await? {
        let kind = if role.id == id {
            Kind::RoleCreate(role)
        } else {
            Kind::RoleUpdate(role)
        };
        st.publish_guild(gid, Intent::Guilds, kind);
    }
    Ok((StatusCode::CREATED, Json(role_by_id(&st, gid, id).await?)))
}

async fn update_role(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, rid)): Path<(u64, u64)>,
    Json(r): Json<RoleBody>,
) -> ApiResult<Json<v1::Role>> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_ROLES)?;
    let role = role_by_id(&st, gid, rid).await?;
    if rid != gid && !ga.can_manage_role_at(role.position) {
        return Err(ApiError::Forbidden("role is above your highest role"));
    }
    if let Some(p) = r.permissions {
        // Only bits that change need to be held by the actor.
        let changed = P::raw(p ^ role.permissions);
        if !can_grant(ga.perms, changed) {
            return Err(ApiError::Forbidden(
                "cannot grant permissions you do not have",
            ));
        }
    }
    if rid == gid && r.name.is_some() {
        return Err(bad("@everyone cannot be renamed"));
    }
    sqlx::query!(
        "UPDATE roles SET name = COALESCE($3, name), permissions = COALESCE($4, permissions), color = COALESCE($5, color),
                hoist = COALESCE($6, hoist), mentionable = COALESCE($7, mentionable)
         WHERE id = $1 AND guild_id = $2",
        rid as i64,
        gid as i64,
        r.name.as_deref().map(str::trim),
        r.permissions.map(|p| p as i64),
        r.color.map(|c| c as i32),
        r.hoist,
        r.mentionable
    )
    .execute(&st.ctx.db)
    .await?;
    let updated = role_by_id(&st, gid, rid).await?;
    audit(&st, gid, a.id(), "role.update", rid, json!({
        "permissions": [role.permissions.to_string(), updated.permissions.to_string()], "name": [role.name, updated.name.clone()]
    }), "").await?;
    st.publish_guild(gid, Intent::Guilds, Kind::RoleUpdate(updated.clone()));
    Ok(Json(updated))
}

#[derive(Deserialize)]
struct Position {
    #[serde(deserialize_with = "extract::id")]
    id: u64,
    position: i32,
}

async fn reorder_roles(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(list): Json<Vec<Position>>,
) -> ApiResult<Json<Vec<v1::Role>>> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_ROLES)?;
    let roles = repo::roles(&st.ctx.db, gid).await?;
    let mut tx = st.ctx.db.begin().await?;
    for p in &list {
        let role = roles
            .iter()
            .find(|r| r.id == p.id)
            .ok_or(ApiError::NotFound)?;
        if p.id == gid || p.position < 1 {
            return Err(bad("@everyone stays at position 0"));
        }
        if !ga.can_manage_role_at(role.position) || !ga.can_manage_role_at(p.position) {
            return Err(ApiError::Forbidden(
                "cannot move roles at or above your highest role",
            ));
        }
        sqlx::query!(
            "UPDATE roles SET position = $3 WHERE id = $1 AND guild_id = $2",
            p.id as i64,
            gid as i64,
            p.position
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let roles = repo::roles(&st.ctx.db, gid).await?;
    for r in &roles {
        st.publish_guild(gid, Intent::Guilds, Kind::RoleUpdate(r.clone()));
    }
    Ok(Json(roles))
}

async fn delete_role(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, rid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_ROLES)?;
    if rid == gid {
        return Err(bad("@everyone cannot be deleted"));
    }
    let role = role_by_id(&st, gid, rid).await?;
    if !ga.can_manage_role_at(role.position) {
        return Err(ApiError::Forbidden("role is above your highest role"));
    }
    sqlx::query!(
        "DELETE FROM roles WHERE id = $1 AND guild_id = $2",
        rid as i64,
        gid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    sqlx::query!(
        "DELETE FROM permission_overwrites WHERE target_id = $1 AND target_type = 0",
        rid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    audit(
        &st,
        gid,
        a.id(),
        "role.delete",
        rid,
        json!({ "name": role.name }),
        "",
    )
    .await?;
    st.publish_guild(
        gid,
        Intent::Guilds,
        Kind::RoleDelete(v1::RoleDelete {
            id: rid,
            guild_id: gid,
        }),
    );
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ members

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    offset: i64,
    limit: Option<i64>,
}

async fn list_members(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Query(p): Query<Page>,
) -> ApiResult<Json<Vec<v1::Member>>> {
    guild_access(&st, gid, a.id()).await?;
    let limit = p.limit.unwrap_or(100).clamp(1, 1000);
    Ok(Json(
        repo::members_page(&st.ctx.db, gid, p.offset.max(0), limit).await?,
    ))
}

async fn get_member(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, uid)): Path<(u64, String)>,
) -> ApiResult<Json<v1::Member>> {
    guild_access(&st, gid, a.id()).await?;
    let uid = resolve_me(&uid, a.id())?;
    Ok(Json(
        repo::member(&st.ctx.db, gid, uid)
            .await?
            .ok_or(ApiError::NotFound)?,
    ))
}

fn resolve_me(raw: &str, me: u64) -> ApiResult<u64> {
    if raw == "@me" {
        Ok(me)
    } else {
        raw.parse().map_err(|_| bad("invalid user id"))
    }
}

#[derive(Deserialize)]
struct UpdateMember {
    nick: Option<String>,
    /// unix millis; 0 or null clears the timeout
    timeout_until: Option<i64>,
    #[serde(default, deserialize_with = "opt_ids")]
    role_ids: Option<Vec<u64>>,
    #[serde(default)]
    reason: String,
    // Per-server profile (only for yourself); "" / [] falls back to the user profile.
    avatar: Option<String>,
    banner: Option<String>,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_colors: Option<Vec<u32>>,
}

fn opt_ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<u64>>, D::Error> {
    extract::ids(d).map(Some)
}

async fn publish_member(st: &AppState, gid: u64, uid: u64) -> ApiResult<v1::Member> {
    let m = repo::member(&st.ctx.db, gid, uid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_guild(gid, Intent::GuildMembers, Kind::MemberUpdate(m.clone()));
    Ok(m)
}

async fn update_member(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, uid)): Path<(u64, String)>,
    Json(r): Json<UpdateMember>,
) -> ApiResult<Json<v1::Member>> {
    let uid = resolve_me(&uid, a.id())?;
    let ga = guild_access(&st, gid, a.id()).await?;
    let target = repo::member(&st.ctx.db, gid, uid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let is_self = uid == a.id();
    let profile_change = r.avatar.is_some()
        || r.banner.is_some()
        || r.bio.is_some()
        || r.pronouns.is_some()
        || r.theme_colors.is_some();
    if profile_change {
        use crate::routes::uploads::member_key_ok;
        use neoshard_proto::profile as pf;
        if !is_self {
            return Err(ApiError::Forbidden("only you can edit your server profile"));
        }
        if r.avatar
            .as_deref()
            .is_some_and(|k| !member_key_ok(k, gid, uid))
            || r.banner
                .as_deref()
                .is_some_and(|k| !member_key_ok(k, gid, uid))
        {
            return Err(bad(
                "images must be keys from /guilds/{gid}/uploads/member-avatar|member-banner",
            ));
        }
        if r.bio
            .as_ref()
            .is_some_and(|b| b.chars().count() > pf::BIO_MAX)
            || r.pronouns
                .as_ref()
                .is_some_and(|p| p.chars().count() > pf::PRONOUNS_MAX)
        {
            return Err(bad("bio ≤ 190, pronouns ≤ 40 characters"));
        }
        if let Some(c) = &r.theme_colors
            && (!(c.is_empty() || c.len() == 2) || !c.iter().all(|&x| pf::valid_color(x)))
        {
            return Err(bad("theme colors: two 0xRRGGBB values or none"));
        }
        sqlx::query!(
            "UPDATE members SET avatar = COALESCE($3, avatar), banner = COALESCE($4, banner), bio = COALESCE($5, bio),
                    pronouns = COALESCE($6, pronouns), theme_colors = COALESCE($7, theme_colors)
             WHERE guild_id = $1 AND user_id = $2",
            gid as i64,
            uid as i64,
            r.avatar,
            r.banner,
            r.bio.as_deref().map(str::trim),
            r.pronouns.as_deref().map(str::trim),
            r.theme_colors.as_ref().map(|c| c.iter().map(|&x| x as i32).collect::<Vec<_>>()) as Option<Vec<i32>>
        )
        .execute(&st.ctx.db)
        .await?;
    }
    if let Some(nick) = &r.nick {
        if is_self {
            ga.require(P::CHANGE_NICKNAME)?;
        } else {
            ga.require(P::MANAGE_NICKNAMES)?;
            if !ga.can_act_on(&target) {
                return Err(ApiError::Forbidden("target has a higher or equal role"));
            }
        }
        if nick.chars().count() > 64 {
            return Err(bad("nick too long"));
        }
        sqlx::query!(
            "UPDATE members SET nick = $3 WHERE guild_id = $1 AND user_id = $2",
            gid as i64,
            uid as i64,
            nick.trim()
        )
        .execute(&st.ctx.db)
        .await?;
    }
    if let Some(until) = r.timeout_until {
        ga.require(P::MODERATE_MEMBERS)?;
        if is_self || !ga.can_act_on(&target) {
            return Err(ApiError::Forbidden("cannot time out this member"));
        }
        let max = repo::now_ms() + 28 * 24 * 3600 * 1000;
        if until > max {
            return Err(bad("timeout can be at most 28 days"));
        }
        let ts = (until > repo::now_ms())
            .then(|| chrono::DateTime::from_timestamp_millis(until))
            .flatten();
        sqlx::query!(
            "UPDATE members SET timeout_until = $3 WHERE guild_id = $1 AND user_id = $2",
            gid as i64,
            uid as i64,
            ts
        )
        .execute(&st.ctx.db)
        .await?;
        audit(
            &st,
            gid,
            a.id(),
            if ts.is_some() {
                "member.timeout"
            } else {
                "member.timeout_clear"
            },
            uid,
            json!({ "until": until }),
            &r.reason,
        )
        .await?;
    }
    if let Some(role_ids) = r.role_ids {
        ga.require(P::MANAGE_ROLES)?;
        if !is_self && !ga.can_act_on(&target) {
            return Err(ApiError::Forbidden("target has a higher or equal role"));
        }
        let all = repo::roles(&st.ctx.db, gid).await?;
        let mut tx = st.ctx.db.begin().await?;
        // Roles the actor cannot manage must be left untouched.
        let current: Vec<u64> = target.role_ids.clone();
        for rid in current.iter().chain(role_ids.iter()) {
            let role = all
                .iter()
                .find(|x| x.id == *rid && x.id != gid)
                .ok_or(bad("unknown role"))?;
            let changing = current.contains(rid) != role_ids.contains(rid);
            if changing && !ga.can_manage_role_at(role.position) {
                return Err(ApiError::Forbidden(
                    "cannot assign roles at or above your highest role",
                ));
            }
        }
        sqlx::query!(
            "DELETE FROM member_roles WHERE guild_id = $1 AND user_id = $2",
            gid as i64,
            uid as i64
        )
        .execute(&mut *tx)
        .await?;
        for rid in &role_ids {
            sqlx::query!("INSERT INTO member_roles (guild_id, user_id, role_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING", gid as i64, uid as i64, *rid as i64)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        audit(
            &st,
            gid,
            a.id(),
            "member.roles",
            uid,
            json!({ "role_ids": role_ids.iter().map(u64::to_string).collect::<Vec<_>>() }),
            &r.reason,
        )
        .await?;
    }
    Ok(Json(publish_member(&st, gid, uid).await?))
}

async fn add_member_role(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, uid, rid)): Path<(u64, u64, u64)>,
) -> ApiResult<StatusCode> {
    member_role(&st, a, gid, uid, rid, true).await
}

async fn remove_member_role(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, uid, rid)): Path<(u64, u64, u64)>,
) -> ApiResult<StatusCode> {
    member_role(&st, a, gid, uid, rid, false).await
}

async fn member_role(
    st: &AppState,
    a: Auth,
    gid: u64,
    uid: u64,
    rid: u64,
    add: bool,
) -> ApiResult<StatusCode> {
    let ga = guild_access(st, gid, a.id()).await?;
    ga.require(P::MANAGE_ROLES)?;
    let role = role_by_id(st, gid, rid).await?;
    if rid == gid || !ga.can_manage_role_at(role.position) {
        return Err(ApiError::Forbidden("cannot assign this role"));
    }
    let target = repo::member(&st.ctx.db, gid, uid)
        .await?
        .ok_or(ApiError::NotFound)?;
    if uid != a.id() && !ga.can_act_on(&target) {
        return Err(ApiError::Forbidden("target has a higher or equal role"));
    }
    if add {
        sqlx::query!("INSERT INTO member_roles (guild_id, user_id, role_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING", gid as i64, uid as i64, rid as i64)
            .execute(&st.ctx.db)
            .await?;
    } else {
        sqlx::query!(
            "DELETE FROM member_roles WHERE guild_id = $1 AND user_id = $2 AND role_id = $3",
            gid as i64,
            uid as i64,
            rid as i64
        )
        .execute(&st.ctx.db)
        .await?;
    }
    audit(
        st,
        gid,
        a.id(),
        if add {
            "member.role_add"
        } else {
            "member.role_remove"
        },
        uid,
        json!({ "role_id": rid.to_string() }),
        "",
    )
    .await?;
    publish_member(st, gid, uid).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct ReasonQuery {
    #[serde(default)]
    reason: String,
}

async fn kick(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, uid)): Path<(u64, u64)>,
    Query(q): Query<ReasonQuery>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::KICK_MEMBERS)?;
    let target = repo::member(&st.ctx.db, gid, uid)
        .await?
        .ok_or(ApiError::NotFound)?;
    if uid == a.id() || !ga.can_act_on(&target) {
        return Err(ApiError::Forbidden("cannot kick this member"));
    }
    remove_member(&st, gid, uid).await?;
    audit(&st, gid, a.id(), "member.kick", uid, json!({}), &q.reason).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ bans

async fn list_bans(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::BAN_MEMBERS)?;
    let rows = sqlx::query!(
        "SELECT user_id, reason, created_at FROM bans WHERE guild_id = $1 ORDER BY created_at DESC",
        gid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    let ids: Vec<i64> = rows.iter().map(|r| r.user_id).collect();
    let users = repo::users(&st.ctx.db, &ids).await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({
                "user": users.get(&(r.user_id as u64)),
                "reason": r.reason,
                "created_at": r.created_at.timestamp_millis(),
            }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize, Default)]
struct BanBody {
    #[serde(default)]
    reason: String,
    /// Delete the user's messages from the last N seconds (max 7 days).
    #[serde(default)]
    delete_message_seconds: u32,
}

async fn ban(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, uid)): Path<(u64, u64)>,
    body: Option<Json<BanBody>>,
) -> ApiResult<StatusCode> {
    let r = body.map(|b| b.0).unwrap_or_default();
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::BAN_MEMBERS)?;
    if uid == a.id() || uid == ga.snap.owner_id {
        return Err(ApiError::Forbidden("cannot ban this user"));
    }
    let target = repo::member(&st.ctx.db, gid, uid).await?;
    if let Some(t) = &target
        && !ga.can_act_on(t)
    {
        return Err(ApiError::Forbidden("target has a higher or equal role"));
    }
    let user = repo::user(&st.ctx.db, uid)
        .await?
        .ok_or(ApiError::NotFound)?
        .public();
    // Ban first, then remove: an invite accepted in between is refused.
    sqlx::query!(
        "INSERT INTO bans (guild_id, user_id, reason, banned_by) VALUES ($1, $2, $3, $4)
         ON CONFLICT (guild_id, user_id) DO UPDATE SET reason = EXCLUDED.reason",
        gid as i64,
        uid as i64,
        r.reason.chars().take(512).collect::<String>(),
        a.id() as i64
    )
    .execute(&st.ctx.db)
    .await?;
    if target.is_some() {
        remove_member(&st, gid, uid).await?;
    }
    let secs = r.delete_message_seconds.min(7 * 24 * 3600);
    if secs > 0 {
        let since = neoshard_proto::snowflake::from_timestamp(
            (repo::now_ms() as u64).saturating_sub(u64::from(secs) * 1000),
        );
        let deleted = sqlx::query!(
            "DELETE FROM messages WHERE guild_id = $1 AND author_id = $2 AND id >= $3 RETURNING id, channel_id",
            gid as i64,
            uid as i64,
            since as i64
        )
        .fetch_all(&st.ctx.db)
        .await?;
        let mut by_channel: std::collections::HashMap<u64, Vec<u64>> = Default::default();
        for d in deleted {
            by_channel
                .entry(d.channel_id as u64)
                .or_default()
                .push(d.id as u64);
        }
        for (cid, ids) in by_channel {
            for chunk in ids.chunks(100) {
                let ev = Kind::MessageDeleteBulk(v1::MessageDeleteBulk {
                    ids: chunk.to_vec(),
                    channel_id: cid,
                    guild_id: gid,
                });
                st.publish(neoshard_server_common::bus::BusEvent::channel(
                    gid,
                    cid,
                    Intent::GuildMessages,
                    ev,
                ));
            }
        }
    }
    audit(
        &st,
        gid,
        a.id(),
        "member.ban",
        uid,
        json!({ "delete_message_seconds": secs }),
        &r.reason,
    )
    .await?;
    st.publish_moderation(
        gid,
        P::BAN_MEMBERS,
        Kind::BanAdd(v1::BanEvent {
            guild_id: gid,
            user: Some(user),
        }),
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn unban(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, uid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::BAN_MEMBERS)?;
    let r = sqlx::query!(
        "DELETE FROM bans WHERE guild_id = $1 AND user_id = $2",
        gid as i64,
        uid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    audit(&st, gid, a.id(), "member.unban", uid, json!({}), "").await?;
    let user = repo::user(&st.ctx.db, uid).await?.map(|u| u.public());
    st.publish_moderation(
        gid,
        P::BAN_MEMBERS,
        Kind::BanRemove(v1::BanEvent {
            guild_id: gid,
            user,
        }),
    );
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ audit log

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default, deserialize_with = "extract::opt_id")]
    before: Option<u64>,
    limit: Option<i64>,
    action: Option<String>,
}

async fn audit_log(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<v1::AuditLogEntry>>> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::VIEW_AUDIT_LOG)?;
    let rows = sqlx::query!(
        "SELECT id, user_id, action, target_id, changes, reason FROM audit_log
         WHERE guild_id = $1 AND id < $2 AND ($3::text IS NULL OR action = $3)
         ORDER BY id DESC LIMIT $4",
        gid as i64,
        q.before.map(|b| b as i64).unwrap_or(i64::MAX),
        q.action,
        q.limit.unwrap_or(50).clamp(1, 100)
    )
    .fetch_all(&st.ctx.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| v1::AuditLogEntry {
                id: r.id as u64,
                guild_id: gid,
                user_id: r.user_id.unwrap_or(0) as u64,
                action: r.action,
                target_id: r.target_id as u64,
                changes_json: r.changes.to_string(),
                reason: r.reason,
            })
            .collect(),
    ))
}
