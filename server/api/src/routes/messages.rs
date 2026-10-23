//! Messages, reactions, pins, read states, typing, search, crossposting.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::Engine;
use neoshard_proto::permissions::Permissions as P;
use neoshard_proto::v1::{self, ChannelType, Intent, MessageKind, event::Kind};
use neoshard_server_common::bus::BusEvent;
use neoshard_server_common::repo::{self, MessageRow};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::authz::{ChannelAccess, channel_access, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::{self, Auth};
use crate::ratelimit::rates;
use crate::routes::{automod, previews};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{cid}/messages", get(list).post(create))
        .route("/channels/{cid}/messages/bulk-delete", post(bulk_delete))
        .route("/channels/{cid}/messages/search", get(search_channel))
        .route(
            "/channels/{cid}/messages/{mid}",
            get(get_one).patch(edit).delete(remove),
        )
        .route("/channels/{cid}/messages/{mid}/ack", post(ack))
        .route("/channels/{cid}/messages/{mid}/crosspost", post(crosspost))
        .route(
            "/channels/{cid}/messages/{mid}/reactions/{emoji}",
            get(reaction_users).delete(remove_all_emoji),
        )
        .route(
            "/channels/{cid}/messages/{mid}/reactions/{emoji}/@me",
            put(react).delete(unreact),
        )
        .route(
            "/channels/{cid}/messages/{mid}/reactions/{emoji}/{uid}",
            delete(remove_user_reaction),
        )
        .route("/channels/{cid}/pins", get(pins))
        .route("/channels/{cid}/pins/{mid}", put(pin).delete(unpin))
        .route("/channels/{cid}/typing", post(typing))
        .route("/guilds/{gid}/messages/search", get(search_guild))
}

pub const MAX_CONTENT: usize = 4000;
const MAX_PINS: i64 = 50;

// ------------------------------------------------------------------ mentions

#[derive(Debug, Default, PartialEq)]
pub struct Mentions {
    pub users: Vec<u64>,
    pub roles: Vec<u64>,
    pub everyone: bool,
}

/// Parse `<@id>`, `<@!id>`, `<@&id>`, `@everyone`, `@here`. Code spans are ignored.
pub fn parse_mentions(content: &str) -> Mentions {
    let mut m = Mentions::default();
    let mut in_code = false;
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if in_code {
            i += 1;
            continue;
        }
        if c == b'<' && bytes.get(i + 1) == Some(&b'@') {
            let mut j = i + 2;
            let role = bytes.get(j) == Some(&b'&');
            if role || bytes.get(j) == Some(&b'!') {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && bytes.get(j) == Some(&b'>') {
                if let Ok(id) = content[start..j].parse::<u64>() {
                    let list = if role { &mut m.roles } else { &mut m.users };
                    if !list.contains(&id) && list.len() < 100 {
                        list.push(id);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        if c == b'@' && (content[i..].starts_with("@everyone") || content[i..].starts_with("@here"))
        {
            m.everyone = true;
        }
        i += 1;
    }
    m
}

// ------------------------------------------------------------------ create

#[derive(Debug, Clone, Default)]
pub struct NewMessage {
    pub content: String,
    pub nonce: Option<String>,
    pub reply_to_id: Option<u64>,
    pub attachment_ids: Vec<u64>,
    pub sticker_id: Option<u64>,
    pub e2ee_payload: Option<Vec<u8>>,
    pub kind: MessageKind,
    pub webhook_id: Option<u64>,
}

impl NewMessage {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Default::default()
        }
    }
}

#[derive(Deserialize)]
struct CreateBody {
    #[serde(default)]
    content: String,
    nonce: Option<String>,
    #[serde(default, deserialize_with = "extract::opt_id")]
    reply_to_id: Option<u64>,
    #[serde(default, deserialize_with = "extract::ids")]
    attachment_ids: Vec<u64>,
    #[serde(default, deserialize_with = "extract::opt_id")]
    sticker_id: Option<u64>,
    /// base64 MLS ciphertext for E2EE channels.
    e2ee_payload: Option<String>,
}

async fn create(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(b): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<v1::Message>)> {
    st.limiter
        .check(&format!("msg:{}:{cid}", a.id()), rates::MESSAGE_CREATE)?;
    st.limiter
        .check(&format!("msg-all:{}", a.id()), rates::MESSAGE_CREATE_ALL)?;
    let e2ee_payload = b
        .e2ee_payload
        .map(|p| base64::engine::general_purpose::STANDARD.decode(p))
        .transpose()
        .map_err(|_| bad("e2ee_payload must be base64"))?;
    let nm = NewMessage {
        content: b.content,
        nonce: b.nonce,
        reply_to_id: b.reply_to_id,
        attachment_ids: b.attachment_ids,
        sticker_id: b.sticker_id,
        e2ee_payload,
        kind: if b.reply_to_id.is_some() {
            MessageKind::Reply
        } else {
            MessageKind::Default
        },
        webhook_id: None,
    };
    let ca = channel_access(&st, cid, a.id()).await?;
    let (msg, created) = send(&st, a.id(), &ca, nm).await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(msg),
    ))
}

/// Create a message as `author` without a permission check (system/webhook/bot paths
/// that have already been authorized).
pub async fn create_internal(
    st: &AppState,
    author: u64,
    channel: &v1::Channel,
    mut nm: NewMessage,
) -> ApiResult<v1::Message> {
    // Internal paths (webhooks, crossposts, bot replies, federation) never mass-ping.
    nm.content = neutralize_everyone(&nm.content);
    let ca = ChannelAccess {
        channel: channel.clone(),
        perms: P::all() - P::MENTION_EVERYONE - P::ADMINISTRATOR,
        guild: None,
    };
    Ok(send(st, author, &ca, nm).await?.0)
}

pub fn neutralize_everyone(s: &str) -> String {
    s.replace("@everyone", "@\u{200B}everyone")
        .replace("@here", "@\u{200B}here")
}

/// The one message-creation path. Returns `(message, created)`; `created` is false
/// when an identical `nonce` was already used (idempotent retry).
async fn send(
    st: &AppState,
    author: u64,
    ca: &ChannelAccess,
    mut nm: NewMessage,
) -> ApiResult<(v1::Message, bool)> {
    let ch = &ca.channel;
    let cid = ch.id;
    let ct = ChannelType::try_from(ch.r#type).unwrap_or(ChannelType::Text);
    match ct {
        ChannelType::Voice | ChannelType::Category => {
            return Err(bad("cannot send messages to this channel type"));
        }
        ChannelType::Forum if nm.kind != MessageKind::ThreadCreated => {
            return Err(bad("post in a forum by creating a thread"));
        }
        _ => {}
    }
    if ct == ChannelType::Thread {
        if ch.archived {
            return Err(ApiError::Conflict("thread is archived"));
        }
        ca.require(P::SEND_IN_THREADS)?;
    } else {
        ca.require(P::SEND_MESSAGES)?;
    }
    if ct == ChannelType::Announcement {
        // Only people who manage messages can post announcements.
        ca.require(P::MANAGE_MESSAGES)?;
    }
    if nm.content.chars().count() > MAX_CONTENT {
        return Err(bad(format!("content longer than {MAX_CONTENT} characters")));
    }
    if let Some(n) = &nm.nonce
        && n.len() > 64
    {
        return Err(bad("nonce longer than 64 bytes"));
    }
    if ch.e2ee {
        if !nm.content.is_empty() || nm.e2ee_payload.is_none() {
            return Err(bad("E2EE channel: send e2ee_payload, not content"));
        }
        if nm
            .e2ee_payload
            .as_ref()
            .is_some_and(|p| p.len() > 64 * 1024)
        {
            return Err(bad("payload too large"));
        }
    } else if nm.e2ee_payload.is_some() {
        return Err(bad("e2ee_payload only allowed in E2EE channels"));
    }
    if nm.content.trim().is_empty()
        && nm.attachment_ids.is_empty()
        && nm.sticker_id.is_none()
        && nm.e2ee_payload.is_none()
    {
        return Err(bad("message is empty"));
    }
    if !nm.attachment_ids.is_empty() {
        ca.require(P::ATTACH_FILES)?;
        if nm.attachment_ids.len() > 10 {
            return Err(bad("at most 10 attachments"));
        }
    }

    // 1:1 DMs stop working while either side blocks the other.
    if ch.r#type == v1::ChannelType::Dm as i32 {
        for &peer in ch.recipient_ids.iter().filter(|&&u| u != author) {
            if repo::blocked_between(&st.ctx.db, author, peer).await? {
                return Err(ApiError::Forbidden("you cannot message this user"));
            }
        }
    }

    // Idempotent retry: same author + nonce in this channel returns the original.
    if let Some(n) = &nm.nonce {
        let existing = sqlx::query_scalar!(
            "SELECT id FROM messages WHERE channel_id = $1 AND author_id = $2 AND nonce = $3",
            cid as i64,
            author as i64,
            n
        )
        .fetch_optional(&st.ctx.db)
        .await?;
        if let Some(id) = existing {
            let m = repo::message(&st.ctx.db, cid, id as u64, author)
                .await?
                .ok_or(ApiError::NotFound)?;
            return Ok((m, false));
        }
    }

    // Slowmode, unless the author can manage the channel or its messages.
    if ch.slowmode_seconds > 0 && !ca.perms.intersects(P::MANAGE_MESSAGES | P::MANAGE_CHANNELS) {
        let since = neoshard_proto::snowflake::from_timestamp(
            (repo::now_ms() as u64).saturating_sub(u64::from(ch.slowmode_seconds) * 1000),
        );
        let recent = sqlx::query_scalar!(
            "SELECT max(id) FROM messages WHERE channel_id = $1 AND author_id = $2 AND id >= $3",
            cid as i64,
            author as i64,
            since as i64
        )
        .fetch_one(&st.ctx.db)
        .await?;
        if let Some(last) = recent {
            let next = neoshard_proto::snowflake::timestamp_ms(last as u64)
                + u64::from(ch.slowmode_seconds) * 1000;
            let wait = next.saturating_sub(repo::now_ms() as u64);
            return Err(ApiError::RateLimited(std::time::Duration::from_millis(
                wait,
            )));
        }
    }

    if let Some(r) = nm.reply_to_id {
        let exists = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM messages WHERE channel_id = $1 AND id = $2) AS "e!""#,
            cid as i64,
            r as i64
        )
        .fetch_one(&st.ctx.db)
        .await?;
        if !exists {
            nm.reply_to_id = None;
            nm.kind = MessageKind::Default;
        }
    }

    let mut mentions = if ch.e2ee {
        Mentions::default()
    } else {
        parse_mentions(&nm.content)
    };
    if !ca.perms.contains(P::MENTION_EVERYONE) {
        mentions.everyone = false;
    }
    // Replies notify the replied-to author.
    if let Some(r) = nm.reply_to_id
        && let Some(author_of) = sqlx::query_scalar!(
            "SELECT author_id FROM messages WHERE channel_id = $1 AND id = $2",
            cid as i64,
            r as i64
        )
        .fetch_optional(&st.ctx.db)
        .await?
    {
        let u = author_of as u64;
        if u != author && !mentions.users.contains(&u) {
            mentions.users.push(u);
        }
    }

    if let Some(ga) = &ca.guild
        && !ga.perms.intersects(P::ADMINISTRATOR | P::MANAGE_GUILD)
    {
        automod::enforce(st, ga.snap.guild_id, author, cid, &nm.content, &mentions).await?;
    }

    let id = st.ctx.id();
    let guild_id = (ch.guild_id != 0).then_some(ch.guild_id as i64);
    let mut tx = st.ctx.db.begin().await?;
    let attachments: Vec<v1::Attachment> = if nm.attachment_ids.is_empty() {
        vec![]
    } else {
        let ids: Vec<i64> = nm.attachment_ids.iter().map(|&x| x as i64).collect();
        let rows = sqlx::query!(
            "UPDATE attachments SET message_id = $1
             WHERE id = ANY($2) AND uploader_id = $3 AND channel_id = $4 AND message_id IS NULL
             RETURNING id, filename, content_type, size, object_key",
            id as i64,
            &ids,
            author as i64,
            cid as i64
        )
        .fetch_all(&mut *tx)
        .await?;
        if rows.len() != ids.len() {
            return Err(bad("unknown or already used attachment"));
        }
        rows.into_iter()
            .map(|r| v1::Attachment {
                id: r.id as u64,
                filename: r.filename,
                content_type: r.content_type,
                size: r.size as u64,
                url: crate::routes::uploads::public_url(st, &r.object_key),
                width: 0,
                height: 0,
            })
            .collect()
    };
    let attachments_json = serde_json::to_value(&attachments).map_err(anyhow::Error::from)?;
    let user_ids: Vec<i64> = mentions.users.iter().map(|&x| x as i64).collect();
    let role_ids: Vec<i64> = mentions.roles.iter().map(|&x| x as i64).collect();
    sqlx::query!(
        "INSERT INTO messages (channel_id, id, guild_id, author_id, content, kind, reply_to_id, mention_everyone,
                               mention_user_ids, mention_role_ids, attachments, nonce, webhook_id, sticker_id, e2ee_payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        cid as i64,
        id as i64,
        guild_id,
        author as i64,
        nm.content,
        nm.kind as i16,
        nm.reply_to_id.map(|x| x as i64),
        mentions.everyone,
        &user_ids,
        &role_ids,
        attachments_json,
        nm.nonce,
        nm.webhook_id.map(|x| x as i64),
        nm.sticker_id.map(|x| x as i64),
        nm.e2ee_payload
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE channels SET last_message_id = $2 WHERE id = $1",
        cid as i64,
        id as i64
    )
    .execute(&mut *tx)
    .await?;
    // The author has read their own message.
    if nm.webhook_id.is_none() {
        sqlx::query!(
            "INSERT INTO read_states (user_id, channel_id, last_read_message_id, mention_count) VALUES ($1, $2, $3, 0)
             ON CONFLICT (user_id, channel_id) DO UPDATE SET last_read_message_id = EXCLUDED.last_read_message_id, mention_count = 0",
            author as i64,
            cid as i64,
            id as i64
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let mut msg = repo::message(&st.ctx.db, cid, id, author)
        .await?
        .ok_or(ApiError::NotFound)?;
    if let Some(wid) = nm.webhook_id {
        fill_webhook_author(st, &mut msg, wid).await?;
    }
    st.publish_channel_event(ch, Intent::GuildMessages, Kind::MessageCreate(msg.clone()))
        .await;

    // Mention counters for the notified users.
    let notify: Vec<u64> = if ch.guild_id == 0 {
        ch.recipient_ids
            .iter()
            .copied()
            .filter(|&u| u != author)
            .collect()
    } else {
        // Only members who can see the channel get a mention counter (no channel-id leaks).
        let mut v = Vec::new();
        let snap = match &ca.guild {
            Some(ga) => Some(ga.snap.clone()),
            None => {
                neoshard_server_common::repo::GuildSnapshot::load(&st.ctx.db, ch.guild_id).await?
            }
        };
        if let Some(snap) = snap {
            for &u in mentions.users.iter().filter(|&&u| u != author).take(50) {
                if let Some(m) = repo::member(&st.ctx.db, ch.guild_id, u).await?
                    && snap
                        .channel_perms(&m, cid, repo::now_ms())
                        .contains(P::VIEW_CHANNEL)
                {
                    v.push(u);
                }
            }
        }
        v
    };
    bump_mentions(st, cid, &notify).await?;

    if !ch.e2ee && ca.perms.contains(P::EMBED_LINKS) {
        previews::spawn_for_message(st.clone(), ch.clone(), msg.clone());
    }
    if ch.guild_id == 0 && nm.webhook_id.is_none() {
        crate::routes::federation::deliver_dm(st.clone(), author, ch.clone(), msg.clone());
    }
    Ok((msg, true))
}

async fn bump_mentions(st: &AppState, cid: u64, users: &[u64]) -> ApiResult<()> {
    if users.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = users.iter().map(|&x| x as i64).collect();
    let rows = sqlx::query!(
        "INSERT INTO read_states (user_id, channel_id, mention_count)
         SELECT u, $2, 1 FROM unnest($1::bigint[]) AS u
         WHERE EXISTS (SELECT 1 FROM users WHERE id = u)
         ON CONFLICT (user_id, channel_id) DO UPDATE SET mention_count = read_states.mention_count + 1
         RETURNING user_id, last_read_message_id, mention_count",
        &ids,
        cid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    for r in rows {
        st.publish_user(
            r.user_id as u64,
            Intent::None,
            Kind::ReadStateUpdate(v1::ReadState {
                channel_id: cid,
                last_read_message_id: r.last_read_message_id as u64,
                mention_count: r.mention_count as u32,
            }),
        );
    }
    Ok(())
}

pub async fn fill_webhook_author(st: &AppState, msg: &mut v1::Message, wid: u64) -> ApiResult<()> {
    if let Some(w) = sqlx::query!(
        "SELECT name, avatar FROM webhooks WHERE id = $1",
        wid as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    {
        msg.author = Some(v1::User {
            id: wid,
            username: w.name.clone(),
            display_name: w.name,
            avatar: w.avatar,
            bot: true,
            ..Default::default()
        });
    }
    Ok(())
}

// ------------------------------------------------------------------ read

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default, deserialize_with = "extract::opt_id")]
    before: Option<u64>,
    #[serde(default, deserialize_with = "extract::opt_id")]
    after: Option<u64>,
    #[serde(default, deserialize_with = "extract::opt_id")]
    around: Option<u64>,
    limit: Option<i64>,
}

async fn list(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<v1::Message>>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::READ_MESSAGE_HISTORY)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    let mut msgs = if let Some(around) = q.around {
        let mut older = repo::messages_page(
            &st.ctx.db,
            cid,
            Some(around + 1),
            None,
            limit / 2 + 1,
            a.id(),
        )
        .await?;
        let newer =
            repo::messages_page(&st.ctx.db, cid, None, Some(around), limit / 2, a.id()).await?;
        older.extend(newer);
        older
    } else {
        repo::messages_page(&st.ctx.db, cid, q.before, q.after, limit, a.id()).await?
    };
    for m in msgs.iter_mut().filter(|m| m.webhook_id != 0) {
        let wid = m.webhook_id;
        fill_webhook_author(&st, m, wid).await?;
    }
    Ok(Json(msgs))
}

async fn get_one(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid)): Path<(u64, u64)>,
) -> ApiResult<Json<v1::Message>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::READ_MESSAGE_HISTORY)?;
    Ok(Json(
        repo::message(&st.ctx.db, cid, mid, a.id())
            .await?
            .ok_or(ApiError::NotFound)?,
    ))
}

// ------------------------------------------------------------------ edit / delete

#[derive(Deserialize)]
struct EditBody {
    content: Option<String>,
    e2ee_payload: Option<String>,
    /// Remove all embeds (link previews).
    #[serde(default)]
    suppress_embeds: bool,
}

async fn edit(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid)): Path<(u64, u64)>,
    Json(b): Json<EditBody>,
) -> ApiResult<Json<v1::Message>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    let author = sqlx::query_scalar!(
        "SELECT author_id FROM messages WHERE channel_id = $1 AND id = $2 AND webhook_id IS NULL",
        cid as i64,
        mid as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if author as u64 != a.id() {
        if b.content.is_some() || b.e2ee_payload.is_some() || !b.suppress_embeds {
            return Err(ApiError::Forbidden("only the author can edit a message"));
        }
        ca.require(P::MANAGE_MESSAGES)?;
    } else if b.content.is_some() || b.e2ee_payload.is_some() {
        // Muted / timed-out authors cannot rewrite history either.
        ca.require(P::SEND_MESSAGES)?;
    }
    if b.content
        .as_ref()
        .is_some_and(|c| c.chars().count() > MAX_CONTENT)
    {
        return Err(bad(format!("content longer than {MAX_CONTENT} characters")));
    }
    let payload = b
        .e2ee_payload
        .map(|p| base64::engine::general_purpose::STANDARD.decode(p))
        .transpose()
        .map_err(|_| bad("bad base64"))?;
    if payload.as_ref().is_some_and(|p| p.len() > 64 * 1024) {
        return Err(bad("payload too large"));
    }
    if ca.channel.e2ee && b.content.as_ref().is_some_and(|c| !c.is_empty()) {
        return Err(bad("E2EE channel: edit e2ee_payload, not content"));
    }
    let mentions = b.content.as_deref().map(parse_mentions).unwrap_or_default();
    if let Some(c) = &b.content
        && let Some(ga) = &ca.guild
        && !ga.perms.intersects(P::ADMINISTRATOR | P::MANAGE_GUILD)
    {
        automod::enforce(&st, ga.snap.guild_id, a.id(), cid, c, &mentions).await?;
    }
    let everyone = mentions.everyone && ca.perms.contains(P::MENTION_EVERYONE);
    let users: Vec<i64> = mentions.users.iter().map(|&x| x as i64).collect();
    let roles: Vec<i64> = mentions.roles.iter().map(|&x| x as i64).collect();
    sqlx::query!(
        "UPDATE messages SET
            content = COALESCE($3, content),
            e2ee_payload = COALESCE($4, e2ee_payload),
            edited_at = CASE WHEN $3::text IS NOT NULL OR $4::bytea IS NOT NULL THEN now() ELSE edited_at END,
            mention_everyone = CASE WHEN $3::text IS NOT NULL THEN $5 ELSE mention_everyone END,
            mention_user_ids = CASE WHEN $3::text IS NOT NULL THEN $6 ELSE mention_user_ids END,
            mention_role_ids = CASE WHEN $3::text IS NOT NULL THEN $7 ELSE mention_role_ids END,
            embeds = CASE WHEN $8 THEN '[]'::jsonb ELSE embeds END
         WHERE channel_id = $1 AND id = $2",
        cid as i64,
        mid as i64,
        b.content,
        payload,
        everyone,
        &users,
        &roles,
        b.suppress_embeds
    )
    .execute(&st.ctx.db)
    .await?;
    let msg = repo::message(&st.ctx.db, cid, mid, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    st.publish_channel_event(
        &ca.channel,
        Intent::GuildMessages,
        Kind::MessageUpdate(msg.clone()),
    )
    .await;
    if b.content.is_some()
        && !ca.channel.e2ee
        && ca.perms.contains(P::EMBED_LINKS)
        && !b.suppress_embeds
    {
        previews::spawn_for_message(st.clone(), ca.channel.clone(), msg.clone());
    }
    Ok(Json(msg))
}

async fn remove(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    let author = sqlx::query_scalar!(
        "SELECT author_id FROM messages WHERE channel_id = $1 AND id = $2",
        cid as i64,
        mid as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if author as u64 != a.id() {
        if ca.is_dm() {
            return Err(ApiError::Forbidden(
                "cannot delete other people's messages in DMs",
            ));
        }
        ca.require(P::MANAGE_MESSAGES)?;
        crate::authz::audit(
            &st,
            ca.guild_id(),
            a.id(),
            "message.delete",
            author as u64,
            json!({ "channel_id": cid.to_string() }),
            "",
        )
        .await?;
    }
    delete_messages(&st, &ca.channel, &[mid]).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_messages(st: &AppState, ch: &v1::Channel, ids: &[u64]) -> ApiResult<()> {
    let raw: Vec<i64> = ids.iter().map(|&x| x as i64).collect();
    sqlx::query!(
        "DELETE FROM messages WHERE channel_id = $1 AND id = ANY($2)",
        ch.id as i64,
        &raw
    )
    .execute(&st.ctx.db)
    .await?;
    sqlx::query!(
        "DELETE FROM reactions WHERE channel_id = $1 AND message_id = ANY($2)",
        ch.id as i64,
        &raw
    )
    .execute(&st.ctx.db)
    .await?;
    let kind = if ids.len() == 1 {
        Kind::MessageDelete(v1::MessageDelete {
            id: ids[0],
            channel_id: ch.id,
            guild_id: ch.guild_id,
        })
    } else {
        Kind::MessageDeleteBulk(v1::MessageDeleteBulk {
            ids: ids.to_vec(),
            channel_id: ch.id,
            guild_id: ch.guild_id,
        })
    };
    st.publish_channel_event(ch, Intent::GuildMessages, kind)
        .await;
    Ok(())
}

#[derive(Deserialize)]
struct BulkDelete {
    #[serde(deserialize_with = "extract::ids")]
    ids: Vec<u64>,
}

async fn bulk_delete(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(b): Json<BulkDelete>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if ca.is_dm() {
        return Err(bad("bulk delete is not available in DMs"));
    }
    ca.require(P::MANAGE_MESSAGES)?;
    if b.ids.is_empty() || b.ids.len() > 100 {
        return Err(bad("2-100 ids"));
    }
    delete_messages(&st, &ca.channel, &b.ids).await?;
    crate::authz::audit(
        &st,
        ca.guild_id(),
        a.id(),
        "message.bulk_delete",
        cid,
        json!({ "count": b.ids.len() }),
        "",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ reactions

/// Unicode emoji (no whitespace/control, ≤ 64 bytes) or custom `name:id`.
async fn validate_emoji(st: &AppState, ca: &ChannelAccess, emoji: &str) -> ApiResult<()> {
    if emoji.is_empty()
        || emoji.len() > 64
        || emoji.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(bad("invalid emoji"));
    }
    if let Some((name, id)) = emoji.rsplit_once(':') {
        let id: i64 = id.parse().map_err(|_| bad("invalid custom emoji"))?;
        let g = sqlx::query_scalar!(
            "SELECT guild_id FROM emojis WHERE id = $1 AND name = $2",
            id,
            name
        )
        .fetch_optional(&st.ctx.db)
        .await?
        .ok_or(bad("unknown custom emoji"))?;
        if g as u64 != ca.channel.guild_id {
            ca.require(P::USE_EXTERNAL_EMOJI)?;
        }
    } else if emoji.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(bad("invalid emoji"));
    }
    Ok(())
}

async fn message_exists(st: &AppState, cid: u64, mid: u64) -> ApiResult<()> {
    let ok = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM messages WHERE channel_id = $1 AND id = $2) AS "e!""#,
        cid as i64,
        mid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if ok { Ok(()) } else { Err(ApiError::NotFound) }
}

async fn react(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid, emoji)): Path<(u64, u64, String)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::READ_MESSAGE_HISTORY | P::ADD_REACTIONS)?;
    validate_emoji(&st, &ca, &emoji).await?;
    message_exists(&st, cid, mid).await?;
    let distinct = sqlx::query_scalar!(r#"SELECT count(DISTINCT emoji) AS "c!" FROM reactions WHERE channel_id = $2 AND message_id = $1"#, mid as i64, cid as i64).fetch_one(&st.ctx.db).await?;
    if distinct >= 20 {
        let has = sqlx::query_scalar!(r#"SELECT EXISTS(SELECT 1 FROM reactions WHERE message_id = $1 AND emoji = $2) AS "e!""#, mid as i64, emoji)
            .fetch_one(&st.ctx.db)
            .await?;
        if !has {
            return Err(bad("at most 20 different reactions per message"));
        }
    }
    let r = sqlx::query!(
        "INSERT INTO reactions (channel_id, message_id, emoji, user_id) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        cid as i64,
        mid as i64,
        emoji,
        a.id() as i64
    )
    .execute(&st.ctx.db)
    .await?;
    if r.rows_affected() > 0 {
        let ev = v1::ReactionEvent {
            message_id: mid,
            channel_id: cid,
            guild_id: ca.channel.guild_id,
            user_id: a.id(),
            emoji,
        };
        st.publish_channel_event(&ca.channel, Intent::GuildReactions, Kind::ReactionAdd(ev))
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn unreact(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid, emoji)): Path<(u64, u64, String)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    remove_reaction(&st, &ca, mid, &emoji, a.id()).await
}

async fn remove_user_reaction(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid, emoji, uid)): Path<(u64, u64, String, u64)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if uid != a.id() {
        ca.require(P::MANAGE_MESSAGES)?;
    }
    remove_reaction(&st, &ca, mid, &emoji, uid).await
}

async fn remove_reaction(
    st: &AppState,
    ca: &ChannelAccess,
    mid: u64,
    emoji: &str,
    uid: u64,
) -> ApiResult<StatusCode> {
    let r = sqlx::query!("DELETE FROM reactions WHERE channel_id = $4 AND message_id = $1 AND emoji = $2 AND user_id = $3", mid as i64, emoji, uid as i64, ca.channel.id as i64)
        .execute(&st.ctx.db)
        .await?;
    if r.rows_affected() > 0 {
        let ev = v1::ReactionEvent {
            message_id: mid,
            channel_id: ca.channel.id,
            guild_id: ca.channel.guild_id,
            user_id: uid,
            emoji: emoji.into(),
        };
        st.publish_channel_event(
            &ca.channel,
            Intent::GuildReactions,
            Kind::ReactionRemove(ev),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_all_emoji(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid, emoji)): Path<(u64, u64, String)>,
) -> ApiResult<StatusCode> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::MANAGE_MESSAGES)?;
    let users = sqlx::query_scalar!("DELETE FROM reactions WHERE channel_id = $3 AND message_id = $1 AND emoji = $2 RETURNING user_id", mid as i64, emoji, cid as i64)
        .fetch_all(&st.ctx.db)
        .await?;
    for uid in users {
        let ev = v1::ReactionEvent {
            message_id: mid,
            channel_id: cid,
            guild_id: ca.channel.guild_id,
            user_id: uid as u64,
            emoji: emoji.clone(),
        };
        st.publish_channel_event(
            &ca.channel,
            Intent::GuildReactions,
            Kind::ReactionRemove(ev),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn reaction_users(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid, emoji)): Path<(u64, u64, String)>,
) -> ApiResult<Json<Vec<v1::User>>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::READ_MESSAGE_HISTORY)?;
    let ids = sqlx::query_scalar!("SELECT user_id FROM reactions WHERE channel_id = $3 AND message_id = $1 AND emoji = $2 ORDER BY created_at LIMIT 100", mid as i64, emoji, cid as i64)
        .fetch_all(&st.ctx.db)
        .await?;
    let map = repo::users(&st.ctx.db, &ids).await?;
    Ok(Json(
        ids.iter()
            .filter_map(|id| map.get(&(*id as u64)).cloned())
            .collect(),
    ))
}

// ------------------------------------------------------------------ pins

async fn pins(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
) -> ApiResult<Json<Vec<v1::Message>>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::READ_MESSAGE_HISTORY)?;
    let rows = sqlx::query_as!(
        MessageRow,
        "SELECT channel_id, id, guild_id, author_id, content, kind, reply_to_id, edited_at, pinned, mention_everyone,
                mention_user_ids, mention_role_ids, attachments, embeds, nonce, webhook_id, sticker_id, thread_id, e2ee_payload
         FROM messages WHERE channel_id = $1 AND pinned ORDER BY id DESC LIMIT 50",
        cid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    Ok(Json(repo::hydrate(&st.ctx.db, rows, a.id()).await?))
}

async fn set_pin(
    st: &AppState,
    a: Auth,
    cid: u64,
    mid: u64,
    pinned: bool,
) -> ApiResult<StatusCode> {
    let ca = channel_access(st, cid, a.id()).await?;
    if !ca.is_dm() {
        ca.require(P::MANAGE_MESSAGES)?;
    }
    if pinned {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "c!" FROM messages WHERE channel_id = $1 AND pinned"#,
            cid as i64
        )
        .fetch_one(&st.ctx.db)
        .await?;
        if n >= MAX_PINS {
            return Err(bad("at most 50 pins per channel"));
        }
    }
    let r = sqlx::query!(
        "UPDATE messages SET pinned = $3 WHERE channel_id = $1 AND id = $2",
        cid as i64,
        mid as i64,
        pinned
    )
    .execute(&st.ctx.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    if let Some(m) = repo::message(&st.ctx.db, cid, mid, a.id()).await? {
        st.publish_channel_event(&ca.channel, Intent::GuildMessages, Kind::MessageUpdate(m))
            .await;
    }
    st.publish_channel_event(
        &ca.channel,
        Intent::GuildMessages,
        Kind::PinsUpdate(v1::PinsUpdate {
            channel_id: cid,
            guild_id: ca.channel.guild_id,
        }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn pin(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    set_pin(&st, a, cid, mid, true).await
}

async fn unpin(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    set_pin(&st, a, cid, mid, false).await
}

// ------------------------------------------------------------------ read state, typing

pub async fn ack_internal(
    st: &AppState,
    user: u64,
    cid: u64,
    mid: u64,
) -> ApiResult<v1::ReadState> {
    let r = sqlx::query!(
        "INSERT INTO read_states (user_id, channel_id, last_read_message_id, mention_count) VALUES ($1, $2, $3, 0)
         ON CONFLICT (user_id, channel_id) DO UPDATE
            SET last_read_message_id = GREATEST(read_states.last_read_message_id, EXCLUDED.last_read_message_id), mention_count = 0
         RETURNING last_read_message_id",
        user as i64,
        cid as i64,
        mid as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    let rs = v1::ReadState {
        channel_id: cid,
        last_read_message_id: r.last_read_message_id as u64,
        mention_count: 0,
    };
    st.publish_user(user, Intent::None, Kind::ReadStateUpdate(rs));
    Ok(rs)
}

async fn ack(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid)): Path<(u64, u64)>,
) -> ApiResult<Json<v1::ReadState>> {
    channel_access(&st, cid, a.id()).await?;
    Ok(Json(ack_internal(&st, a.id(), cid, mid).await?))
}

async fn typing(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
) -> ApiResult<StatusCode> {
    st.limiter
        .check(&format!("typing:{}", a.id()), rates::TYPING)?;
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::SEND_MESSAGES)?;
    let ev = v1::TypingStart {
        channel_id: cid,
        guild_id: ca.channel.guild_id,
        user_id: a.id(),
        at: repo::now_ms(),
    };
    st.publish_channel_event(&ca.channel, Intent::GuildTyping, Kind::TypingStart(ev))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ crosspost

async fn crosspost(
    State(st): State<AppState>,
    a: Auth,
    Path((cid, mid)): Path<(u64, u64)>,
) -> ApiResult<Json<serde_json::Value>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    if ca.channel.r#type != ChannelType::Announcement as i32 {
        return Err(bad("not an announcement channel"));
    }
    ca.require(P::MANAGE_MESSAGES)?;
    let msg = repo::message(&st.ctx.db, cid, mid, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    let hooks = sqlx::query!(
        "SELECT w.id, w.channel_id FROM channel_follows f JOIN webhooks w ON w.id = f.webhook_id WHERE f.source_channel_id = $1",
        cid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    let mut sent = 0;
    for h in hooks {
        if let Some(target) = repo::channel(&st.ctx.db, h.channel_id as u64).await? {
            let nm = NewMessage {
                content: msg.content.clone(),
                webhook_id: Some(h.id as u64),
                ..Default::default()
            };
            if create_internal(&st, h.id as u64, &target, nm).await.is_ok() {
                sent += 1;
            }
        }
    }
    Ok(Json(json!({ "delivered": sent })))
}

// ------------------------------------------------------------------ search

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default, deserialize_with = "extract::opt_id")]
    author_id: Option<u64>,
    #[serde(default, deserialize_with = "extract::opt_id")]
    channel_id: Option<u64>,
    /// "attachment" | "link" | "pinned"
    has: Option<String>,
    #[serde(default)]
    offset: i64,
    limit: Option<i64>,
}

/// Full-text search (Postgres `tsvector`, `simple` config → language-agnostic).
async fn search_guild(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let ga = guild_access(&st, gid, a.id()).await?;
    let now = repo::now_ms();
    let visible: Vec<i64> = ga
        .snap
        .channels
        .keys()
        .filter(|cid| {
            ga.snap
                .channel_perms(&ga.member, **cid, now)
                .contains(P::VIEW_CHANNEL | P::READ_MESSAGE_HISTORY)
        })
        .filter(|cid| q.channel_id.is_none_or(|c| c == **cid))
        .map(|&c| c as i64)
        .collect();
    run_search(&st, a.id(), &visible, &q).await
}

async fn search_channel(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::READ_MESSAGE_HISTORY)?;
    if ca.channel.e2ee {
        return Err(bad("E2EE channels can only be searched on the client"));
    }
    run_search(&st, a.id(), &[cid as i64], &q).await
}

async fn run_search(
    st: &AppState,
    viewer: u64,
    channels: &[i64],
    q: &SearchQuery,
) -> ApiResult<Json<serde_json::Value>> {
    let text = q.q.trim();
    if text.is_empty() && q.author_id.is_none() && q.has.is_none() {
        return Err(bad("empty query"));
    }
    if text.len() > 200 {
        return Err(bad("query too long"));
    }
    let has = q.has.as_deref().unwrap_or("");
    let limit = q.limit.unwrap_or(25).clamp(1, 50);
    let rows = sqlx::query_as!(
        MessageRow,
        "SELECT channel_id, id, guild_id, author_id, content, kind, reply_to_id, edited_at, pinned, mention_everyone,
                mention_user_ids, mention_role_ids, attachments, embeds, nonce, webhook_id, sticker_id, thread_id, e2ee_payload
         FROM messages
         WHERE channel_id = ANY($1)
           AND ($2 = '' OR tsv @@ websearch_to_tsquery('simple', $2))
           AND ($3::bigint IS NULL OR author_id = $3)
           AND ($4 <> 'attachment' OR attachments <> '[]'::jsonb)
           AND ($4 <> 'link' OR content ~ 'https?://')
           AND ($4 <> 'pinned' OR pinned)
         ORDER BY id DESC OFFSET $5 LIMIT $6",
        channels,
        text,
        q.author_id.map(|x| x as i64),
        has,
        q.offset.max(0),
        limit
    )
    .fetch_all(&st.ctx.db)
    .await?;
    let msgs = repo::hydrate(&st.ctx.db, rows, viewer).await?;
    let mut by_channel: HashMap<String, usize> = HashMap::new();
    for m in &msgs {
        *by_channel.entry(m.channel_id.to_string()).or_default() += 1;
    }
    Ok(Json(json!({ "messages": msgs, "by_channel": by_channel })))
}

/// Publish typing from the gateway path (already authorized).
pub fn typing_event(ch: &v1::Channel, user: u64) -> BusEvent {
    let ev = Kind::TypingStart(v1::TypingStart {
        channel_id: ch.id,
        guild_id: ch.guild_id,
        user_id: user,
        at: repo::now_ms(),
    });
    BusEvent::channel(ch.guild_id, ch.id, Intent::GuildTyping, ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions() {
        let m = parse_mentions("hi <@123> and <@!456>, role <@&789> @everyone");
        assert_eq!(
            m,
            Mentions {
                users: vec![123, 456],
                roles: vec![789],
                everyone: true
            }
        );
    }

    #[test]
    fn mentions_in_code_ignored() {
        let m = parse_mentions("`<@1> @everyone` real <@2>");
        assert_eq!(m.users, vec![2]);
        assert!(!m.everyone);
    }

    #[test]
    fn malformed_mentions() {
        let m = parse_mentions("<@> <@abc> <@12 <@&>");
        assert_eq!(m, Mentions::default());
        assert_eq!(parse_mentions("<@5><@5>").users, vec![5]);
        assert!(parse_mentions("@here").everyone);
    }
}
