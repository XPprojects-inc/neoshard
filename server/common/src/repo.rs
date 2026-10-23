//! Shared read queries and row → protocol conversions, used by the API and
//! the gateway. All queries are compile-time checked (`sqlx::query!`).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use neoshard_proto::permissions::{MemberContext, Permissions};
use neoshard_proto::v1;
use sqlx::PgExecutor;

pub type Db = sqlx::PgPool;

pub fn ms(t: Option<DateTime<Utc>>) -> i64 {
    t.map(|t| t.timestamp_millis()).unwrap_or(0)
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn ids(v: &[i64]) -> Vec<u64> {
    v.iter().map(|&x| x as u64).collect()
}

// ------------------------------------------------------------------ users

pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub avatar: String,
    pub flags: i64,
    pub bot: bool,
    pub instance: String,
    pub mfa_enabled: bool,
    pub avatar_decoration: String,
    pub nameplate: String,
    pub name_font: String,
    pub name_effect: String,
    pub name_colors: Vec<i32>,
    pub tag_guild_id: Option<i64>,
    pub tag: Option<String>,
    pub tag_badge: Option<String>,
}

impl UserRow {
    /// Public view (no private fields).
    pub fn public(self) -> v1::User {
        let name_style = (!self.name_font.is_empty()
            || !self.name_effect.is_empty()
            || !self.name_colors.is_empty())
        .then(|| v1::NameStyle {
            font: self.name_font,
            effect: self.name_effect,
            colors: self.name_colors.iter().map(|&c| c as u32).collect(),
        });
        let guild_tag = match (self.tag_guild_id, self.tag) {
            (Some(gid), Some(tag)) if !tag.is_empty() => Some(v1::GuildTag {
                guild_id: gid as u64,
                tag,
                badge: self.tag_badge.unwrap_or_default(),
            }),
            _ => None,
        };
        v1::User {
            id: self.id as u64,
            username: self.username,
            display_name: self.display_name,
            avatar: self.avatar,
            flags: self.flags as u64,
            bot: self.bot,
            instance: self.instance,
            mfa_enabled: false,
            avatar_decoration: self.avatar_decoration,
            nameplate: self.nameplate,
            name_style,
            guild_tag,
        }
    }

    /// View for the user themself.
    pub fn private(self) -> v1::User {
        let mfa = self.mfa_enabled;
        let mut u = self.public();
        u.mfa_enabled = mfa;
        u
    }
}

// The server tag is shown only while the user is still a member of that guild.
pub async fn user(db: impl PgExecutor<'_>, id: u64) -> sqlx::Result<Option<UserRow>> {
    sqlx::query_as!(
        UserRow,
        r#"SELECT u.id, u.username, u.display_name, u.avatar, u.flags, u.bot, u.instance, u.mfa_enabled,
                  u.avatar_decoration, u.nameplate, u.name_font, u.name_effect, u.name_colors,
                  g.id AS "tag_guild_id?", g.tag AS "tag?", g.tag_badge AS "tag_badge?"
           FROM users u
           LEFT JOIN guilds g ON g.id = u.primary_guild_id AND g.tag <> ''
                AND EXISTS (SELECT 1 FROM members m WHERE m.guild_id = g.id AND m.user_id = u.id)
           WHERE u.id = $1 AND u.deleted_at IS NULL"#,
        id as i64
    )
    .fetch_optional(db)
    .await
}

pub async fn users(db: impl PgExecutor<'_>, ids: &[i64]) -> sqlx::Result<HashMap<u64, v1::User>> {
    let rows = sqlx::query_as!(
        UserRow,
        r#"SELECT u.id, u.username, u.display_name, u.avatar, u.flags, u.bot, u.instance, u.mfa_enabled,
                  u.avatar_decoration, u.nameplate, u.name_font, u.name_effect, u.name_colors,
                  g.id AS "tag_guild_id?", g.tag AS "tag?", g.tag_badge AS "tag_badge?"
           FROM users u
           LEFT JOIN guilds g ON g.id = u.primary_guild_id AND g.tag <> ''
                AND EXISTS (SELECT 1 FROM members m WHERE m.guild_id = g.id AND m.user_id = u.id)
           WHERE u.id = ANY($1)"#,
        ids
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.id as u64, r.public()))
        .collect())
}

// ------------------------------------------------------------------ channels

pub struct ChannelRow {
    pub id: i64,
    pub guild_id: Option<i64>,
    pub r#type: i16,
    pub name: String,
    pub topic: String,
    pub position: i32,
    pub parent_id: Option<i64>,
    pub nsfw: bool,
    pub slowmode_seconds: i32,
    pub last_message_id: i64,
    pub owner_id: Option<i64>,
    pub e2ee: bool,
    pub archived: bool,
}

impl ChannelRow {
    pub fn into_proto(
        self,
        overwrites: Vec<v1::PermissionOverwrite>,
        recipients: Vec<u64>,
    ) -> v1::Channel {
        v1::Channel {
            id: self.id as u64,
            guild_id: self.guild_id.unwrap_or(0) as u64,
            r#type: i32::from(self.r#type),
            name: self.name,
            topic: self.topic,
            position: self.position,
            parent_id: self.parent_id.unwrap_or(0) as u64,
            nsfw: self.nsfw,
            slowmode_seconds: self.slowmode_seconds as u32,
            overwrites,
            last_message_id: self.last_message_id as u64,
            recipient_ids: recipients,
            e2ee: self.e2ee,
            owner_id: self.owner_id.unwrap_or(0) as u64,
            forum_tags: vec![],
            archived: self.archived,
        }
    }
}

pub fn is_dm_type(t: i32) -> bool {
    t == v1::ChannelType::Dm as i32 || t == v1::ChannelType::GroupDm as i32
}

async fn overwrites_for(
    db: impl PgExecutor<'_>,
    channel_ids: &[i64],
) -> sqlx::Result<HashMap<u64, Vec<v1::PermissionOverwrite>>> {
    let rows = sqlx::query!(
        "SELECT channel_id, target_id, target_type, allow, deny FROM permission_overwrites WHERE channel_id = ANY($1)",
        channel_ids
    )
    .fetch_all(db)
    .await?;
    let mut map: HashMap<u64, Vec<v1::PermissionOverwrite>> = HashMap::new();
    for r in rows {
        map.entry(r.channel_id as u64)
            .or_default()
            .push(v1::PermissionOverwrite {
                id: r.target_id as u64,
                r#type: i32::from(r.target_type),
                allow: r.allow as u64,
                deny: r.deny as u64,
            });
    }
    Ok(map)
}

async fn recipients_for(
    db: impl PgExecutor<'_>,
    channel_ids: &[i64],
) -> sqlx::Result<HashMap<u64, Vec<u64>>> {
    let rows = sqlx::query!(
        "SELECT channel_id, user_id FROM channel_recipients WHERE channel_id = ANY($1)",
        channel_ids
    )
    .fetch_all(db)
    .await?;
    let mut map: HashMap<u64, Vec<u64>> = HashMap::new();
    for r in rows {
        map.entry(r.channel_id as u64)
            .or_default()
            .push(r.user_id as u64);
    }
    Ok(map)
}

pub async fn channel(db: &Db, id: u64) -> sqlx::Result<Option<v1::Channel>> {
    let row = sqlx::query_as!(
        ChannelRow,
        "SELECT id, guild_id, type, name, topic, position, parent_id, nsfw, slowmode_seconds, last_message_id, owner_id, e2ee, archived
         FROM channels WHERE id = $1",
        id as i64
    )
    .fetch_optional(db)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let ids = [row.id];
    let mut ow = overwrites_for(db, &ids).await?;
    let mut rc = if is_dm_type(i32::from(row.r#type)) {
        recipients_for(db, &ids).await?
    } else {
        HashMap::new()
    };
    let mut ch = row.into_proto(
        ow.remove(&id).unwrap_or_default(),
        rc.remove(&id).unwrap_or_default(),
    );
    if ch.r#type == v1::ChannelType::Forum as i32 {
        ch.forum_tags = forum_tags(db, id).await?;
    }
    Ok(Some(ch))
}

pub async fn forum_tags(db: &Db, channel_id: u64) -> sqlx::Result<Vec<v1::ForumTag>> {
    Ok(sqlx::query!(
        "SELECT id, name, emoji FROM forum_tags WHERE channel_id = $1 ORDER BY id",
        channel_id as i64
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| v1::ForumTag {
        id: r.id as u64,
        name: r.name,
        emoji: r.emoji,
    })
    .collect())
}

pub async fn guild_channels(db: &Db, guild_id: u64) -> sqlx::Result<Vec<v1::Channel>> {
    let rows = sqlx::query_as!(
        ChannelRow,
        "SELECT id, guild_id, type, name, topic, position, parent_id, nsfw, slowmode_seconds, last_message_id, owner_id, e2ee, archived
         FROM channels WHERE guild_id = $1 AND NOT (type = 6 AND archived) ORDER BY position, id",
        guild_id as i64
    )
    .fetch_all(db)
    .await?;
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut ow = overwrites_for(db, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let id = r.id as u64;
            r.into_proto(ow.remove(&id).unwrap_or_default(), vec![])
        })
        .collect())
}

pub async fn private_channels(db: &Db, user_id: u64) -> sqlx::Result<Vec<v1::Channel>> {
    let rows = sqlx::query_as!(
        ChannelRow,
        "SELECT c.id, c.guild_id, c.type, c.name, c.topic, c.position, c.parent_id, c.nsfw, c.slowmode_seconds,
                c.last_message_id, c.owner_id, c.e2ee, c.archived
         FROM channels c JOIN channel_recipients r ON r.channel_id = c.id
         WHERE r.user_id = $1 ORDER BY c.last_message_id DESC",
        user_id as i64
    )
    .fetch_all(db)
    .await?;
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut rc = recipients_for(db, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let id = r.id as u64;
            r.into_proto(vec![], rc.remove(&id).unwrap_or_default())
        })
        .collect())
}

pub async fn is_recipient(
    db: impl PgExecutor<'_>,
    channel_id: u64,
    user_id: u64,
) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM channel_recipients WHERE channel_id = $1 AND user_id = $2) AS "e!""#,
        channel_id as i64,
        user_id as i64
    )
    .fetch_one(db)
    .await
}

pub async fn recipients(db: impl PgExecutor<'_>, channel_id: u64) -> sqlx::Result<Vec<u64>> {
    Ok(ids(&sqlx::query_scalar!(
        "SELECT user_id FROM channel_recipients WHERE channel_id = $1",
        channel_id as i64
    )
    .fetch_all(db)
    .await?))
}

// ------------------------------------------------------------------ roles, members

pub async fn roles(db: impl PgExecutor<'_>, guild_id: u64) -> sqlx::Result<Vec<v1::Role>> {
    Ok(sqlx::query!(
        "SELECT id, guild_id, name, color, position, permissions, hoist, mentionable FROM roles WHERE guild_id = $1 ORDER BY position, id",
        guild_id as i64
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| v1::Role {
        id: r.id as u64,
        guild_id: r.guild_id as u64,
        name: r.name,
        color: r.color as u32,
        position: r.position,
        permissions: r.permissions as u64,
        hoist: r.hoist,
        mentionable: r.mentionable,
    })
    .collect())
}

pub async fn member(db: &Db, guild_id: u64, user_id: u64) -> sqlx::Result<Option<v1::Member>> {
    let row = sqlx::query!(
        r#"SELECT m.nick, m.joined_at, m.timeout_until, m.avatar,
                  COALESCE(array_agg(mr.role_id) FILTER (WHERE mr.role_id IS NOT NULL), '{}') AS "role_ids!"
           FROM members m LEFT JOIN member_roles mr ON mr.guild_id = m.guild_id AND mr.user_id = m.user_id
           WHERE m.guild_id = $1 AND m.user_id = $2
           GROUP BY m.guild_id, m.user_id"#,
        guild_id as i64,
        user_id as i64
    )
    .fetch_optional(db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let u = user(db, user_id).await?.map(UserRow::public);
    Ok(Some(v1::Member {
        guild_id,
        user: u,
        nick: r.nick,
        role_ids: ids(&r.role_ids),
        joined_at: r.joined_at.timestamp_millis(),
        timeout_until: ms(r.timeout_until),
        avatar: r.avatar,
    }))
}

/// Page of members ordered by (joined_at, user_id) — used by the lazy member list.
pub async fn members_page(
    db: &Db,
    guild_id: u64,
    offset: i64,
    limit: i64,
) -> sqlx::Result<Vec<v1::Member>> {
    let rows = sqlx::query!(
        r#"SELECT m.user_id, m.nick, m.joined_at, m.timeout_until, m.avatar,
                  COALESCE(array_agg(mr.role_id) FILTER (WHERE mr.role_id IS NOT NULL), '{}') AS "role_ids!"
           FROM members m
           LEFT JOIN member_roles mr ON mr.guild_id = m.guild_id AND mr.user_id = m.user_id
           WHERE m.guild_id = $1
           GROUP BY m.guild_id, m.user_id
           ORDER BY m.joined_at, m.user_id
           OFFSET $2 LIMIT $3"#,
        guild_id as i64,
        offset,
        limit
    )
    .fetch_all(db)
    .await?;
    let user_ids: Vec<i64> = rows.iter().map(|r| r.user_id).collect();
    let mut by_id = users(db, &user_ids).await?;
    Ok(rows
        .into_iter()
        .map(|r| v1::Member {
            guild_id,
            user: by_id.remove(&(r.user_id as u64)),
            nick: r.nick,
            role_ids: ids(&r.role_ids),
            joined_at: r.joined_at.timestamp_millis(),
            timeout_until: ms(r.timeout_until),
            avatar: r.avatar,
        })
        .collect())
}

pub async fn member_count(db: impl PgExecutor<'_>, guild_id: u64) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM members WHERE guild_id = $1"#,
        guild_id as i64
    )
    .fetch_one(db)
    .await
}

pub async fn guild_ids_for_user(db: impl PgExecutor<'_>, user_id: u64) -> sqlx::Result<Vec<u64>> {
    Ok(ids(&sqlx::query_scalar!(
        "SELECT guild_id FROM members WHERE user_id = $1",
        user_id as i64
    )
    .fetch_all(db)
    .await?))
}

/// Full guild object as seen by `viewer` (channels they cannot view are omitted).
pub async fn guild_full(db: &Db, guild_id: u64, viewer: u64) -> sqlx::Result<Option<v1::Guild>> {
    let g = sqlx::query!(
        "SELECT id, name, icon, owner_id, instance, tag, tag_badge FROM guilds WHERE id = $1",
        guild_id as i64
    )
    .fetch_optional(db)
    .await?;
    let Some(g) = g else { return Ok(None) };
    let self_member = member(db, guild_id, viewer).await?;
    let roles = roles(db, guild_id).await?;
    let mut channels = guild_channels(db, guild_id).await?;
    if let Some(m) = &self_member {
        let snap = GuildSnapshot::from_parts(guild_id, g.owner_id as u64, &roles, &channels);
        channels.retain(|c| {
            snap.channel_perms(m, c.id, now_ms())
                .contains(Permissions::VIEW_CHANNEL)
        });
    } else {
        channels.clear();
    }
    let emojis = sqlx::query!(
        "SELECT id, name, animated, object_key FROM emojis WHERE guild_id = $1",
        guild_id as i64
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|e| v1::Emoji {
        id: e.id as u64,
        guild_id,
        name: e.name,
        animated: e.animated,
        key: e.object_key,
    })
    .collect();
    let stickers = sqlx::query!(
        "SELECT id, name, format, object_key FROM stickers WHERE guild_id = $1",
        guild_id as i64
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|s| v1::Sticker {
        id: s.id as u64,
        guild_id,
        name: s.name,
        format: s.format,
        key: s.object_key,
    })
    .collect();
    Ok(Some(v1::Guild {
        id: g.id as u64,
        name: g.name,
        icon: g.icon,
        owner_id: g.owner_id as u64,
        channels,
        roles,
        emojis,
        stickers,
        member_count: member_count(db, guild_id).await? as u32,
        self_member,
        instance: g.instance,
        voice_states: vec![],
        tag: g.tag,
        tag_badge: g.tag_badge,
    }))
}

pub async fn read_states(db: &Db, user_id: u64) -> sqlx::Result<Vec<v1::ReadState>> {
    Ok(sqlx::query!(
        "SELECT channel_id, last_read_message_id, mention_count FROM read_states WHERE user_id = $1",
        user_id as i64
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| v1::ReadState {
        channel_id: r.channel_id as u64,
        last_read_message_id: r.last_read_message_id as u64,
        mention_count: r.mention_count as u32,
    })
    .collect())
}

// ------------------------------------------------------------------ permissions

/// Everything needed to compute any member's permissions in a guild, minus
/// the member's own roles. Cheap to keep per guild in memory.
#[derive(Debug, Clone, Default)]
pub struct GuildSnapshot {
    pub guild_id: u64,
    pub owner_id: u64,
    pub everyone: Permissions,
    /// role id → (position, permissions)
    pub roles: HashMap<u64, (i32, Permissions)>,
    /// channel id → (overwrites, parent id, type)
    pub channels: HashMap<u64, (Vec<v1::PermissionOverwrite>, u64, i32)>,
}

impl GuildSnapshot {
    pub fn from_parts(
        guild_id: u64,
        owner_id: u64,
        roles: &[v1::Role],
        channels: &[v1::Channel],
    ) -> Self {
        let mut s = Self {
            guild_id,
            owner_id,
            ..Default::default()
        };
        for r in roles {
            if r.id == guild_id {
                s.everyone = Permissions::raw(r.permissions);
            } else {
                s.roles
                    .insert(r.id, (r.position, Permissions::raw(r.permissions)));
            }
        }
        for c in channels {
            s.channels
                .insert(c.id, (c.overwrites.clone(), c.parent_id, c.r#type));
        }
        s
    }

    pub async fn load(db: &Db, guild_id: u64) -> sqlx::Result<Option<Self>> {
        let owner =
            sqlx::query_scalar!("SELECT owner_id FROM guilds WHERE id = $1", guild_id as i64)
                .fetch_optional(db)
                .await?;
        let Some(owner) = owner else { return Ok(None) };
        let roles = roles(db, guild_id).await?;
        // Include archived threads: permissions must be computable for any channel.
        let rows = sqlx::query!(
            "SELECT id, parent_id, type FROM channels WHERE guild_id = $1",
            guild_id as i64
        )
        .fetch_all(db)
        .await?;
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let mut ow = overwrites_for(db, &ids).await?;
        let mut s = Self::from_parts(guild_id, owner as u64, &roles, &[]);
        for r in rows {
            let id = r.id as u64;
            s.channels.insert(
                id,
                (
                    ow.remove(&id).unwrap_or_default(),
                    r.parent_id.unwrap_or(0) as u64,
                    i32::from(r.r#type),
                ),
            );
        }
        Ok(Some(s))
    }

    fn role_list(&self, member: &v1::Member) -> Vec<(u64, Permissions)> {
        member
            .role_ids
            .iter()
            .filter_map(|id| self.roles.get(id).map(|(_, p)| (*id, *p)))
            .collect()
    }

    fn ctx<'a>(
        &self,
        member: &v1::Member,
        roles: &'a [(u64, Permissions)],
        now: i64,
    ) -> MemberContext<'a> {
        MemberContext {
            guild_id: self.guild_id,
            guild_owner_id: self.owner_id,
            user_id: member.user.as_ref().map(|u| u.id).unwrap_or(0),
            everyone: self.everyone,
            roles,
            timeout_until: member.timeout_until,
            now,
        }
    }

    pub fn guild_perms(&self, member: &v1::Member, now: i64) -> Permissions {
        let roles = self.role_list(member);
        self.ctx(member, &roles, now).guild_permissions()
    }

    /// Channel permissions; threads inherit their parent channel's overwrites.
    pub fn channel_perms(&self, member: &v1::Member, channel_id: u64, now: i64) -> Permissions {
        let Some((ow, parent, ty)) = self.channels.get(&channel_id) else {
            return Permissions::empty();
        };
        let roles = self.role_list(member);
        let ctx = self.ctx(member, &roles, now);
        if *ty == v1::ChannelType::Thread as i32 {
            let parent_ow = self
                .channels
                .get(parent)
                .map(|c| c.0.as_slice())
                .unwrap_or(&[]);
            return ctx.channel_permissions(parent_ow);
        }
        ctx.channel_permissions(ow)
    }

    /// Highest role position of a member (0 = only @everyone).
    pub fn top_position(&self, member: &v1::Member) -> i32 {
        member
            .role_ids
            .iter()
            .filter_map(|id| self.roles.get(id).map(|r| r.0))
            .max()
            .unwrap_or(0)
    }
}

// ------------------------------------------------------------------ messages

pub struct MessageRow {
    pub channel_id: i64,
    pub id: i64,
    pub guild_id: Option<i64>,
    pub author_id: i64,
    pub content: String,
    pub kind: i16,
    pub reply_to_id: Option<i64>,
    pub edited_at: Option<DateTime<Utc>>,
    pub pinned: bool,
    pub mention_everyone: bool,
    pub mention_user_ids: Vec<i64>,
    pub mention_role_ids: Vec<i64>,
    pub attachments: serde_json::Value,
    pub embeds: serde_json::Value,
    pub nonce: Option<String>,
    pub webhook_id: Option<i64>,
    pub sticker_id: Option<i64>,
    pub thread_id: Option<i64>,
    pub e2ee_payload: Option<Vec<u8>>,
}

impl MessageRow {
    pub fn into_proto(self, author: Option<v1::User>) -> v1::Message {
        v1::Message {
            id: self.id as u64,
            channel_id: self.channel_id as u64,
            guild_id: self.guild_id.unwrap_or(0) as u64,
            author,
            content: self.content,
            edited_at: ms(self.edited_at),
            reply_to_id: self.reply_to_id.unwrap_or(0) as u64,
            attachments: serde_json::from_value(self.attachments).unwrap_or_default(),
            embeds: serde_json::from_value(self.embeds).unwrap_or_default(),
            mention_user_ids: ids(&self.mention_user_ids),
            mention_role_ids: ids(&self.mention_role_ids),
            mention_everyone: self.mention_everyone,
            pinned: self.pinned,
            reactions: vec![],
            kind: i32::from(self.kind),
            nonce: self.nonce.unwrap_or_default(),
            webhook_id: self.webhook_id.unwrap_or(0) as u64,
            e2ee_payload: self.e2ee_payload.unwrap_or_default(),
            sticker_id: self.sticker_id.unwrap_or(0) as u64,
            thread_id: self.thread_id.unwrap_or(0) as u64,
        }
    }
}

/// Attach authors and reaction counts (with `me` for `viewer`).
pub async fn hydrate(
    db: &Db,
    rows: Vec<MessageRow>,
    viewer: u64,
) -> sqlx::Result<Vec<v1::Message>> {
    let author_ids: Vec<i64> = rows.iter().map(|r| r.author_id).collect();
    let msg_ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut authors = users(db, &author_ids).await?;
    let reactions = sqlx::query!(
        r#"SELECT message_id, emoji, count(*) AS "count!", bool_or(user_id = $2) AS "me!"
           FROM reactions WHERE message_id = ANY($1)
           GROUP BY message_id, emoji ORDER BY min(created_at)"#,
        &msg_ids,
        viewer as i64
    )
    .fetch_all(db)
    .await?;
    let mut rmap: HashMap<u64, Vec<v1::ReactionCount>> = HashMap::new();
    for r in reactions {
        rmap.entry(r.message_id as u64)
            .or_default()
            .push(v1::ReactionCount {
                emoji: r.emoji,
                count: r.count as u32,
                me: r.me,
            });
    }
    Ok(rows
        .into_iter()
        .map(|r| {
            let author = authors
                .get(&(r.author_id as u64))
                .cloned()
                .or_else(|| authors.remove(&(r.author_id as u64)));
            let id = r.id as u64;
            let mut m = r.into_proto(author);
            m.reactions = rmap.remove(&id).unwrap_or_default();
            m
        })
        .collect())
}

pub async fn message(
    db: &Db,
    channel_id: u64,
    id: u64,
    viewer: u64,
) -> sqlx::Result<Option<v1::Message>> {
    let row = sqlx::query_as!(
        MessageRow,
        "SELECT channel_id, id, guild_id, author_id, content, kind, reply_to_id, edited_at, pinned, mention_everyone,
                mention_user_ids, mention_role_ids, attachments, embeds, nonce, webhook_id, sticker_id, thread_id, e2ee_payload
         FROM messages WHERE channel_id = $1 AND id = $2",
        channel_id as i64,
        id as i64
    )
    .fetch_optional(db)
    .await?;
    match row {
        Some(r) => Ok(hydrate(db, vec![r], viewer).await?.pop()),
        None => Ok(None),
    }
}

/// History page. Returns messages oldest → newest.
pub async fn messages_page(
    db: &Db,
    channel_id: u64,
    before: Option<u64>,
    after: Option<u64>,
    limit: i64,
    viewer: u64,
) -> sqlx::Result<Vec<v1::Message>> {
    let mut rows = if let Some(after) = after {
        sqlx::query_as!(
            MessageRow,
            "SELECT channel_id, id, guild_id, author_id, content, kind, reply_to_id, edited_at, pinned, mention_everyone,
                    mention_user_ids, mention_role_ids, attachments, embeds, nonce, webhook_id, sticker_id, thread_id, e2ee_payload
             FROM messages WHERE channel_id = $1 AND id > $2 ORDER BY id ASC LIMIT $3",
            channel_id as i64,
            after as i64,
            limit
        )
        .fetch_all(db)
        .await?
    } else {
        let mut r = sqlx::query_as!(
            MessageRow,
            "SELECT channel_id, id, guild_id, author_id, content, kind, reply_to_id, edited_at, pinned, mention_everyone,
                    mention_user_ids, mention_role_ids, attachments, embeds, nonce, webhook_id, sticker_id, thread_id, e2ee_payload
             FROM messages WHERE channel_id = $1 AND id < $2 ORDER BY id DESC LIMIT $3",
            channel_id as i64,
            before.map(|b| b as i64).unwrap_or(i64::MAX),
            limit
        )
        .fetch_all(db)
        .await?;
        r.reverse();
        r
    };
    rows.truncate(limit as usize);
    hydrate(db, rows, viewer).await
}

// ------------------------------------------------------------------ relationships (ADR-0021)

/// The viewer's relationships with users, newest first.
pub async fn relationships(db: &Db, user_id: u64) -> sqlx::Result<Vec<v1::Relationship>> {
    let rows = sqlx::query!(
        "SELECT peer_id, kind, created_at FROM relationships WHERE user_id = $1 ORDER BY created_at DESC",
        user_id as i64
    )
    .fetch_all(db)
    .await?;
    let ids: Vec<i64> = rows.iter().map(|r| r.peer_id).collect();
    let mut users = users(db, &ids).await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(v1::Relationship {
                user: Some(users.remove(&(r.peer_id as u64))?),
                kind: i32::from(r.kind),
                since: r.created_at.timestamp_millis(),
            })
        })
        .collect())
}

/// Accepted friends of a user.
pub async fn friend_ids(db: impl PgExecutor<'_>, user_id: u64) -> sqlx::Result<Vec<u64>> {
    Ok(sqlx::query_scalar!(
        "SELECT peer_id FROM relationships WHERE user_id = $1 AND kind = 1",
        user_id as i64
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|x| x as u64)
    .collect())
}

/// `a`'s relationship towards `b` (0 = none).
pub async fn relationship_kind(db: impl PgExecutor<'_>, a: u64, b: u64) -> sqlx::Result<i32> {
    Ok(sqlx::query_scalar!(
        "SELECT kind FROM relationships WHERE user_id = $1 AND peer_id = $2",
        a as i64,
        b as i64
    )
    .fetch_optional(db)
    .await?
    .map(i32::from)
    .unwrap_or(0))
}

/// Either side blocked the other.
pub async fn blocked_between(db: impl PgExecutor<'_>, a: u64, b: u64) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM relationships
                          WHERE kind = 2 AND ((user_id = $1 AND peer_id = $2) OR (user_id = $2 AND peer_id = $1))) AS "b!""#,
        a as i64,
        b as i64
    )
    .fetch_one(db)
    .await
}
