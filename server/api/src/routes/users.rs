//! Profiles, DMs and group DMs, guild emojis and stickers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use neoshard_proto::permissions::Permissions as P;
use neoshard_proto::v1::{self, ChannelType, Intent, event::Kind};
use neoshard_server_common::repo;
use serde::Deserialize;

use crate::AppState;
use crate::authz::{audit, channel_access, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::{self, Auth};
use crate::routes::uploads;
use neoshard_proto::profile;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users/@me", get(me).patch(update_me))
        .route("/users/{uid}", get(user))
        .route("/users/@me/channels", get(my_channels).post(open_dm))
        .route("/users/@me/read-states", get(read_states))
        .route(
            "/channels/{cid}/recipients/{uid}",
            put(add_recipient).delete(remove_recipient),
        )
        .route("/guilds/{gid}/emojis", get(list_emojis).post(create_emoji))
        .route("/guilds/{gid}/emojis/{eid}", delete(delete_emoji))
        .route(
            "/guilds/{gid}/stickers",
            get(list_stickers).post(create_sticker),
        )
        .route("/guilds/{gid}/stickers/{sid}", delete(delete_sticker))
}

async fn me(State(st): State<AppState>, a: Auth) -> ApiResult<Json<v1::User>> {
    Ok(Json(crate::routes::auth::load_me(&st, a.id()).await?))
}

#[derive(Deserialize)]
struct NameStyleIn {
    #[serde(default)]
    font: String,
    #[serde(default)]
    effect: String,
    #[serde(default)]
    colors: Vec<u32>,
}

/// Every field is optional: absent = unchanged. Images are object keys from
/// `POST /uploads/{avatar|banner}` ("" resets). `banner_color: -1` clears it,
/// `theme_colors: []` resets the theme, `primary_guild_id: "0"` hides the tag.
#[derive(Deserialize)]
struct UpdateMe {
    display_name: Option<String>,
    avatar: Option<String>,
    banner: Option<String>,
    pronouns: Option<String>,
    bio: Option<String>,
    banner_color: Option<i64>,
    theme_colors: Option<Vec<u32>>,
    avatar_decoration: Option<String>,
    profile_effect: Option<String>,
    nameplate: Option<String>,
    name_style: Option<NameStyleIn>,
    #[serde(default, deserialize_with = "extract::opt_id")]
    primary_guild_id: Option<u64>,
}

fn check_colors(colors: &[u32]) -> ApiResult<()> {
    if !(colors.is_empty() || colors.len() == 2) || !colors.iter().all(|&c| profile::valid_color(c))
    {
        return Err(bad("theme colors: exactly two 0xRRGGBB values, or none"));
    }
    Ok(())
}

async fn update_me(
    State(st): State<AppState>,
    a: Auth,
    Json(r): Json<UpdateMe>,
) -> ApiResult<Json<v1::User>> {
    let uid = a.id();
    let too_long =
        |v: &Option<String>, max: usize| v.as_ref().is_some_and(|s| s.chars().count() > max);
    if too_long(&r.display_name, profile::DISPLAY_NAME_MAX)
        || too_long(&r.pronouns, profile::PRONOUNS_MAX)
        || too_long(&r.bio, profile::BIO_MAX)
    {
        return Err(bad(format!(
            "display_name ≤ {}, pronouns ≤ {}, bio ≤ {} characters",
            profile::DISPLAY_NAME_MAX,
            profile::PRONOUNS_MAX,
            profile::BIO_MAX
        )));
    }
    if r.avatar
        .as_deref()
        .is_some_and(|k| !uploads::user_key_ok(k, "avatars", uid))
    {
        return Err(bad("avatar must be a key from /uploads/avatar"));
    }
    if r.banner
        .as_deref()
        .is_some_and(|k| !uploads::user_key_ok(k, "banners", uid))
    {
        return Err(bad("banner must be a key from /uploads/banner"));
    }
    if r.banner_color
        .is_some_and(|c| !(-1..=0xFF_FFFF).contains(&c))
    {
        return Err(bad("banner_color is 0xRRGGBB or -1"));
    }
    if let Some(c) = &r.theme_colors {
        check_colors(c)?;
    }
    let catalog_ok =
        |v: &Option<String>, list: &[&str]| v.as_deref().is_none_or(|id| profile::valid(list, id));
    if !catalog_ok(&r.avatar_decoration, profile::AVATAR_DECORATIONS)
        || !catalog_ok(&r.profile_effect, profile::PROFILE_EFFECTS)
        || !catalog_ok(&r.nameplate, profile::NAMEPLATES)
    {
        return Err(bad("unknown cosmetic id (see spec/profile-catalog.md)"));
    }
    if let Some(ns) = &r.name_style
        && (!profile::valid(profile::NAME_FONTS, &ns.font)
            || !profile::valid(profile::NAME_EFFECTS, &ns.effect)
            || ns.colors.len() > 2
            || !ns.colors.iter().all(|&c| profile::valid_color(c)))
    {
        return Err(bad("invalid name_style"));
    }
    if let Some(gid) = r.primary_guild_id.filter(|g| *g != 0) {
        let ok = sqlx::query_scalar!(
            "SELECT g.tag <> '' FROM guilds g JOIN members m ON m.guild_id = g.id AND m.user_id = $2 WHERE g.id = $1",
            gid as i64,
            uid as i64
        )
        .fetch_optional(&st.ctx.db)
        .await?
        .flatten()
        .unwrap_or(false);
        if !ok {
            return Err(bad(
                "you can only show the tag of a server you are in and that has a tag",
            ));
        }
    }
    let (font, effect, ncolors) = match &r.name_style {
        Some(ns) => (
            Some(ns.font.clone()),
            Some(ns.effect.clone()),
            Some(ns.colors.iter().map(|&c| c as i32).collect::<Vec<_>>()),
        ),
        None => (None, None, None),
    };
    sqlx::query!(
        r#"UPDATE users SET
              display_name = COALESCE($2, display_name),
              avatar = COALESCE($3, avatar),
              banner = COALESCE($4, banner),
              pronouns = COALESCE($5, pronouns),
              bio = COALESCE($6, bio),
              banner_color = CASE WHEN $7::int IS NULL THEN banner_color WHEN $7 < 0 THEN NULL ELSE $7 END,
              theme_colors = COALESCE($8, theme_colors),
              avatar_decoration = COALESCE($9, avatar_decoration),
              profile_effect = COALESCE($10, profile_effect),
              nameplate = COALESCE($11, nameplate),
              name_font = COALESCE($12, name_font),
              name_effect = COALESCE($13, name_effect),
              name_colors = COALESCE($14, name_colors),
              primary_guild_id = CASE WHEN $15::bigint IS NULL THEN primary_guild_id WHEN $15 = 0 THEN NULL ELSE $15 END,
              updated_at = now()
           WHERE id = $1"#,
        uid as i64,
        r.display_name.as_deref().map(str::trim),
        r.avatar,
        r.banner,
        r.pronouns.as_deref().map(str::trim),
        r.bio.as_deref().map(str::trim),
        r.banner_color.map(|c| c as i32),
        r.theme_colors.as_ref().map(|c| c.iter().map(|&x| x as i32).collect::<Vec<_>>()) as Option<Vec<i32>>,
        r.avatar_decoration,
        r.profile_effect,
        r.nameplate,
        font,
        effect,
        ncolors as Option<Vec<i32>>,
        r.primary_guild_id.map(|g| g as i64)
    )
    .execute(&st.ctx.db)
    .await?;
    let u = crate::routes::auth::load_me(&st, uid).await?;
    publish_user(&st, &u).await?;
    Ok(Json(u))
}

/// Tell everyone who can see the user (guilds, DMs, the user's own sessions).
pub(crate) async fn publish_user(st: &AppState, u: &v1::User) -> ApiResult<()> {
    let mut public = u.clone();
    public.mfa_enabled = false;
    for gid in repo::guild_ids_for_user(&st.ctx.db, u.id).await? {
        st.publish_guild(gid, Intent::GuildMembers, Kind::UserUpdate(public.clone()));
    }
    let peers = sqlx::query_scalar!(
        "SELECT DISTINCT b.user_id FROM channel_recipients a JOIN channel_recipients b ON b.channel_id = a.channel_id AND b.user_id <> a.user_id WHERE a.user_id = $1",
        u.id as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    for p in peers {
        st.publish_user(p as u64, Intent::None, Kind::UserUpdate(public.clone()));
    }
    st.publish_user(u.id, Intent::None, Kind::UserUpdate(u.clone()));
    Ok(())
}

async fn user(
    State(st): State<AppState>,
    _a: Auth,
    Path(uid): Path<u64>,
) -> ApiResult<Json<v1::User>> {
    Ok(Json(
        repo::user(&st.ctx.db, uid)
            .await?
            .ok_or(ApiError::NotFound)?
            .public(),
    ))
}

async fn my_channels(State(st): State<AppState>, a: Auth) -> ApiResult<Json<Vec<v1::Channel>>> {
    Ok(Json(repo::private_channels(&st.ctx.db, a.id()).await?))
}

async fn read_states(State(st): State<AppState>, a: Auth) -> ApiResult<Json<Vec<v1::ReadState>>> {
    Ok(Json(repo::read_states(&st.ctx.db, a.id()).await?))
}

async fn shares_guild(st: &AppState, a: u64, b: u64) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM members m1 JOIN members m2 ON m1.guild_id = m2.guild_id WHERE m1.user_id = $1 AND m2.user_id = $2) AS "e!""#,
        a as i64,
        b as i64
    )
    .fetch_one(&st.ctx.db)
    .await?)
}

#[derive(Deserialize)]
struct OpenDm {
    #[serde(default, deserialize_with = "extract::opt_id")]
    recipient_id: Option<u64>,
    #[serde(default, deserialize_with = "extract::ids")]
    recipient_ids: Vec<u64>,
    #[serde(default)]
    name: String,
    /// MLS end-to-end encryption (ADR-0013).
    #[serde(default)]
    e2ee: bool,
}

/// Open (or return the existing) 1:1 DM, or create a group DM (≤ 10 people).
async fn open_dm(
    State(st): State<AppState>,
    a: Auth,
    Json(r): Json<OpenDm>,
) -> ApiResult<Json<v1::Channel>> {
    let me = a.id();
    let mut others: Vec<u64> = r
        .recipient_id
        .into_iter()
        .chain(r.recipient_ids)
        .filter(|&u| u != me)
        .collect();
    others.sort_unstable();
    others.dedup();
    if others.is_empty() {
        return Err(bad("recipient_id or recipient_ids required"));
    }
    if others.len() > 9 {
        return Err(bad("group DMs have at most 10 members"));
    }
    for &u in &others {
        let remote = sqlx::query_scalar!(
            "SELECT instance <> '' FROM users WHERE id = $1 AND deleted_at IS NULL",
            u as i64
        )
        .fetch_optional(&st.ctx.db)
        .await?
        .flatten()
        .ok_or(ApiError::NotFound)?;
        if repo::blocked_between(&st.ctx.db, me, u).await? {
            return Err(ApiError::Forbidden("you cannot message this user"));
        }
        // Anti-spam: local users must be friends or share a guild; remote (federated) users are allowed.
        let friends = repo::relationship_kind(&st.ctx.db, me, u).await?
            == v1::RelationshipKind::Friend as i32;
        if !remote && !friends && !shares_guild(&st, me, u).await? {
            return Err(ApiError::Forbidden(
                "you can only message friends or people you share a guild with",
            ));
        }
    }
    if others.len() == 1 {
        let (lo, hi) = if me < others[0] {
            (me, others[0])
        } else {
            (others[0], me)
        };
        if let Some(cid) = sqlx::query_scalar!(
            "SELECT channel_id FROM dm_pairs WHERE user_a = $1 AND user_b = $2 AND e2ee = $3",
            lo as i64,
            hi as i64,
            r.e2ee
        )
        .fetch_optional(&st.ctx.db)
        .await?
        {
            // Re-add self if the DM was closed earlier.
            sqlx::query!("INSERT INTO channel_recipients (channel_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", cid, me as i64).execute(&st.ctx.db).await?;
            return Ok(Json(
                repo::channel(&st.ctx.db, cid as u64)
                    .await?
                    .ok_or(ApiError::NotFound)?,
            ));
        }
    }
    let cid = st.ctx.id();
    let ty = if others.len() == 1 {
        ChannelType::Dm
    } else {
        ChannelType::GroupDm
    };
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "INSERT INTO channels (id, type, name, owner_id, e2ee) VALUES ($1, $2, $3, $4, $5)",
        cid as i64,
        ty as i16,
        r.name.trim().chars().take(100).collect::<String>(),
        me as i64,
        r.e2ee
    )
    .execute(&mut *tx)
    .await?;
    for u in others.iter().chain(std::iter::once(&me)) {
        sqlx::query!(
            "INSERT INTO channel_recipients (channel_id, user_id) VALUES ($1, $2)",
            cid as i64,
            *u as i64
        )
        .execute(&mut *tx)
        .await?;
    }
    if ty == ChannelType::Dm {
        let (lo, hi) = if me < others[0] {
            (me, others[0])
        } else {
            (others[0], me)
        };
        let inserted = sqlx::query!("INSERT INTO dm_pairs (user_a, user_b, channel_id, e2ee) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING", lo as i64, hi as i64, cid as i64, r.e2ee)
            .execute(&mut *tx)
            .await?;
        if inserted.rows_affected() == 0 {
            // Lost a race with a concurrent open: use the winner's channel.
            tx.rollback().await?;
            let cid = sqlx::query_scalar!(
                "SELECT channel_id FROM dm_pairs WHERE user_a = $1 AND user_b = $2 AND e2ee = $3",
                lo as i64,
                hi as i64,
                r.e2ee
            )
            .fetch_one(&st.ctx.db)
            .await?;
            return Ok(Json(
                repo::channel(&st.ctx.db, cid as u64)
                    .await?
                    .ok_or(ApiError::NotFound)?,
            ));
        }
    }
    tx.commit().await?;
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(&ch, Intent::DirectMessages, Kind::ChannelCreate(ch.clone()))
        .await;
    Ok(Json(ch))
}

async fn add_recipient(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, uid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if ca.channel.r#type != ChannelType::GroupDm as i32 {
        return Err(bad("not a group DM"));
    }
    if ca.channel.recipient_ids.len() >= 10 {
        return Err(bad("group DMs have at most 10 members"));
    }
    if !shares_guild(&st, a.id(), uid).await? {
        return Err(ApiError::Forbidden(
            "you can only add people you share a guild with",
        ));
    }
    sqlx::query!("INSERT INTO channel_recipients (channel_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", cid as i64, uid as i64).execute(&st.ctx.db).await?;
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(&ch, Intent::DirectMessages, Kind::ChannelUpdate(ch.clone()))
        .await;
    st.publish_user(uid, Intent::DirectMessages, Kind::ChannelCreate(ch));
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_recipient(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, uid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if ca.channel.r#type != ChannelType::GroupDm as i32 {
        return Err(bad("not a group DM"));
    }
    if uid != a.id() && ca.channel.owner_id != a.id() {
        return Err(ApiError::Forbidden("only the owner can remove others"));
    }
    sqlx::query!(
        "DELETE FROM channel_recipients WHERE channel_id = $1 AND user_id = $2",
        cid as i64,
        uid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    let ch = repo::channel(&st.ctx.db, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(&ch, Intent::DirectMessages, Kind::ChannelUpdate(ch.clone()))
        .await;
    st.publish_user(
        uid,
        Intent::DirectMessages,
        Kind::ChannelDelete(v1::ChannelDelete {
            id: cid,
            guild_id: 0,
        }),
    );
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ emojis & stickers

async fn publish_guild_update(st: &AppState, gid: u64, viewer: u64) -> ApiResult<()> {
    let mut g = repo::guild_full(&st.ctx.db, gid, viewer)
        .await?
        .ok_or(ApiError::NotFound)?;
    g.channels.clear();
    g.roles.clear();
    g.self_member = None;
    st.publish_guild(gid, Intent::Guilds, Kind::GuildUpdate(g));
    Ok(())
}

async fn list_emojis(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<Vec<v1::Emoji>>> {
    guild_access(&st, gid, a.id()).await?;
    Ok(Json(
        repo::guild_full(&st.ctx.db, gid, a.id())
            .await?
            .ok_or(ApiError::NotFound)?
            .emojis,
    ))
}

#[derive(Deserialize)]
struct AssetBody {
    name: String,
    key: String,
    #[serde(default)]
    animated: bool,
    #[serde(default = "default_format")]
    format: String,
}

fn default_format() -> String {
    "png".into()
}

async fn create_emoji(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(b): Json<AssetBody>,
) -> ApiResult<(StatusCode, Json<v1::Emoji>)> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_EMOJIS)?;
    if !b.key.starts_with(&format!("emojis/{gid}/")) {
        return Err(bad("key must come from /guilds/{gid}/uploads/emoji"));
    }
    let n = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM emojis WHERE guild_id = $1"#,
        gid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if n >= 200 {
        return Err(bad("emoji limit reached (200)"));
    }
    let id = st.ctx.id();
    sqlx::query!(
        "INSERT INTO emojis (id, guild_id, name, animated, object_key, created_by) VALUES ($1, $2, $3, $4, $5, $6)",
        id as i64,
        gid as i64,
        b.name,
        b.animated,
        b.key,
        a.id() as i64
    )
    .execute(&st.ctx.db)
    .await?;
    audit(
        &st,
        gid,
        a.id(),
        "emoji.create",
        id,
        serde_json::json!({ "name": b.name }),
        "",
    )
    .await?;
    publish_guild_update(&st, gid, a.id()).await?;
    Ok((
        StatusCode::CREATED,
        Json(v1::Emoji {
            id,
            guild_id: gid,
            name: b.name,
            animated: b.animated,
            key: b.key,
        }),
    ))
}

async fn delete_emoji(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, eid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_EMOJIS)?;
    sqlx::query!(
        "DELETE FROM emojis WHERE id = $1 AND guild_id = $2",
        eid as i64,
        gid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    publish_guild_update(&st, gid, a.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_stickers(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<Vec<v1::Sticker>>> {
    guild_access(&st, gid, a.id()).await?;
    Ok(Json(
        repo::guild_full(&st.ctx.db, gid, a.id())
            .await?
            .ok_or(ApiError::NotFound)?
            .stickers,
    ))
}

async fn create_sticker(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(b): Json<AssetBody>,
) -> ApiResult<(StatusCode, Json<v1::Sticker>)> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_EMOJIS)?;
    if !b.key.starts_with(&format!("stickers/{gid}/")) {
        return Err(bad("key must come from /guilds/{gid}/uploads/sticker"));
    }
    let id = st.ctx.id();
    sqlx::query!(
        "INSERT INTO stickers (id, guild_id, name, format, object_key) VALUES ($1, $2, $3, $4, $5)",
        id as i64,
        gid as i64,
        b.name,
        b.format,
        b.key
    )
    .execute(&st.ctx.db)
    .await?;
    publish_guild_update(&st, gid, a.id()).await?;
    Ok((
        StatusCode::CREATED,
        Json(v1::Sticker {
            id,
            guild_id: gid,
            name: b.name,
            format: b.format,
            key: b.key,
        }),
    ))
}

async fn delete_sticker(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, sid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_EMOJIS)?;
    sqlx::query!(
        "DELETE FROM stickers WHERE id = $1 AND guild_id = $2",
        sid as i64,
        gid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    publish_guild_update(&st, gid, a.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}
