//! Permission bits and **the** permission algorithm.
//!
//! This module is the only place permissions are computed — the server uses it
//! to authorize requests and `neoshard-core` uses it to decide what the UI
//! shows. Bit values are NeoShard's own and part of protocol v1
//! (`spec/permissions.md`); they are append-only.

use bitflags::bitflags;

use crate::v1::{OverwriteType, PermissionOverwrite};

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Permissions: u64 {
        // --- text (bits 0..15)
        const VIEW_CHANNEL         = 1 << 0;
        const SEND_MESSAGES        = 1 << 1;
        const READ_MESSAGE_HISTORY = 1 << 2;
        const ADD_REACTIONS        = 1 << 3;
        const ATTACH_FILES         = 1 << 4;
        const EMBED_LINKS          = 1 << 5;
        const MENTION_EVERYONE     = 1 << 6;
        const MANAGE_MESSAGES      = 1 << 7;
        const CREATE_THREADS       = 1 << 8;
        const SEND_IN_THREADS      = 1 << 9;
        const USE_EXTERNAL_EMOJI   = 1 << 10;
        const USE_COMMANDS         = 1 << 11;
        const MANAGE_THREADS       = 1 << 12;
        // --- voice (bits 16..31)
        const CONNECT              = 1 << 16;
        const SPEAK                = 1 << 17;
        const VIDEO                = 1 << 18;
        const STREAM               = 1 << 19;
        const MUTE_MEMBERS         = 1 << 20;
        const DEAFEN_MEMBERS       = 1 << 21;
        const MOVE_MEMBERS         = 1 << 22;
        const PRIORITY_SPEAKER     = 1 << 23;
        // --- membership (bits 32..39)
        const CREATE_INVITE        = 1 << 32;
        const CHANGE_NICKNAME      = 1 << 33;
        const MANAGE_NICKNAMES     = 1 << 34;
        const KICK_MEMBERS         = 1 << 35;
        const BAN_MEMBERS          = 1 << 36;
        const MODERATE_MEMBERS     = 1 << 37;
        // --- management (bits 40..)
        const MANAGE_CHANNELS      = 1 << 40;
        const MANAGE_ROLES         = 1 << 41;
        const MANAGE_GUILD         = 1 << 42;
        const MANAGE_WEBHOOKS      = 1 << 43;
        const MANAGE_EMOJIS        = 1 << 44;
        const VIEW_AUDIT_LOG       = 1 << 45;
        const ADMINISTRATOR        = 1 << 63;
    }
}

impl Permissions {
    /// Default permissions of the `@everyone` role in a new guild.
    pub const DEFAULT_EVERYONE: Self = Self::VIEW_CHANNEL
        .union(Self::SEND_MESSAGES)
        .union(Self::READ_MESSAGE_HISTORY)
        .union(Self::ADD_REACTIONS)
        .union(Self::ATTACH_FILES)
        .union(Self::EMBED_LINKS)
        .union(Self::CREATE_THREADS)
        .union(Self::SEND_IN_THREADS)
        .union(Self::USE_EXTERNAL_EMOJI)
        .union(Self::USE_COMMANDS)
        .union(Self::CONNECT)
        .union(Self::SPEAK)
        .union(Self::VIDEO)
        .union(Self::STREAM)
        .union(Self::CREATE_INVITE)
        .union(Self::CHANGE_NICKNAME);

    /// What a timed-out member keeps.
    pub const TIMEOUT_ALLOWED: Self = Self::VIEW_CHANNEL.union(Self::READ_MESSAGE_HISTORY);

    /// Bits that only make sense on a channel (can appear in overwrites).
    pub const CHANNEL_SCOPED: Self = Self::from_bits_truncate(0x0000_0000_FFFF_FFFF)
        .union(Self::CREATE_INVITE)
        .union(Self::MANAGE_CHANNELS)
        .union(Self::MANAGE_ROLES)
        .union(Self::MANAGE_WEBHOOKS);

    /// Build from raw bits, keeping unknown bits (forward compatibility).
    pub const fn raw(bits: u64) -> Self {
        Self::from_bits_retain(bits)
    }
}

/// Everything needed to compute a member's permissions.
#[derive(Debug, Clone)]
pub struct MemberContext<'a> {
    pub guild_id: u64,
    pub guild_owner_id: u64,
    pub user_id: u64,
    /// Permissions of the `@everyone` role (id == guild id).
    pub everyone: Permissions,
    /// `(role_id, permissions)` for each role of the member (excluding @everyone).
    pub roles: &'a [(u64, Permissions)],
    /// Unix millis until which the member is timed out (0 = none).
    pub timeout_until: i64,
    /// Current unix millis.
    pub now: i64,
}

impl MemberContext<'_> {
    fn is_owner(&self) -> bool {
        self.user_id == self.guild_owner_id
    }

    fn timed_out(&self) -> bool {
        self.timeout_until > self.now
    }

    /// Guild-level permissions (no channel overwrites applied).
    pub fn guild_permissions(&self) -> Permissions {
        if self.is_owner() {
            return Permissions::all();
        }
        let mut p = self.everyone;
        for (_, rp) in self.roles {
            p |= *rp;
        }
        if p.contains(Permissions::ADMINISTRATOR) {
            return Permissions::all();
        }
        if self.timed_out() {
            p &= Permissions::TIMEOUT_ALLOWED;
        }
        p
    }

    /// Effective permissions in a channel with the given overwrites.
    pub fn channel_permissions(&self, overwrites: &[PermissionOverwrite]) -> Permissions {
        if self.is_owner() {
            return Permissions::all();
        }
        let mut base = self.everyone;
        for (_, rp) in self.roles {
            base |= *rp;
        }
        if base.contains(Permissions::ADMINISTRATOR) {
            return Permissions::all();
        }

        let mut p = base;
        let is_role = |o: &PermissionOverwrite| o.r#type == OverwriteType::Role as i32;

        // 1. @everyone overwrite
        if let Some(o) = overwrites
            .iter()
            .find(|o| is_role(o) && o.id == self.guild_id)
        {
            p = (p & !Permissions::raw(o.deny)) | Permissions::raw(o.allow);
        }
        // 2. all role overwrites of the member, aggregated then applied
        let (mut allow, mut deny) = (Permissions::empty(), Permissions::empty());
        for o in overwrites
            .iter()
            .filter(|o| is_role(o) && o.id != self.guild_id)
        {
            if self.roles.iter().any(|(rid, _)| *rid == o.id) {
                allow |= Permissions::raw(o.allow);
                deny |= Permissions::raw(o.deny);
            }
        }
        p = (p & !deny) | allow;
        // 3. member overwrite
        if let Some(o) = overwrites
            .iter()
            .find(|o| !is_role(o) && o.id == self.user_id)
        {
            p = (p & !Permissions::raw(o.deny)) | Permissions::raw(o.allow);
        }

        // Implicit rules.
        if !p.contains(Permissions::VIEW_CHANNEL) {
            return Permissions::empty();
        }
        if !p.contains(Permissions::SEND_MESSAGES) {
            p -= Permissions::MENTION_EVERYONE
                | Permissions::ATTACH_FILES
                | Permissions::EMBED_LINKS;
        }
        if self.timed_out() {
            p &= Permissions::TIMEOUT_ALLOWED;
        }
        p
    }
}

/// Role hierarchy check: may an actor whose highest role position is
/// `actor_top` (and who is/isn't the owner) act on a target whose highest
/// position is `target_top`? Equal positions are NOT allowed.
pub fn can_act_on(
    actor_is_owner: bool,
    actor_top: i32,
    target_is_owner: bool,
    target_top: i32,
) -> bool {
    if target_is_owner {
        return false;
    }
    actor_is_owner || actor_top > target_top
}

/// May an actor with guild permissions `actor` grant `requested` to a role?
/// Non-admins can only grant bits they hold themselves.
pub fn can_grant(actor: Permissions, requested: Permissions) -> bool {
    actor.contains(Permissions::ADMINISTRATOR) || actor.contains(requested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Permissions as P;

    const G: u64 = 100;
    const OWNER: u64 = 1;
    const U: u64 = 2;
    const MOD_ROLE: u64 = 10;
    const MUTED_ROLE: u64 = 11;

    fn ow(id: u64, t: OverwriteType, allow: P, deny: P) -> PermissionOverwrite {
        PermissionOverwrite {
            id,
            r#type: t as i32,
            allow: allow.bits(),
            deny: deny.bits(),
        }
    }

    fn ctx<'a>(user: u64, roles: &'a [(u64, P)]) -> MemberContext<'a> {
        MemberContext {
            guild_id: G,
            guild_owner_id: OWNER,
            user_id: user,
            everyone: P::DEFAULT_EVERYONE,
            roles,
            timeout_until: 0,
            now: 1_000,
        }
    }

    #[test]
    fn owner_has_everything() {
        let c = ctx(OWNER, &[]);
        assert_eq!(c.guild_permissions(), P::all());
        let deny_all = [ow(G, OverwriteType::Role, P::empty(), P::all())];
        assert_eq!(c.channel_permissions(&deny_all), P::all());
    }

    #[test]
    fn administrator_ignores_overwrites() {
        let roles = [(MOD_ROLE, P::ADMINISTRATOR)];
        let c = ctx(U, &roles);
        let deny = [ow(U, OverwriteType::Member, P::empty(), P::VIEW_CHANNEL)];
        assert_eq!(c.channel_permissions(&deny), P::all());
    }

    #[test]
    fn roles_are_ored() {
        let roles = [(MOD_ROLE, P::KICK_MEMBERS), (MUTED_ROLE, P::BAN_MEMBERS)];
        let p = ctx(U, &roles).guild_permissions();
        assert!(p.contains(P::KICK_MEMBERS | P::BAN_MEMBERS | P::SEND_MESSAGES));
    }

    #[test]
    fn everyone_overwrite_then_roles_then_member() {
        let roles = [(MOD_ROLE, P::empty())];
        let c = ctx(U, &roles);
        // @everyone cannot send; mod role can; member explicitly denied again.
        let mut ows = vec![
            ow(G, OverwriteType::Role, P::empty(), P::SEND_MESSAGES),
            ow(MOD_ROLE, OverwriteType::Role, P::SEND_MESSAGES, P::empty()),
        ];
        assert!(c.channel_permissions(&ows).contains(P::SEND_MESSAGES));
        ows.push(ow(U, OverwriteType::Member, P::empty(), P::SEND_MESSAGES));
        assert!(!c.channel_permissions(&ows).contains(P::SEND_MESSAGES));
    }

    #[test]
    fn role_allow_wins_over_role_deny_after_aggregation() {
        // One role denies, another allows the same bit: allow is applied after deny.
        let roles = [(MOD_ROLE, P::empty()), (MUTED_ROLE, P::empty())];
        let ows = [
            ow(
                MUTED_ROLE,
                OverwriteType::Role,
                P::empty(),
                P::SEND_MESSAGES,
            ),
            ow(MOD_ROLE, OverwriteType::Role, P::SEND_MESSAGES, P::empty()),
        ];
        assert!(
            ctx(U, &roles)
                .channel_permissions(&ows)
                .contains(P::SEND_MESSAGES)
        );
    }

    #[test]
    fn overwrites_for_roles_member_lacks_are_ignored() {
        let ows = [ow(
            MOD_ROLE,
            OverwriteType::Role,
            P::empty(),
            P::SEND_MESSAGES,
        )];
        assert!(
            ctx(U, &[])
                .channel_permissions(&ows)
                .contains(P::SEND_MESSAGES)
        );
    }

    #[test]
    fn no_view_means_nothing() {
        let ows = [ow(
            G,
            OverwriteType::Role,
            P::SEND_MESSAGES,
            P::VIEW_CHANNEL,
        )];
        assert_eq!(ctx(U, &[]).channel_permissions(&ows), P::empty());
    }

    #[test]
    fn no_send_strips_send_dependent_bits() {
        let roles = [(MOD_ROLE, P::MENTION_EVERYONE)];
        let ows = [ow(G, OverwriteType::Role, P::empty(), P::SEND_MESSAGES)];
        let p = ctx(U, &roles).channel_permissions(&ows);
        assert!(p.contains(P::VIEW_CHANNEL));
        assert!(
            !p.intersects(
                P::SEND_MESSAGES | P::MENTION_EVERYONE | P::ATTACH_FILES | P::EMBED_LINKS
            )
        );
    }

    #[test]
    fn timeout_keeps_only_read() {
        let roles = [(MOD_ROLE, P::KICK_MEMBERS)];
        let mut c = ctx(U, &roles);
        c.timeout_until = 2_000;
        assert_eq!(c.guild_permissions(), P::TIMEOUT_ALLOWED);
        assert_eq!(c.channel_permissions(&[]), P::TIMEOUT_ALLOWED);
        c.now = 3_000; // expired
        assert!(c.channel_permissions(&[]).contains(P::SEND_MESSAGES));
    }

    #[test]
    fn timeout_does_not_affect_admin() {
        let roles = [(MOD_ROLE, P::ADMINISTRATOR)];
        let mut c = ctx(U, &roles);
        c.timeout_until = i64::MAX;
        assert_eq!(c.guild_permissions(), P::all());
    }

    #[test]
    fn unknown_bits_survive() {
        let p = P::raw(1 << 60);
        assert_eq!(p.bits(), 1 << 60);
    }

    #[test]
    fn hierarchy() {
        assert!(can_act_on(false, 5, false, 3));
        assert!(!can_act_on(false, 3, false, 3));
        assert!(!can_act_on(false, 2, false, 3));
        assert!(can_act_on(true, 0, false, 99));
        assert!(!can_act_on(true, 99, true, 0));
    }

    #[test]
    fn grant_only_what_you_have() {
        let me = P::MANAGE_ROLES | P::KICK_MEMBERS;
        assert!(can_grant(me, P::KICK_MEMBERS));
        assert!(!can_grant(me, P::BAN_MEMBERS));
        assert!(can_grant(P::ADMINISTRATOR, P::all()));
    }
}
