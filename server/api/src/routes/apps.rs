//! Bot applications, slash commands and interactions.
//!
//! Flow: a user runs `/command` → `POST /interactions` → the bot receives
//! `interaction_create` on its Gateway session → the bot answers with
//! `POST /interactions/{id}/{token}/callback` within 15 minutes.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use neoshard_proto::permissions::Permissions as P;
use neoshard_proto::v1::{self, Intent, MessageKind, event::Kind};
use neoshard_server_common::{auth, repo};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::authz::{audit, channel_access, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::{self, Auth};
use crate::routes::messages::{NewMessage, create_internal};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/applications", get(list_apps).post(create_app))
        .route("/applications/{aid}", delete(delete_app))
        .route("/applications/{aid}/reset-token", post(reset_token))
        .route(
            "/applications/{aid}/commands",
            get(list_global).put(overwrite_global),
        )
        .route(
            "/applications/{aid}/guilds/{gid}/commands",
            put(overwrite_guild),
        )
        .route("/guilds/{gid}/bots", post(add_bot))
        .route("/guilds/{gid}/commands", get(guild_commands))
        .route("/interactions", post(invoke))
        .route("/interactions/{iid}/{token}/callback", post(callback))
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(20)
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.len() < 2 { "bot".into() } else { s }
}

async fn owned_app(st: &AppState, aid: u64, owner: u64) -> ApiResult<(i64, String)> {
    let r = sqlx::query!(
        "SELECT bot_user_id, name FROM applications WHERE id = $1 AND owner_id = $2",
        aid as i64,
        owner as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok((r.bot_user_id, r.name))
}

#[derive(Deserialize)]
struct CreateApp {
    name: String,
    #[serde(default)]
    description: String,
}

async fn bot_token(st: &AppState, bot_user: i64) -> ApiResult<String> {
    let (token, hash) = auth::new_token("nsb_");
    sqlx::query!("DELETE FROM sessions WHERE user_id = $1", bot_user)
        .execute(&st.ctx.db)
        .await?;
    st.revoke_sessions(bot_user as u64, vec![], true, 0);
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, token_hash, device) VALUES ($1, $2, $3, 'bot')",
        st.ctx.id() as i64,
        bot_user,
        hash
    )
    .execute(&st.ctx.db)
    .await?;
    st.ctx.sessions.evict_user(bot_user as u64);
    Ok(token)
}

async fn create_app(
    State(st): State<AppState>,
    a: Auth,
    Json(b): Json<CreateApp>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    if a.0.bot {
        return Err(ApiError::Forbidden("bots cannot create applications"));
    }
    let name = b.name.trim();
    if !(2..=32).contains(&name.chars().count()) {
        return Err(bad("name must be 2-32 characters"));
    }
    let n = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM applications WHERE owner_id = $1"#,
        a.id() as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if n >= 25 {
        return Err(bad("application limit reached (25)"));
    }
    let (aid, bot) = (st.ctx.id(), st.ctx.id());
    let username = format!("{}_{}", slug(name), bot % 10_000);
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "INSERT INTO users (id, username, display_name, bot) VALUES ($1, $2, $3, TRUE)",
        bot as i64,
        username,
        name
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO applications (id, owner_id, bot_user_id, name, description) VALUES ($1, $2, $3, $4, $5)",
        aid as i64,
        a.id() as i64,
        bot as i64,
        name,
        b.description
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let token = bot_token(&st, bot as i64).await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": aid.to_string(), "bot_user_id": bot.to_string(), "bot_username": username, "token": token }),
        ),
    ))
}

async fn list_apps(State(st): State<AppState>, a: Auth) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query!("SELECT id, name, bot_user_id, description FROM applications WHERE owner_id = $1 ORDER BY id", a.id() as i64)
        .fetch_all(&st.ctx.db)
        .await?;
    Ok(Json(json!(rows.into_iter().map(|r| json!({
        "id": r.id.to_string(), "name": r.name, "bot_user_id": r.bot_user_id.to_string(), "description": r.description
    })).collect::<Vec<_>>())))
}

async fn reset_token(
    State(st): State<AppState>,
    a: Auth,
    Path(aid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    let (bot, _) = owned_app(&st, aid, a.id()).await?;
    Ok(Json(json!({ "token": bot_token(&st, bot).await? })))
}

async fn delete_app(
    State(st): State<AppState>,
    a: Auth,
    Path(aid): Path<u64>,
) -> ApiResult<StatusCode> {
    let (bot, _) = owned_app(&st, aid, a.id()).await?;
    sqlx::query!("DELETE FROM applications WHERE id = $1", aid as i64)
        .execute(&st.ctx.db)
        .await?;
    sqlx::query!("UPDATE users SET deleted_at = now() WHERE id = $1", bot)
        .execute(&st.ctx.db)
        .await?;
    sqlx::query!("DELETE FROM sessions WHERE user_id = $1", bot)
        .execute(&st.ctx.db)
        .await?;
    sqlx::query!("DELETE FROM members WHERE user_id = $1", bot)
        .execute(&st.ctx.db)
        .await?;
    st.revoke_sessions(bot as u64, vec![], true, 0);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AddBot {
    #[serde(deserialize_with = "extract::id")]
    application_id: u64,
    /// Permissions for the bot's managed role.
    #[serde(default, deserialize_with = "extract::bits")]
    permissions: u64,
}

async fn add_bot(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(b): Json<AddBot>,
) -> ApiResult<StatusCode> {
    let ga = guild_access(&st, gid, a.id()).await?;
    ga.require(P::MANAGE_GUILD)?;
    if !neoshard_proto::permissions::can_grant(ga.perms, P::raw(b.permissions)) {
        return Err(ApiError::Forbidden(
            "cannot grant permissions you do not have",
        ));
    }
    let app = sqlx::query!(
        "SELECT bot_user_id, name FROM applications WHERE id = $1",
        b.application_id as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let mut tx = st.ctx.db.begin().await?;
    let added = sqlx::query!(
        "INSERT INTO members (guild_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        gid as i64,
        app.bot_user_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added > 0 && b.permissions != 0 {
        let rid = st.ctx.id();
        sqlx::query!(
            "UPDATE roles SET position = position + 1 WHERE guild_id = $1 AND id <> $1",
            gid as i64
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO roles (id, guild_id, name, position, permissions) VALUES ($1, $2, $3, 1, $4)",
            rid as i64,
            gid as i64,
            app.name,
            b.permissions as i64
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO member_roles (guild_id, user_id, role_id) VALUES ($1, $2, $3)",
            gid as i64,
            app.bot_user_id,
            rid as i64
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    if added > 0 {
        audit(
            &st,
            gid,
            a.id(),
            "bot.add",
            app.bot_user_id as u64,
            json!({ "application_id": b.application_id.to_string() }),
            "",
        )
        .await?;
        for r in repo::roles(&st.ctx.db, gid).await? {
            st.publish_guild(gid, Intent::Guilds, Kind::RoleUpdate(r));
        }
        if let Some(m) = repo::member(&st.ctx.db, gid, app.bot_user_id as u64).await? {
            st.publish_guild(gid, Intent::GuildMembers, Kind::MemberAdd(m));
        }
        if let Some(g) = repo::guild_full(&st.ctx.db, gid, app.bot_user_id as u64).await? {
            st.publish_user(app.bot_user_id as u64, Intent::Guilds, Kind::GuildCreate(g));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ commands

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CommandDef {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    options: serde_json::Value,
}

fn valid_command(c: &CommandDef) -> bool {
    (1..=32).contains(&c.name.len())
        && c.name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && c.description.chars().count() <= 100
}

async fn overwrite(
    st: &AppState,
    aid: u64,
    gid: u64,
    cmds: Vec<CommandDef>,
) -> ApiResult<Json<serde_json::Value>> {
    if cmds.len() > 100 || !cmds.iter().all(valid_command) {
        return Err(bad("≤ 100 commands; names 1-32 of a-z 0-9 _ -"));
    }
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "DELETE FROM application_commands WHERE application_id = $1 AND guild_id = $2",
        aid as i64,
        gid as i64
    )
    .execute(&mut *tx)
    .await?;
    for c in &cmds {
        let options = if c.options.is_null() {
            json!([])
        } else {
            c.options.clone()
        };
        sqlx::query!(
            "INSERT INTO application_commands (id, application_id, guild_id, name, description, options) VALUES ($1, $2, $3, $4, $5, $6)",
            st.ctx.id() as i64,
            aid as i64,
            gid as i64,
            c.name,
            c.description,
            options
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!(cmds)))
}

/// Bot tokens and owners may manage commands.
async fn can_manage_app(st: &AppState, a: &Auth, aid: u64) -> ApiResult<()> {
    let ok = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM applications WHERE id = $1 AND (owner_id = $2 OR bot_user_id = $2)) AS "e!""#,
        aid as i64,
        a.id() as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if ok { Ok(()) } else { Err(ApiError::NotFound) }
}

async fn overwrite_global(
    State(st): State<AppState>,
    a: Auth,
    Path(aid): Path<u64>,
    Json(cmds): Json<Vec<CommandDef>>,
) -> ApiResult<Json<serde_json::Value>> {
    can_manage_app(&st, &a, aid).await?;
    overwrite(&st, aid, 0, cmds).await
}

async fn overwrite_guild(
    State(st): State<AppState>,
    a: Auth,
    Path((aid, gid)): Path<(u64, u64)>,
    Json(cmds): Json<Vec<CommandDef>>,
) -> ApiResult<Json<serde_json::Value>> {
    can_manage_app(&st, &a, aid).await?;
    overwrite(&st, aid, gid, cmds).await
}

async fn list_global(
    State(st): State<AppState>,
    a: Auth,
    Path(aid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    can_manage_app(&st, &a, aid).await?;
    let rows = sqlx::query!("SELECT name, description, options FROM application_commands WHERE application_id = $1 AND guild_id = 0 ORDER BY name", aid as i64)
        .fetch_all(&st.ctx.db)
        .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({ "name": r.name, "description": r.description, "options": r.options }))
            .collect::<Vec<_>>()
    )))
}

/// Commands available in a guild (global commands of bots in the guild + guild commands), for autocomplete.
async fn guild_commands(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    guild_access(&st, gid, a.id()).await?;
    let rows = sqlx::query!(
        "SELECT c.application_id, c.name, c.description, c.options, ap.name AS app_name
         FROM application_commands c
         JOIN applications ap ON ap.id = c.application_id
         JOIN members m ON m.user_id = ap.bot_user_id AND m.guild_id = $1
         WHERE c.guild_id IN (0, $1) ORDER BY c.name",
        gid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|r| json!({
                "application_id": r.application_id.to_string(), "application": r.app_name,
                "name": r.name, "description": r.description, "options": r.options
            }))
            .collect::<Vec<_>>()
    )))
}

// ------------------------------------------------------------------ interactions

#[derive(Deserialize)]
struct Invoke {
    #[serde(deserialize_with = "extract::id")]
    application_id: u64,
    #[serde(deserialize_with = "extract::id")]
    channel_id: u64,
    command: String,
    #[serde(default)]
    options: serde_json::Value,
}

async fn invoke(
    State(st): State<AppState>,
    a: Auth,
    Json(b): Json<Invoke>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let ca = channel_access(&st, b.channel_id, a.id()).await?;
    ca.require(P::USE_COMMANDS)?;
    let gid = ca.channel.guild_id;
    let app = sqlx::query!(
        "SELECT ap.bot_user_id FROM applications ap
         JOIN application_commands c ON c.application_id = ap.id AND c.name = $2 AND c.guild_id IN (0, $3)
         WHERE ap.id = $1 LIMIT 1",
        b.application_id as i64,
        b.command,
        gid as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if gid != 0
        && repo::member(&st.ctx.db, gid, app.bot_user_id as u64)
            .await?
            .is_none()
    {
        return Err(ApiError::NotFound);
    }
    let (token, hash) = auth::new_token("nsi_");
    let iid = st.ctx.id();
    sqlx::query!(
        "INSERT INTO interactions (id, application_id, token_hash, guild_id, channel_id, user_id, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + interval '15 minutes')",
        iid as i64,
        b.application_id as i64,
        hash,
        (gid != 0).then_some(gid as i64),
        b.channel_id as i64,
        a.id() as i64
    )
    .execute(&st.ctx.db)
    .await?;
    let user = repo::user(&st.ctx.db, a.id()).await?.map(|u| u.public());
    let ev = v1::InteractionCreate {
        id: iid,
        token,
        application_id: b.application_id,
        guild_id: gid,
        channel_id: b.channel_id,
        user,
        command: b.command,
        options_json: if b.options.is_null() {
            "{}".into()
        } else {
            b.options.to_string()
        },
    };
    st.publish_user(
        app.bot_user_id as u64,
        Intent::None,
        Kind::InteractionCreate(ev),
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "interaction_id": iid.to_string() })),
    ))
}

#[derive(Deserialize)]
struct Callback {
    content: String,
    /// Only the invoking user sees the reply; it is not stored.
    #[serde(default)]
    ephemeral: bool,
}

async fn callback(
    State(st): State<AppState>,
    Path((iid, token)): Path<(u64, String)>,
    Json(b): Json<Callback>,
) -> ApiResult<(StatusCode, Json<v1::Message>)> {
    let row = sqlx::query!(
        "UPDATE interactions SET responded = TRUE
         WHERE id = $1 AND token_hash = $2 AND NOT responded AND expires_at > now()
         RETURNING application_id, channel_id, user_id",
        iid as i64,
        auth::hash_token(&token)
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if b.content.trim().is_empty()
        || b.content.chars().count() > crate::routes::messages::MAX_CONTENT
    {
        return Err(bad("content must be 1-4000 characters"));
    }
    let bot = sqlx::query_scalar!(
        "SELECT bot_user_id FROM applications WHERE id = $1",
        row.application_id
    )
    .fetch_one(&st.ctx.db)
    .await?;
    let ch = repo::channel(&st.ctx.db, row.channel_id as u64)
        .await?
        .ok_or(ApiError::NotFound)?;
    if b.ephemeral {
        let author = repo::user(&st.ctx.db, bot as u64)
            .await?
            .map(|u| u.public());
        let msg = v1::Message {
            id: st.ctx.id(),
            channel_id: ch.id,
            guild_id: ch.guild_id,
            author,
            content: b.content,
            kind: MessageKind::SlashCommand as i32,
            ..Default::default()
        };
        st.publish_user(
            row.user_id as u64,
            Intent::None,
            Kind::MessageCreate(msg.clone()),
        );
        return Ok((StatusCode::OK, Json(msg)));
    }
    let nm = NewMessage {
        content: b.content,
        kind: MessageKind::SlashCommand,
        ..Default::default()
    };
    let msg = create_internal(&st, bot as u64, &ch, nm).await?;
    Ok((StatusCode::CREATED, Json(msg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Music Bot!"), "music_bot");
        assert_eq!(slug("Я"), "bot");
    }

    #[test]
    fn command_names() {
        let ok = CommandDef {
            name: "roll-dice".into(),
            description: "".into(),
            options: json!([]),
        };
        assert!(valid_command(&ok));
        let bad = CommandDef {
            name: "Roll".into(),
            description: "".into(),
            options: json!([]),
        };
        assert!(!valid_command(&bad));
    }
}
