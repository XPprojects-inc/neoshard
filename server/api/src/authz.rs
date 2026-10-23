//! Authorization: one check per request, through `neoshard_proto::permissions`.
//!
//! Objects the caller cannot see are reported as `404`, not `403`, so that
//! channel and guild existence does not leak.

use neoshard_proto::permissions::{Permissions, can_act_on};
use neoshard_proto::v1;
use neoshard_server_common::repo::{self, GuildSnapshot, now_ms};

use crate::AppState;
use crate::error::{ApiError, ApiResult};

/// Permissions in DMs and group DMs.
pub const DM_PERMS: Permissions = Permissions::VIEW_CHANNEL
    .union(Permissions::SEND_MESSAGES)
    .union(Permissions::READ_MESSAGE_HISTORY)
    .union(Permissions::ADD_REACTIONS)
    .union(Permissions::ATTACH_FILES)
    .union(Permissions::EMBED_LINKS)
    .union(Permissions::USE_EXTERNAL_EMOJI)
    .union(Permissions::CONNECT)
    .union(Permissions::SPEAK)
    .union(Permissions::VIDEO)
    .union(Permissions::STREAM);

pub fn require(have: Permissions, need: Permissions) -> ApiResult<()> {
    if have.contains(need) {
        Ok(())
    } else {
        Err(ApiError::MissingPermissions(need - have))
    }
}

pub struct GuildAccess {
    pub snap: GuildSnapshot,
    pub member: v1::Member,
    pub perms: Permissions,
}

impl GuildAccess {
    pub fn require(&self, need: Permissions) -> ApiResult<()> {
        require(self.perms, need)
    }

    pub fn is_owner(&self) -> bool {
        self.member.user.as_ref().map(|u| u.id) == Some(self.snap.owner_id)
    }

    pub fn top(&self) -> i32 {
        self.snap.top_position(&self.member)
    }

    /// Hierarchy check against another member of the guild.
    pub fn can_act_on(&self, target: &v1::Member) -> bool {
        let target_owner = target.user.as_ref().map(|u| u.id) == Some(self.snap.owner_id);
        can_act_on(
            self.is_owner(),
            self.top(),
            target_owner,
            self.snap.top_position(target),
        )
    }

    /// May the caller manage a role at `position`? (strictly below own top role)
    pub fn can_manage_role_at(&self, position: i32) -> bool {
        self.is_owner() || self.top() > position
    }
}

pub async fn guild_access(st: &AppState, guild_id: u64, user_id: u64) -> ApiResult<GuildAccess> {
    let snap = GuildSnapshot::load(&st.ctx.db, guild_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let member = repo::member(&st.ctx.db, guild_id, user_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let perms = snap.guild_perms(&member, now_ms());
    Ok(GuildAccess {
        snap,
        member,
        perms,
    })
}

pub struct ChannelAccess {
    pub channel: v1::Channel,
    pub perms: Permissions,
    /// `None` for DMs.
    pub guild: Option<GuildAccess>,
}

impl ChannelAccess {
    pub fn require(&self, need: Permissions) -> ApiResult<()> {
        require(self.perms, need)
    }

    pub fn guild_id(&self) -> u64 {
        self.channel.guild_id
    }

    pub fn is_dm(&self) -> bool {
        self.guild.is_none()
    }
}

pub async fn channel_access(
    st: &AppState,
    channel_id: u64,
    user_id: u64,
) -> ApiResult<ChannelAccess> {
    let channel = repo::channel(&st.ctx.db, channel_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if repo::is_dm_type(channel.r#type) {
        if !channel.recipient_ids.contains(&user_id) {
            return Err(ApiError::NotFound);
        }
        return Ok(ChannelAccess {
            channel,
            perms: DM_PERMS,
            guild: None,
        });
    }
    let ga = guild_access(st, channel.guild_id, user_id).await?;
    let perms = ga.snap.channel_perms(&ga.member, channel_id, now_ms());
    if !perms.contains(Permissions::VIEW_CHANNEL) {
        return Err(ApiError::NotFound);
    }
    Ok(ChannelAccess {
        channel,
        perms,
        guild: Some(ga),
    })
}

/// Append an audit-log entry.
pub async fn audit(
    st: &AppState,
    guild_id: u64,
    actor: u64,
    action: &str,
    target: u64,
    changes: serde_json::Value,
    reason: &str,
) -> ApiResult<()> {
    sqlx::query!(
        "INSERT INTO audit_log (id, guild_id, user_id, action, target_id, changes, reason) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        st.ctx.id() as i64,
        guild_id as i64,
        actor as i64,
        action,
        target as i64,
        changes,
        reason.chars().take(512).collect::<String>()
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(())
}
