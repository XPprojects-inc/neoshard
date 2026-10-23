//! Client-side state cache. Applies server events, answers history from
//! memory, computes permissions, and snapshots itself for offline start.

use std::collections::{BTreeMap, HashMap, VecDeque};

use neoshard_proto::core_api::{CacheSnapshot, History};
use neoshard_proto::permissions::{MemberContext, Permissions};
use neoshard_proto::v1::{self, event::Kind};

/// Messages kept in memory per channel (the UI virtualizes; older pages are refetched).
pub const MESSAGES_PER_CHANNEL: usize = 1000;
/// Messages per channel persisted for offline start.
pub const SNAPSHOT_MESSAGES: usize = 50;

#[derive(Default)]
pub struct Cache {
    pub me: Option<v1::User>,
    pub guilds: BTreeMap<u64, v1::Guild>,
    pub private_channels: BTreeMap<u64, v1::Channel>,
    pub read_states: HashMap<u64, v1::ReadState>,
    pub presences: HashMap<u64, v1::Presence>,
    pub messages: HashMap<u64, VecDeque<v1::Message>>,
    /// Channels whose oldest message is known to be the first one.
    pub reached_start: HashMap<u64, bool>,
    pub users: HashMap<u64, v1::User>,
}

fn insert_sorted(list: &mut VecDeque<v1::Message>, m: v1::Message) {
    // Fast path: newest message.
    match list.back() {
        None => list.push_back(m),
        Some(last) if last.id < m.id => list.push_back(m),
        _ => match list.binary_search_by_key(&m.id, |x| x.id) {
            Ok(i) => list[i] = m,
            Err(i) => list.insert(i, m),
        },
    }
}

impl Cache {
    pub fn load_ready(&mut self, r: &v1::Ready) {
        self.me = r.user.clone();
        self.guilds = r.guilds.iter().map(|g| (g.id, g.clone())).collect();
        self.private_channels = r
            .private_channels
            .iter()
            .map(|c| (c.id, c.clone()))
            .collect();
        self.read_states = r.read_states.iter().map(|s| (s.channel_id, *s)).collect();
        self.presences = r.presences.iter().map(|p| (p.user_id, p.clone())).collect();
        // Keep cached messages: after reconnect they are still valid history.
    }

    pub fn me_id(&self) -> u64 {
        self.me.as_ref().map(|u| u.id).unwrap_or(0)
    }

    pub fn channel(&self, id: u64) -> Option<&v1::Channel> {
        self.private_channels.get(&id).or_else(|| {
            self.guilds
                .values()
                .flat_map(|g| g.channels.iter())
                .find(|c| c.id == id)
        })
    }

    fn channel_mut(&mut self, guild_id: u64, id: u64) -> Option<&mut v1::Channel> {
        if guild_id == 0 {
            return self.private_channels.get_mut(&id);
        }
        self.guilds
            .get_mut(&guild_id)?
            .channels
            .iter_mut()
            .find(|c| c.id == id)
    }

    fn message_mut(&mut self, channel_id: u64, id: u64) -> Option<&mut v1::Message> {
        let list = self.messages.get_mut(&channel_id)?;
        let i = list.binary_search_by_key(&id, |m| m.id).ok()?;
        list.get_mut(i)
    }

    pub fn put_message(&mut self, m: v1::Message) {
        if let Some(a) = &m.author {
            self.users.insert(a.id, a.clone());
        }
        let cid = m.channel_id;
        let list = self.messages.entry(cid).or_default();
        insert_sorted(list, m);
        let mut trimmed = false;
        while list.len() > MESSAGES_PER_CHANNEL {
            list.pop_front();
            trimmed = true;
        }
        if trimmed {
            // The oldest messages are gone from memory: history must be refetched.
            self.reached_start.remove(&cid);
        }
    }

    /// Apply a server event. Returns false if the event is irrelevant (unknown guild etc.).
    pub fn apply(&mut self, ev: &v1::Event) -> bool {
        let Some(kind) = &ev.kind else { return false };
        let me = self.me_id();
        match kind {
            Kind::GuildCreate(g) => {
                self.guilds.insert(g.id, g.clone());
            }
            Kind::GuildUpdate(g) => {
                if let Some(cur) = self.guilds.get_mut(&g.id) {
                    cur.name = g.name.clone();
                    cur.icon = g.icon.clone();
                    cur.owner_id = g.owner_id;
                    cur.emojis = g.emojis.clone();
                    cur.stickers = g.stickers.clone();
                    if g.member_count > 0 {
                        cur.member_count = g.member_count;
                    }
                }
            }
            Kind::GuildDelete(d) => {
                if let Some(g) = self.guilds.remove(&d.id) {
                    for c in g.channels {
                        self.messages.remove(&c.id);
                    }
                }
            }
            Kind::ChannelCreate(c) | Kind::ChannelUpdate(c) => {
                if c.guild_id == 0 {
                    self.private_channels.insert(c.id, c.clone());
                } else if let Some(g) = self.guilds.get_mut(&c.guild_id) {
                    match g.channels.iter_mut().find(|x| x.id == c.id) {
                        Some(x) => *x = c.clone(),
                        None => g.channels.push(c.clone()),
                    }
                    g.channels.sort_by_key(|x| (x.position, x.id));
                } else {
                    return false;
                }
            }
            Kind::ChannelDelete(d) => {
                if d.guild_id == 0 {
                    self.private_channels.remove(&d.id);
                } else if let Some(g) = self.guilds.get_mut(&d.guild_id) {
                    g.channels.retain(|c| c.id != d.id);
                }
                self.messages.remove(&d.id);
            }
            Kind::RoleCreate(r) | Kind::RoleUpdate(r) => {
                let Some(g) = self.guilds.get_mut(&r.guild_id) else {
                    return false;
                };
                match g.roles.iter_mut().find(|x| x.id == r.id) {
                    Some(x) => *x = r.clone(),
                    None => g.roles.push(r.clone()),
                }
                g.roles.sort_by_key(|x| (x.position, x.id));
            }
            Kind::RoleDelete(d) => {
                let Some(g) = self.guilds.get_mut(&d.guild_id) else {
                    return false;
                };
                g.roles.retain(|r| r.id != d.id);
                if let Some(m) = g.self_member.as_mut() {
                    m.role_ids.retain(|r| *r != d.id);
                }
            }
            Kind::MemberAdd(m) => {
                if let Some(g) = self.guilds.get_mut(&m.guild_id) {
                    g.member_count += 1;
                }
                if let Some(u) = &m.user {
                    self.users.insert(u.id, u.clone());
                }
            }
            Kind::MemberUpdate(m) => {
                if m.user.as_ref().map(|u| u.id) == Some(me)
                    && let Some(g) = self.guilds.get_mut(&m.guild_id)
                {
                    g.self_member = Some(m.clone());
                }
                if let Some(u) = &m.user {
                    self.users.insert(u.id, u.clone());
                }
            }
            Kind::MemberRemove(r) => {
                if r.user_id == me {
                    self.guilds.remove(&r.guild_id);
                } else if let Some(g) = self.guilds.get_mut(&r.guild_id) {
                    g.member_count = g.member_count.saturating_sub(1);
                }
            }
            Kind::MessageCreate(m) => {
                let (gid, cid, mid) = (m.guild_id, m.channel_id, m.id);
                self.put_message(m.clone());
                if let Some(c) = self.channel_mut(gid, cid) {
                    c.last_message_id = c.last_message_id.max(mid);
                }
                if m.author.as_ref().map(|a| a.id) == Some(me) {
                    self.read_states.insert(
                        cid,
                        v1::ReadState {
                            channel_id: cid,
                            last_read_message_id: mid,
                            mention_count: 0,
                        },
                    );
                }
            }
            Kind::MessageUpdate(m) => {
                if let Some(cur) = self.message_mut(m.channel_id, m.id) {
                    // Updates may be partial for bots without content intent; keep reactions we track.
                    let reactions = std::mem::take(&mut cur.reactions);
                    *cur = m.clone();
                    if cur.reactions.is_empty() {
                        cur.reactions = reactions;
                    }
                }
            }
            Kind::MessageDelete(d) => {
                if let Some(list) = self.messages.get_mut(&d.channel_id) {
                    list.retain(|m| m.id != d.id);
                }
            }
            Kind::MessageDeleteBulk(d) => {
                if let Some(list) = self.messages.get_mut(&d.channel_id) {
                    list.retain(|m| !d.ids.contains(&m.id));
                }
            }
            Kind::ReactionAdd(r) | Kind::ReactionRemove(r) => {
                let add = matches!(kind, Kind::ReactionAdd(_));
                let mine = r.user_id == me;
                if let Some(m) = self.message_mut(r.channel_id, r.message_id) {
                    match m.reactions.iter_mut().position(|x| x.emoji == r.emoji) {
                        Some(i) => {
                            let x = &mut m.reactions[i];
                            if add {
                                x.count += 1;
                                x.me |= mine;
                            } else {
                                x.count = x.count.saturating_sub(1);
                                if mine {
                                    x.me = false;
                                }
                                if x.count == 0 {
                                    m.reactions.remove(i);
                                }
                            }
                        }
                        None if add => m.reactions.push(v1::ReactionCount {
                            emoji: r.emoji.clone(),
                            count: 1,
                            me: mine,
                        }),
                        None => {}
                    }
                }
            }
            Kind::PresenceUpdate(p) => {
                self.presences.insert(p.user_id, p.clone());
            }
            Kind::ReadStateUpdate(r) => {
                self.read_states.insert(r.channel_id, *r);
            }
            Kind::UserUpdate(u) => {
                if u.id == me {
                    self.me = Some(u.clone());
                }
                self.users.insert(u.id, u.clone());
            }
            Kind::VoiceStateUpdate(v) => {
                if let Some(g) = self.guilds.get_mut(&v.guild_id) {
                    g.voice_states.retain(|x| x.user_id != v.user_id);
                    if v.channel_id != 0 {
                        g.voice_states.push(*v);
                    }
                }
            }
            Kind::MemberListChunk(c) => {
                for m in &c.members {
                    if let Some(u) = &m.user {
                        self.users.insert(u.id, u.clone());
                    }
                }
                for p in &c.presences {
                    self.presences.insert(p.user_id, p.clone());
                }
            }
            _ => {}
        }
        true
    }

    /// Merge a fetched history page.
    pub fn merge_history(&mut self, channel_id: u64, page: Vec<v1::Message>, requested: usize) {
        let full = page.len() >= requested;
        for m in page {
            self.put_message(m);
        }
        if !full {
            self.reached_start.insert(channel_id, true);
        }
    }

    /// Messages older than `before` (0 = latest), newest last, if the cache can answer.
    pub fn history(&self, channel_id: u64, before: u64, limit: usize) -> Option<History> {
        let list = self.messages.get(&channel_id)?;
        let older: Vec<&v1::Message> = list
            .iter()
            .filter(|m| before == 0 || m.id < before)
            .collect();
        let reached = self
            .reached_start
            .get(&channel_id)
            .copied()
            .unwrap_or(false);
        if older.len() < limit && !reached {
            return None;
        }
        let start = older.len().saturating_sub(limit);
        let messages: Vec<v1::Message> = older[start..].iter().map(|m| (*m).clone()).collect();
        Some(History {
            channel_id,
            reached_start: reached && start == 0,
            messages,
        })
    }

    pub fn unread(&self, channel_id: u64) -> bool {
        let last = self
            .channel(channel_id)
            .map(|c| c.last_message_id)
            .unwrap_or(0);
        let read = self
            .read_states
            .get(&channel_id)
            .map(|r| r.last_read_message_id)
            .unwrap_or(0);
        last > read
    }

    /// Effective permissions of the current user in a channel.
    pub fn permissions(&self, channel_id: u64, now_ms: i64) -> Permissions {
        if self.private_channels.contains_key(&channel_id) {
            return Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::READ_MESSAGE_HISTORY
                | Permissions::ADD_REACTIONS
                | Permissions::ATTACH_FILES
                | Permissions::EMBED_LINKS
                | Permissions::CONNECT
                | Permissions::SPEAK
                | Permissions::VIDEO
                | Permissions::STREAM;
        }
        let Some(g) = self
            .guilds
            .values()
            .find(|g| g.channels.iter().any(|c| c.id == channel_id))
        else {
            return Permissions::empty();
        };
        let Some(member) = &g.self_member else {
            return Permissions::empty();
        };
        let everyone = g
            .roles
            .iter()
            .find(|r| r.id == g.id)
            .map(|r| Permissions::raw(r.permissions))
            .unwrap_or_default();
        let roles: Vec<(u64, Permissions)> = g
            .roles
            .iter()
            .filter(|r| member.role_ids.contains(&r.id))
            .map(|r| (r.id, Permissions::raw(r.permissions)))
            .collect();
        let ctx = MemberContext {
            guild_id: g.id,
            guild_owner_id: g.owner_id,
            user_id: self.me_id(),
            everyone,
            roles: &roles,
            timeout_until: member.timeout_until,
            now: now_ms,
        };
        let ch = g
            .channels
            .iter()
            .find(|c| c.id == channel_id)
            .expect("found above");
        if ch.r#type == v1::ChannelType::Thread as i32 {
            let parent = g
                .channels
                .iter()
                .find(|c| c.id == ch.parent_id)
                .map(|c| c.overwrites.as_slice())
                .unwrap_or(&[]);
            return ctx.channel_permissions(parent);
        }
        ctx.channel_permissions(&ch.overwrites)
    }

    pub fn snapshot(&self, api_base: &str, gateway_url: &str, token: &str) -> CacheSnapshot {
        let ready = v1::Ready {
            user: self.me.clone(),
            guilds: self.guilds.values().cloned().collect(),
            private_channels: self.private_channels.values().cloned().collect(),
            read_states: self.read_states.values().cloned().collect(),
            ..Default::default()
        };
        let histories = self
            .messages
            .iter()
            .map(|(cid, list)| {
                let start = list.len().saturating_sub(SNAPSHOT_MESSAGES);
                History {
                    channel_id: *cid,
                    messages: list.iter().skip(start).cloned().collect(),
                    reached_start: false,
                }
            })
            .collect();
        CacheSnapshot {
            ready: Some(ready),
            histories,
            api_base: api_base.into(),
            gateway_url: gateway_url.into(),
            token: token.into(),
            mls_state: Vec::new(),
        }
    }

    pub fn restore(&mut self, s: &CacheSnapshot) {
        if let Some(r) = &s.ready {
            self.load_ready(r);
        }
        for h in &s.histories {
            for m in &h.messages {
                self.put_message(m.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: u64, cid: u64) -> v1::Message {
        v1::Message {
            id,
            channel_id: cid,
            guild_id: 1,
            content: format!("m{id}"),
            ..Default::default()
        }
    }

    fn ev(k: Kind) -> v1::Event {
        v1::Event { kind: Some(k) }
    }

    fn guild() -> v1::Guild {
        v1::Guild {
            id: 1,
            owner_id: 99,
            channels: vec![v1::Channel {
                id: 10,
                guild_id: 1,
                r#type: 1,
                ..Default::default()
            }],
            roles: vec![v1::Role {
                id: 1,
                guild_id: 1,
                permissions: Permissions::DEFAULT_EVERYONE.bits(),
                ..Default::default()
            }],
            self_member: Some(v1::Member {
                guild_id: 1,
                user: Some(v1::User {
                    id: 5,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn cache() -> Cache {
        let mut c = Cache::default();
        c.load_ready(&v1::Ready {
            user: Some(v1::User {
                id: 5,
                ..Default::default()
            }),
            guilds: vec![guild()],
            ..Default::default()
        });
        c
    }

    #[test]
    fn messages_sorted_and_deduped() {
        let mut c = cache();
        for id in [3, 1, 2, 3] {
            c.apply(&ev(Kind::MessageCreate(msg(id, 10))));
        }
        let ids: Vec<u64> = c.messages[&10].iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(c.channel(10).unwrap().last_message_id, 3);
        c.apply(&ev(Kind::MessageDelete(v1::MessageDelete {
            id: 2,
            channel_id: 10,
            guild_id: 1,
        })));
        assert_eq!(c.messages[&10].len(), 2);
    }

    #[test]
    fn reactions_count_and_me() {
        let mut c = cache();
        c.apply(&ev(Kind::MessageCreate(msg(1, 10))));
        let r = |user_id| v1::ReactionEvent {
            message_id: 1,
            channel_id: 10,
            guild_id: 1,
            user_id,
            emoji: "👍".into(),
        };
        c.apply(&ev(Kind::ReactionAdd(r(5))));
        c.apply(&ev(Kind::ReactionAdd(r(6))));
        let m = &c.messages[&10][0];
        assert_eq!((m.reactions[0].count, m.reactions[0].me), (2, true));
        c.apply(&ev(Kind::ReactionRemove(r(5))));
        c.apply(&ev(Kind::ReactionRemove(r(6))));
        assert!(c.messages[&10][0].reactions.is_empty());
    }

    #[test]
    fn history_from_cache_or_none() {
        let mut c = cache();
        for id in 1..=10 {
            c.apply(&ev(Kind::MessageCreate(msg(id, 10))));
        }
        let h = c.history(10, 0, 5).unwrap();
        assert_eq!(
            h.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![6, 7, 8, 9, 10]
        );
        assert!(
            c.history(10, 6, 10).is_none(),
            "not enough cached and start unknown"
        );
        c.merge_history(10, vec![], 50);
        let h = c.history(10, 6, 10).unwrap();
        assert_eq!(h.messages.len(), 5);
        assert!(h.reached_start);
    }

    #[test]
    fn permissions_follow_overwrites_and_roles() {
        let mut c = cache();
        assert!(c.permissions(10, 0).contains(Permissions::SEND_MESSAGES));
        let mut ch = c.channel(10).unwrap().clone();
        ch.overwrites = vec![v1::PermissionOverwrite {
            id: 1,
            r#type: 0,
            allow: 0,
            deny: Permissions::SEND_MESSAGES.bits(),
        }];
        c.apply(&ev(Kind::ChannelUpdate(ch)));
        assert!(!c.permissions(10, 0).contains(Permissions::SEND_MESSAGES));
        assert!(c.permissions(10, 0).contains(Permissions::VIEW_CHANNEL));
        assert!(c.permissions(999, 0).is_empty());
    }

    #[test]
    fn unread_tracking() {
        let mut c = cache();
        c.apply(&ev(Kind::MessageCreate(msg(7, 10))));
        assert!(c.unread(10));
        c.apply(&ev(Kind::ReadStateUpdate(v1::ReadState {
            channel_id: 10,
            last_read_message_id: 7,
            mention_count: 0,
        })));
        assert!(!c.unread(10));
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut c = cache();
        for id in 1..=80 {
            c.apply(&ev(Kind::MessageCreate(msg(id, 10))));
        }
        let snap = c.snapshot("https://x", "wss://x/gateway", "");
        let bytes = prost::Message::encode_to_vec(&snap);
        let back: CacheSnapshot = prost::Message::decode(bytes.as_slice()).unwrap();
        let mut c2 = Cache::default();
        c2.restore(&back);
        assert_eq!(c2.guilds.len(), 1);
        assert_eq!(c2.messages[&10].len(), SNAPSHOT_MESSAGES);
        assert_eq!(c2.messages[&10].back().unwrap().id, 80);
    }

    #[test]
    fn removed_from_guild() {
        let mut c = cache();
        c.apply(&ev(Kind::MemberRemove(v1::MemberRemove {
            guild_id: 1,
            user_id: 5,
        })));
        assert!(c.guilds.is_empty());
    }
}
