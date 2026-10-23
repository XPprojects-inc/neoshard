//! Channels, permission overwrites, threads, forum posts, invites.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use neoshard_proto::permissions::{Permissions as P, can_grant};
use neoshard_proto::v1::{self, ChannelType, Intent, OverwriteType, event::Kind};
use neoshard_server_common::repo;
use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::authz::{audit, channel_access, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::{self, Auth, ClientIp};
use crate::ratelimit::rates;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/guilds/{gid}/channels",
            get(list_channels)
                .post(create_channel)
                .patch(reorder_channels),
        )
        .route(
            "/channels/{cid}",
            get(get_channel)
                .patch(update_channel)
                .delete(delete_channel),
        )
        .route(
            "/channels/{cid}/overwrites/{target}",
            put(put_overwrite).delete(delete_overwrite),
        )
        .route("/channels/{cid}/threads", post(create_thread))
        .route(
            "/channels/{cid}/threads/{tid}/archive",
            post(archive_thread),
        )
        .route(
            "/channels/{cid}/invites",
            get(list_invites).post(create_invite),
        )
        .route(
            "/invites/{code}",
            get(get_invite).post(accept_invite).delete(delete_invite),
        )
        .route("/channels/{cid}/forum-tags", post(create_forum_tag))
        .route("/channels/{cid}/forum-tags/{tag}", delete(delete_forum_tag))
        .route("/channels/{cid}/followers", patch(follow_announcement))
}

fn kind_from(t: i32) -> ApiResult<ChannelType> {
    let ct = ChannelType::try_from(t).map_err(|_| bad("unknown channel type"))?;
    match ct {
        ChannelType::Text
        | ChannelType::Voice
        | ChannelType::Category
        | ChannelType::Announcement
        | ChannelType::Forum => Ok(ct),
        _ => Err(bad("this channel type cannot be created here")),
    }
}

fn clean_name(name: &str, ct: ChannelType) -> ApiResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 100 {
        return Err(bad("name must be 1-100 characters"));
    }
    // Text-like channels use slug names (lowercase, dashes), as users expect.
    if matches!(
        ct,
        ChannelType::Text | ChannelType::Announcement | ChannelType::Forum
    ) {
        return Ok(n
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-"));
    }
    Ok(n.to_string())
}

#[derive(Deserialize)]
struct OverwriteBody {
    #[serde(deserialize_with = "extract::id")]
    id: u64,
    #[serde(rename = "type", default)]
    kind: i32,
    #[serde(default, deserialize_with = "extract::bits")]
    allow: u64,
    #[serde(default, deserialize_with = "extract::bits")]
    deny: u64,
}

#[derive(Deserialize)]
struct CreateChannel {
    name: String,
    #[serde(rename = "type", default = "default_type")]
    kind: i32,
    #[serde(default, deserialize_with = "extract::opt_id")]
    parent_id: Option<u64>,
    #[serde(default)]
    topic: String,
    position: Option<i32>,
    #[serde(default)]
    nsfw: bool,
    #[serde(default)]
    slowmode_seconds: u32,
    #[serde(default)]
    overwrites: Vec<OverwriteBody>,
}

fn default_type() -> i32 {
    ChannelType::Text as i32
}

async fn list_channels(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<Vec<v1::Channel>>> {
    guild_access(&st, gid, a.id()).await?;
    Ok(Json(
        repo::guild_full(&st.ctx.db, gid, a.id())
            .await?
            .ok_or(ApiError::NotFound)?
            .channels,
    ))
}

async fn create_channel(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(r): Json<CreateChannel>,
) -> ApiResult<(StatusCode, Json<v1::Channel>)> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_CHANNELS)?;
    let ct = kind_from(r.kind)?;
    let name = clean_name(&r.name, ct)?;
    if r.slowmode_seconds > 21600 || r.topic.chars().count() > 1024 {
        return Err(bad("slowmode ≤ 21600 s, topic ≤ 1024 chars"));
    }
    if let Some(p) = r.parent_id {
        let parent = ga.snap.channels.get(&p).ok_or(bad("unknown parent"))?;
        if parent.2 != ChannelType::Category as i32 || ct == ChannelType::Category {
            return Err(bad(
                "parent must be a category, categories cannot be nested",
            ));
        }
    }
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM channels WHERE guild_id = $1"#,
        gid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if count >= 500 {
        return Err(bad("channel limit reached (500)"));
    }
    if !r.overwrites.is_empty() {
        ga.require(P::MANAGE_ROLES)?;
    }
    for o in &r.overwrites {
        if OverwriteType::try_from(o.kind).is_err() || o.allow & o.deny != 0 {
            return Err(bad("invalid overwrite"));
        }
        if !can_grant(ga.perms, P::raw(o.allow | o.deny)) {
            return Err(ApiError::Forbidden(
                "cannot set overwrites for permissions you do not have",
            ));
        }
        if o.kind == OverwriteType::Role as i32 && o.id != gid && !ga.snap.roles.contains_key(&o.id)
        {
            return Err(bad("unknown role"));
        }
        check_overwrite_target(&st, &ga, gid, o.id, o.kind).await?;
    }
    let id = st.ctx.id();
    let position = match r.position {
        Some(p) => p,
        None => sqlx::query_scalar!(
            r#"SELECT COALESCE(max(position) + 1, 0) AS "p!" FROM channels WHERE guild_id = $1"#,
            gid as i64
        )
        .fetch_one(&st.ctx.db)
        .await?,
    };
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "INSERT INTO channels (id, guild_id, type, name, topic, position, parent_id, nsfw, slowmode_seconds) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        id as i64,
        gid as i64,
        ct as i16,
        name,
        r.topic,
        position,
        r.parent_id.map(|p| p as i64),
        r.nsfw,
        r.slowmode_seconds as i32
    )
    .execute(&mut *tx)
    .await?;
    for o in &r.overwrites {
        sqlx::query!(
            "INSERT INTO permission_overwrites (channel_id, target_id, target_type, allow, deny) VALUES ($1, $2, $3, $4, $5)",
            id as i64,
            o.id as i64,
            o.kind as i16,
            o.allow as i64,
            o.deny as i64
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let ch = repo::channel(&st.ctx.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    audit(
        &st,
        gid,
        a.id(),
        "channel.create",
        id,
        json!({ "name": ch.name, "type": ch.r#type }),
        "",
    )
    .await?;
    st.publish_channel_event(&ch, Intent::Guilds, Kind::ChannelCreate(ch.clone()))
        .await;
    Ok((StatusCode::CREATED, Json(ch)))
}

async fn get_channel(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
) -> ApiResult<Json<v1::Channel>> {
    Ok(Json(channel_access(&st, cid, a.id()).await?.channel))
}

#[derive(Deserialize)]
struct UpdateChannel {
    name: Option<String>,
    topic: Option<String>,
    position: Option<i32>,
    #[serde(default, deserialize_with = "extract::opt_id")]
    parent_id: Option<u64>,
    #[serde(default)]
    clear_parent: bool,
    nsfw: Option<bool>,
    slowmode_seconds: Option<u32>,
}

async fn update_channel(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(r): Json<UpdateChannel>,
) -> ApiResult<Json<v1::Channel>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    let gid = ca.guild_id();
    if ca.is_dm() {
        // Group DM: only name, by any recipient.
        if ca.channel.r#type != ChannelType::GroupDm as i32 {
            return Err(ApiError::Forbidden("1:1 DMs cannot be edited"));
        }
        if let Some(n) = &r.name {
            sqlx::query!(
                "UPDATE channels SET name = $2 WHERE id = $1",
                cid as i64,
                n.trim().chars().take(100).collect::<String>()
            )
            .execute(&st.ctx.db)
            .await?;
        }
    } else {
        ca.require(P::MANAGE_CHANNELS)?;
        let ct = ChannelType::try_from(ca.channel.r#type).unwrap_or(ChannelType::Text);
        if let Some(p) = r.parent_id {
            let ga = ca.guild.as_ref().ok_or(bad("no guild"))?;
            valid_parent(ga, ct, p)?;
        }
        let name = r.name.as_deref().map(|n| clean_name(n, ct)).transpose()?;
        if r.slowmode_seconds.is_some_and(|s| s > 21600) {
            return Err(bad("slowmode ≤ 21600 s"));
        }
        sqlx::query!(
            "UPDATE channels SET name = COALESCE($2, name), topic = COALESCE($3, topic), position = COALESCE($4, position),
                    parent_id = CASE WHEN $5 THEN NULL ELSE COALESCE($6, parent_id) END,
                    nsfw = COALESCE($7, nsfw), slowmode_seconds = COALESCE($8, slowmode_seconds)
             WHERE id = $1",
            cid as i64,
            name,
            r.topic,
            r.position,
            r.clear_parent,
            r.parent_id.map(|p| p as i64),
            r.nsfw,
            r.slowmode_seconds.map(|s| s as i32)
        )
        .execute(&st.ctx.db)
        .await?;
        audit(
            &st,
            gid,
            a.id(),
            "channel.update",
            cid,
            json!({ "name": r.name, "topic": r.topic }),
            "",
        )
        .await?;
    }
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(&ch, Intent::Guilds, Kind::ChannelUpdate(ch.clone()))
        .await;
    Ok(Json(ch))
}

#[derive(Deserialize)]
struct ChannelPosition {
    #[serde(deserialize_with = "extract::id")]
    id: u64,
    position: i32,
    #[serde(default, deserialize_with = "extract::opt_id")]
    parent_id: Option<u64>,
}

/// Batch reorder (drag & drop in the sidebar).
async fn reorder_channels(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(list): Json<Vec<ChannelPosition>>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_CHANNELS)?;
    if list.len() > 500 {
        return Err(bad("too many channels"));
    }
    let mut tx = st.ctx.db.begin().await?;
    let now = repo::now_ms();
    for p in &list {
        // Hidden channels look like missing ones.
        if !ga
            .snap
            .channel_perms(&ga.member, p.id, now)
            .contains(P::VIEW_CHANNEL)
        {
            return Err(ApiError::NotFound);
        }
        if let Some(parent) = p.parent_id {
            let ct = ChannelType::try_from(ga.snap.channels.get(&p.id).map(|c| c.2).unwrap_or(0))
                .unwrap_or(ChannelType::Text);
            valid_parent(&ga, ct, parent)?;
        }
        sqlx::query!(
            "UPDATE channels SET position = $3, parent_id = $4 WHERE id = $1 AND guild_id = $2",
            p.id as i64,
            gid as i64,
            p.position,
            p.parent_id.map(|x| x as i64)
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    for p in &list {
        if let Some(ch) = repo::channel(&st.ctx.db, p.id).await? {
            st.publish_channel_event(&ch, Intent::Guilds, Kind::ChannelUpdate(ch.clone()))
                .await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_channel(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if ca.is_dm() {
        // Leaving a DM: remove self from recipients.
        sqlx::query!(
            "DELETE FROM channel_recipients WHERE channel_id = $1 AND user_id = $2",
            cid as i64,
            a.id() as i64
        )
        .execute(&st.ctx.db)
        .await?;
        st.publish_user(
            a.id(),
            Intent::DirectMessages,
            Kind::ChannelDelete(v1::ChannelDelete {
                id: cid,
                guild_id: 0,
            }),
        );
        return Ok(StatusCode::NO_CONTENT);
    }
    let thread_owner =
        ca.channel.r#type == ChannelType::Thread as i32 && ca.channel.owner_id == a.id();
    if !thread_owner {
        ca.require(P::MANAGE_CHANNELS)?;
    }
    let gid = ca.guild_id();
    sqlx::query!("DELETE FROM channels WHERE id = $1", cid as i64)
        .execute(&st.ctx.db)
        .await?;
    sqlx::query!("DELETE FROM messages WHERE channel_id = $1", cid as i64)
        .execute(&st.ctx.db)
        .await?;
    // Published after the delete so refreshed permission snapshots no longer contain it.
    st.publish_channel_event(
        &ca.channel,
        Intent::Guilds,
        Kind::ChannelDelete(v1::ChannelDelete {
            id: cid,
            guild_id: gid,
        }),
    )
    .await;
    audit(
        &st,
        gid,
        a.id(),
        "channel.delete",
        cid,
        json!({ "name": ca.channel.name }),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ overwrites

#[derive(Deserialize)]
struct PutOverwrite {
    #[serde(rename = "type", default)]
    kind: i32,
    #[serde(default, deserialize_with = "extract::bits")]
    allow: u64,
    #[serde(default, deserialize_with = "extract::bits")]
    deny: u64,
}

async fn put_overwrite(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, target)): Path<(u64, u64)>,
    Json(r): Json<PutOverwrite>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    let ga = ca.guild.as_ref().ok_or(bad("DMs have no overwrites"))?;
    ca.require(P::MANAGE_ROLES)?;
    if OverwriteType::try_from(r.kind).is_err() {
        return Err(bad("type must be 0 (role) or 1 (member)"));
    }
    if r.allow & r.deny != 0 {
        return Err(bad("allow and deny overlap"));
    }
    // Only bits the actor holds in this channel can be changed.
    if !(ga.perms.contains(P::ADMINISTRATOR) || ca.perms.contains(P::raw(r.allow | r.deny))) {
        return Err(ApiError::Forbidden(
            "cannot set overwrites for permissions you do not have",
        ));
    }
    if r.kind == OverwriteType::Role as i32
        && target != ca.guild_id()
        && !ga.snap.roles.contains_key(&target)
    {
        return Err(bad("unknown role"));
    }
    check_overwrite_target(&st, ga, ca.guild_id(), target, r.kind).await?;
    // Changing an existing overwrite also needs the bits being removed.
    if let Some(old) = ca.channel.overwrites.iter().find(|o| o.id == target)
        && !(ga.perms.contains(P::ADMINISTRATOR) || ca.perms.contains(P::raw(old.allow | old.deny)))
    {
        return Err(ApiError::Forbidden(
            "cannot change overwrites for permissions you do not have",
        ));
    }
    sqlx::query!(
        "INSERT INTO permission_overwrites (channel_id, target_id, target_type, allow, deny) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (channel_id, target_id) DO UPDATE SET target_type = EXCLUDED.target_type, allow = EXCLUDED.allow, deny = EXCLUDED.deny",
        cid as i64,
        target as i64,
        r.kind as i16,
        r.allow as i64,
        r.deny as i64
    )
    .execute(&st.ctx.db)
    .await?;
    audit(&st, ca.guild_id(), a.id(), "channel.overwrite", cid, json!({ "target": target.to_string(), "allow": r.allow.to_string(), "deny": r.deny.to_string() }), "").await?;
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Broadcast to the whole guild: members who just lost access must learn it too.
    st.publish_guild(ch.guild_id, Intent::Guilds, Kind::ChannelUpdate(ch));
    Ok(StatusCode::NO_CONTENT)
}

/// A channel's parent must be a category of the same guild; threads keep their parent.
fn valid_parent(ga: &crate::authz::GuildAccess, child: ChannelType, parent: u64) -> ApiResult<()> {
    if child == ChannelType::Thread || child == ChannelType::Category {
        return Err(bad("threads and categories cannot be re-parented"));
    }
    match ga.snap.channels.get(&parent) {
        Some((_, _, t)) if *t == ChannelType::Category as i32 => Ok(()),
        _ => Err(bad("parent must be a category in this guild")),
    }
}

/// Role hierarchy for overwrites: only roles strictly below the actor's top
/// role, and only members the actor may act on.
async fn check_overwrite_target(
    st: &AppState,
    ga: &crate::authz::GuildAccess,
    gid: u64,
    target: u64,
    kind: i32,
) -> ApiResult<()> {
    if ga.is_owner() || ga.perms.contains(P::ADMINISTRATOR) {
        return Ok(());
    }
    if kind == OverwriteType::Role as i32 {
        if target != gid {
            let pos = ga.snap.roles.get(&target).map(|r| r.0).unwrap_or(i32::MAX);
            if !ga.can_manage_role_at(pos) {
                return Err(ApiError::Forbidden("role is above your highest role"));
            }
        }
    } else if let Some(m) = repo::member(&st.ctx.db, gid, target).await?
        && target != ga.member.user.as_ref().map(|u| u.id).unwrap_or(0)
        && !ga.can_act_on(&m)
    {
        return Err(ApiError::Forbidden("target has a higher or equal role"));
    }
    Ok(())
}

async fn delete_overwrite(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, target)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::MANAGE_ROLES)?;
    let ga = ca.guild.as_ref().ok_or(bad("DMs have no overwrites"))?;
    let Some(old) = ca.channel.overwrites.iter().find(|o| o.id == target) else {
        return Err(ApiError::NotFound);
    };
    check_overwrite_target(&st, ga, ca.guild_id(), target, old.r#type).await?;
    if !(ga.perms.contains(P::ADMINISTRATOR) || ca.perms.contains(P::raw(old.allow | old.deny))) {
        return Err(ApiError::Forbidden(
            "cannot remove overwrites for permissions you do not have",
        ));
    }
    sqlx::query!(
        "DELETE FROM permission_overwrites WHERE channel_id = $1 AND target_id = $2",
        cid as i64,
        target as i64
    )
    .execute(&st.ctx.db)
    .await?;
    audit(
        &st,
        ca.guild_id(),
        a.id(),
        "channel.overwrite_delete",
        cid,
        json!({ "target": target.to_string() }),
        "",
    )
    .await?;
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_guild(ch.guild_id, Intent::Guilds, Kind::ChannelUpdate(ch));
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ threads & forums

#[derive(Deserialize)]
struct CreateThread {
    name: String,
    /// Start the thread from an existing message.
    #[serde(default, deserialize_with = "extract::opt_id")]
    message_id: Option<u64>,
    /// Forum posts: the first message.
    content: Option<String>,
}

async fn create_thread(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(r): Json<CreateThread>,
) -> ApiResult<(StatusCode, Json<v1::Channel>)> {
    let ca = channel_access(&st, cid, a.id()).await?;
    let parent_type = ChannelType::try_from(ca.channel.r#type).unwrap_or(ChannelType::Text);
    if !matches!(
        parent_type,
        ChannelType::Text | ChannelType::Announcement | ChannelType::Forum
    ) {
        return Err(bad(
            "threads can be created in text, announcement and forum channels",
        ));
    }
    ca.require(P::CREATE_THREADS)?;
    let name = r.name.trim().chars().take(100).collect::<String>();
    if name.is_empty() {
        return Err(bad("name required"));
    }
    if parent_type == ChannelType::Forum && r.content.as_deref().is_none_or(|c| c.trim().is_empty())
    {
        return Err(bad("forum posts need content"));
    }
    let gid = ca.guild_id();
    let tid = st.ctx.id();
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "INSERT INTO channels (id, guild_id, type, name, parent_id, owner_id) VALUES ($1, $2, 6, $3, $4, $5)",
        tid as i64,
        gid as i64,
        name,
        cid as i64,
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO thread_members (thread_id, user_id) VALUES ($1, $2)",
        tid as i64,
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?;
    if let Some(mid) = r.message_id {
        let updated = sqlx::query!("UPDATE messages SET thread_id = $3 WHERE channel_id = $1 AND id = $2 AND thread_id IS NULL", cid as i64, mid as i64, tid as i64)
            .execute(&mut *tx)
            .await?;
        if updated.rows_affected() == 0 {
            return Err(ApiError::Conflict(
                "message not found or already has a thread",
            ));
        }
    }
    tx.commit().await?;
    let thread = repo::channel(&st.ctx.db, tid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(&thread, Intent::Guilds, Kind::ChannelCreate(thread.clone()))
        .await;
    if let Some(content) = r.content.filter(|c| !c.trim().is_empty()) {
        crate::routes::messages::create_internal(
            &st,
            a.id(),
            &thread,
            crate::routes::messages::NewMessage::text(content),
        )
        .await?;
    }
    if let Some(mid) = r.message_id
        && let Some(m) = repo::message(&st.ctx.db, cid, mid, 0).await?
    {
        st.publish_channel_event(&ca.channel, Intent::GuildMessages, Kind::MessageUpdate(m))
            .await;
    }
    Ok((StatusCode::CREATED, Json(thread)))
}

async fn archive_thread(
    State(st): State<AppState>,
    a: Auth,
    Path((_cid, tid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, tid, a.id()).await?;
    if ca.channel.r#type != ChannelType::Thread as i32 {
        return Err(bad("not a thread"));
    }
    if ca.channel.owner_id != a.id() {
        ca.require(P::MANAGE_THREADS)?;
    }
    sqlx::query!(
        "UPDATE channels SET archived = TRUE WHERE id = $1",
        tid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    let ch = repo::channel(&st.ctx.db, tid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(&ch, Intent::Guilds, Kind::ChannelUpdate(ch.clone()))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ForumTagBody {
    name: String,
    #[serde(default)]
    emoji: String,
}

async fn create_forum_tag(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(r): Json<ForumTagBody>,
) -> ApiResult<(StatusCode, Json<v1::ForumTag>)> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::MANAGE_CHANNELS)?;
    if ca.channel.r#type != ChannelType::Forum as i32 {
        return Err(bad("not a forum"));
    }
    let id = st.ctx.id();
    sqlx::query!(
        "INSERT INTO forum_tags (id, channel_id, name, emoji) VALUES ($1, $2, $3, $4)",
        id as i64,
        cid as i64,
        r.name.trim(),
        r.emoji
    )
    .execute(&st.ctx.db)
    .await?;
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(&ch, Intent::Guilds, Kind::ChannelUpdate(ch.clone()))
        .await;
    Ok((
        StatusCode::CREATED,
        Json(v1::ForumTag {
            id,
            name: r.name.trim().into(),
            emoji: r.emoji,
        }),
    ))
}

async fn delete_forum_tag(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, tag)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::MANAGE_CHANNELS)?;
    sqlx::query!(
        "DELETE FROM forum_tags WHERE id = $1 AND channel_id = $2",
        tag as i64,
        cid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Follow {
    #[serde(deserialize_with = "extract::id")]
    webhook_channel_id: u64,
}

/// Follow an announcement channel: creates a webhook in the target channel
/// that receives crossposted announcements.
async fn follow_announcement(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(r): Json<Follow>,
) -> ApiResult<Json<serde_json::Value>> {
    let src = channel_access(&st, cid, a.id()).await?;
    if src.channel.r#type != ChannelType::Announcement as i32 {
        return Err(bad("not an announcement channel"));
    }
    let dst = channel_access(&st, r.webhook_channel_id, a.id()).await?;
    dst.require(P::MANAGE_WEBHOOKS)?;
    let (id, token) = crate::routes::webhooks::create_webhook_row(
        &st,
        &dst.channel,
        a.id(),
        &format!("📣 {}", src.channel.name),
    )
    .await?;
    sqlx::query!("INSERT INTO audit_log (id, guild_id, user_id, action, target_id, changes) VALUES ($1, $2, $3, 'channel.follow', $4, $5)",
        st.ctx.id() as i64, dst.guild_id() as i64, a.id() as i64, cid as i64, json!({ "webhook_id": id.to_string(), "source": cid.to_string() }))
        .execute(&st.ctx.db)
        .await?;
    // Crossposting (POST /channels/{cid}/messages/{mid}/crosspost) fans out to these webhooks.
    sqlx::query!(
        "INSERT INTO channel_follows (source_channel_id, webhook_id) VALUES ($1, $2)",
        cid as i64,
        id as i64
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(Json(
        json!({ "webhook_id": id.to_string(), "token": token }),
    ))
}

// ------------------------------------------------------------------ invites

#[derive(Deserialize, Default)]
struct CreateInvite {
    /// 0 = unlimited
    #[serde(default)]
    max_uses: u32,
    /// seconds; 0 = never. Default 7 days.
    max_age: Option<u64>,
}

fn invite_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..8)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

async fn invite_proto(st: &AppState, code: &str) -> ApiResult<v1::Invite> {
    let r = sqlx::query!(
        "SELECT i.code, i.guild_id, i.channel_id, i.inviter_id, i.max_uses, i.uses, i.expires_at, g.name
         FROM invites i JOIN guilds g ON g.id = i.guild_id WHERE i.code = $1",
        code
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(v1::Invite {
        code: r.code,
        guild_id: r.guild_id as u64,
        channel_id: r.channel_id as u64,
        inviter_id: r.inviter_id.unwrap_or(0) as u64,
        max_uses: r.max_uses as u32,
        uses: r.uses as u32,
        expires_at: repo::ms(r.expires_at),
        guild_name: r.name,
    })
}

async fn create_invite(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    body: Option<Json<CreateInvite>>,
) -> ApiResult<(StatusCode, Json<v1::Invite>)> {
    let r = body.map(|b| b.0).unwrap_or_default();
    let ca = channel_access(&st, cid, a.id()).await?;
    if ca.is_dm() {
        return Err(bad("cannot invite to a DM"));
    }
    ca.require(P::CREATE_INVITE)?;
    let max_age = r.max_age.unwrap_or(7 * 24 * 3600).min(30 * 24 * 3600);
    let expires =
        (max_age > 0).then(|| chrono::Utc::now() + chrono::Duration::seconds(max_age as i64));
    let code = invite_code();
    sqlx::query!(
        "INSERT INTO invites (code, guild_id, channel_id, inviter_id, max_uses, expires_at) VALUES ($1, $2, $3, $4, $5, $6)",
        code,
        ca.guild_id() as i64,
        cid as i64,
        a.id() as i64,
        r.max_uses.min(1000) as i32,
        expires
    )
    .execute(&st.ctx.db)
    .await?;
    audit(
        &st,
        ca.guild_id(),
        a.id(),
        "invite.create",
        cid,
        json!({ "code": code }),
        "",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(invite_proto(&st, &code).await?)))
}

async fn list_invites(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
) -> ApiResult<Json<Vec<v1::Invite>>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::MANAGE_CHANNELS)?;
    let codes = sqlx::query_scalar!(
        "SELECT code FROM invites WHERE channel_id = $1 ORDER BY created_at DESC",
        cid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    let mut out = Vec::with_capacity(codes.len());
    for c in codes {
        out.push(invite_proto(&st, &c).await?);
    }
    Ok(Json(out))
}

/// Public invite preview (no auth): guild name and counts only.
async fn get_invite(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(code): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    st.limiter
        .check(&format!("inv-preview:{ip}"), rates::INVITE_PREVIEW)?;
    if code.len() > 16 {
        return Err(ApiError::NotFound);
    }
    let inv = invite_proto(&st, &code).await?;
    if inv.expires_at != 0 && inv.expires_at < repo::now_ms() {
        return Err(ApiError::NotFound);
    }
    let members = repo::member_count(&st.ctx.db, inv.guild_id).await?;
    Ok(Json(
        json!({ "code": inv.code, "guild": { "id": inv.guild_id.to_string(), "name": inv.guild_name }, "member_count": members }),
    ))
}

async fn accept_invite(
    State(st): State<AppState>,
    a: Auth,
    ClientIp(ip): ClientIp,
    Path(code): Path<String>,
) -> ApiResult<Json<v1::Guild>> {
    st.limiter
        .check(&format!("inv:{}:{ip}", a.id()), rates::INVITE_ACCEPT)?;
    let joined = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM members WHERE user_id = $1"#,
        a.id() as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if joined >= 200 {
        return Err(bad("guild limit reached (200)"));
    }
    let mut tx = st.ctx.db.begin().await?;
    // Atomic use: expiry and max_uses are enforced by the WHERE clause.
    let inv = sqlx::query!(
        "UPDATE invites SET uses = uses + 1
         WHERE code = $1 AND (expires_at IS NULL OR expires_at > now()) AND (max_uses = 0 OR uses < max_uses)
         RETURNING guild_id",
        code
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    let gid = inv.guild_id as u64;
    let banned = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM bans WHERE guild_id = $1 AND user_id = $2) AS "b!""#,
        gid as i64,
        a.id() as i64
    )
    .fetch_one(&mut *tx)
    .await?;
    if banned {
        return Err(ApiError::Forbidden("you are banned from this guild"));
    }
    let inserted = sqlx::query!(
        "INSERT INTO members (guild_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        gid as i64,
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        // Already a member: do not consume the invite.
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok(Json(announce_join(&st, gid, a.id(), inserted > 0).await?))
}

/// Joined (or already a member): tell the guild and the user's sessions, return the guild.
async fn announce_join(
    st: &AppState,
    gid: u64,
    uid: u64,
    new_member: bool,
) -> ApiResult<v1::Guild> {
    if new_member {
        let m = repo::member(&st.ctx.db, gid, uid)
            .await?
            .ok_or(ApiError::NotFound)?;
        st.publish_guild(gid, Intent::GuildMembers, Kind::MemberAdd(m));
    }
    let guild = repo::guild_full(&st.ctx.db, gid, uid)
        .await?
        .ok_or(ApiError::NotFound)?;
    if new_member {
        st.publish_user(uid, Intent::Guilds, Kind::GuildCreate(guild.clone()));
    }
    Ok(guild)
}

/// Join without an invite (public guilds): the same limits and bans as invites.
pub(crate) async fn add_member(st: &AppState, gid: u64, uid: u64) -> ApiResult<v1::Guild> {
    let joined = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM members WHERE user_id = $1"#,
        uid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if joined >= 200 {
        return Err(bad("guild limit reached (200)"));
    }
    let banned = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM bans WHERE guild_id = $1 AND user_id = $2) AS "b!""#,
        gid as i64,
        uid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if banned {
        return Err(ApiError::Forbidden("you are banned from this guild"));
    }
    let inserted = sqlx::query!(
        "INSERT INTO members (guild_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        gid as i64,
        uid as i64
    )
    .execute(&st.ctx.db)
    .await?
    .rows_affected();
    announce_join(st, gid, uid, inserted > 0).await
}

async fn delete_invite(
    State(st): State<AppState>,
    a: Auth,
    Path(code): Path<String>,
) -> ApiResult<StatusCode> {
    let inv = invite_proto(&st, &code).await?;
    let ga = guild_access(&st, inv.guild_id, a.id()).await?;
    if inv.inviter_id != a.id() {
        ga.require(P::MANAGE_GUILD)?;
    }
    sqlx::query!("DELETE FROM invites WHERE code = $1", code)
        .execute(&st.ctx.db)
        .await?;
    audit(
        &st,
        inv.guild_id,
        a.id(),
        "invite.delete",
        inv.channel_id,
        json!({ "code": code }),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        let c = invite_code();
        assert_eq!(c.len(), 8);
        assert!(c.chars().all(|ch| ch.is_ascii_alphanumeric()));
        assert_ne!(invite_code(), invite_code());
    }

    #[test]
    fn slug_names() {
        assert_eq!(
            clean_name("  Общий  Чат ", ChannelType::Text).unwrap(),
            "общий-чат"
        );
        assert_eq!(
            clean_name("Голос Тест", ChannelType::Voice).unwrap(),
            "Голос Тест"
        );
        assert!(clean_name("  ", ChannelType::Text).is_err());
    }
}
