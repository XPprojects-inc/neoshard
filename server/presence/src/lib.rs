//! Presence and voice-state registry of a node.
//!
//! Status is account-wide (the last status a client set) and a user is
//! online while at least one session is connected. `Invisible` is reported
//! to everyone else as `Offline`. Voice state: one voice channel per user.
//!
//! Single-node implementation; a cluster deployment backs this with
//! Valkey hashes (`presence:{user}`) with the same API (docs/scaling.md).
//! SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashSet;

use dashmap::DashMap;
use neoshard_proto::v1::{Presence as PresenceMsg, Status, VoiceState};

/// Custom status: text and/or emoji, optionally expiring.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Custom {
    pub text: String,
    pub emoji: String,
    /// unix millis, 0 = never
    pub expires_at: i64,
}

impl Custom {
    pub fn new(text: &str, emoji: &str, expires_at: i64) -> Self {
        Custom {
            text: text.chars().take(128).collect(),
            emoji: emoji.chars().take(64).collect(),
            expires_at: expires_at.max(0),
        }
    }

    fn active(&self, now_ms: i64) -> bool {
        self.expires_at == 0 || now_ms < self.expires_at
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
struct Entry {
    sessions: HashSet<String>,
    status: Status,
    custom: Custom,
}

#[derive(Default)]
pub struct Registry {
    users: DashMap<u64, Entry>,
    voice: DashMap<u64, VoiceState>,
}

fn public_status(s: Status) -> Status {
    if s == Status::Invisible {
        Status::Offline
    } else {
        s
    }
}

impl Registry {
    /// Presence as seen by others.
    pub fn get(&self, user_id: u64) -> PresenceMsg {
        match self.users.get(&user_id) {
            Some(e) if !e.sessions.is_empty() => {
                // Invisible users and expired statuses show nothing.
                let custom = if e.status != Status::Invisible && e.custom.active(now_ms()) {
                    e.custom.clone()
                } else {
                    Custom::default()
                };
                PresenceMsg {
                    user_id,
                    status: public_status(e.status) as i32,
                    custom_status: custom.text,
                    custom_emoji: custom.emoji,
                    custom_expires_at: custom.expires_at,
                }
            }
            _ => PresenceMsg {
                user_id,
                status: Status::Offline as i32,
                ..Default::default()
            },
        }
    }

    /// The user's own view: real status (also Invisible) and custom status,
    /// when at least one session is connected.
    pub fn get_self(&self, user_id: u64) -> Option<(Status, Custom)> {
        self.users
            .get(&user_id)
            .filter(|e| !e.sessions.is_empty())
            .map(|e| (e.status, e.custom.clone()))
    }

    pub fn is_online(&self, user_id: u64) -> bool {
        self.get(user_id).status != Status::Offline as i32
    }

    fn change<F: FnOnce(&mut Entry)>(&self, user_id: u64, f: F) -> Option<PresenceMsg> {
        let before = self.get(user_id);
        {
            let mut e = self.users.entry(user_id).or_insert_with(|| Entry {
                sessions: HashSet::new(),
                status: Status::Online,
                custom: Custom::default(),
            });
            f(&mut e);
        }
        let after = self.get(user_id);
        if self
            .users
            .get(&user_id)
            .is_some_and(|e| e.sessions.is_empty())
        {
            self.users.remove(&user_id);
        }
        (before != after).then_some(after)
    }

    /// A session connected. Returns the new public presence if it changed.
    /// `custom` is the stored custom status, applied when the first session connects.
    pub fn connect(
        &self,
        user_id: u64,
        session: &str,
        initial: Status,
        custom: &Custom,
    ) -> Option<PresenceMsg> {
        self.change(user_id, |e| {
            if e.sessions.is_empty() {
                if initial != Status::Offline {
                    e.status = initial;
                }
                e.custom = custom.clone();
            }
            e.sessions.insert(session.to_string());
        })
    }

    pub fn disconnect(&self, user_id: u64, session: &str) -> Option<PresenceMsg> {
        self.change(user_id, |e| {
            e.sessions.remove(session);
        })
    }

    pub fn set(&self, user_id: u64, status: Status, custom: &Custom) -> Option<PresenceMsg> {
        self.change(user_id, |e| {
            if !e.sessions.is_empty() {
                e.status = if status == Status::Offline {
                    Status::Invisible
                } else {
                    status
                };
                e.custom = custom.clone();
            }
        })
    }

    pub fn online_count(&self) -> usize {
        self.users.len()
    }

    // ---------------------------------------------------------------- voice

    /// Set (channel_id != 0) or clear the user's voice state. Returns the
    /// previous state if the user moved or left a channel.
    pub fn voice_set(&self, state: VoiceState) -> Option<VoiceState> {
        if state.channel_id == 0 {
            return self.voice.remove(&state.user_id).map(|(_, v)| v);
        }
        self.voice.insert(state.user_id, state)
    }

    pub fn voice_of(&self, user_id: u64) -> Option<VoiceState> {
        self.voice.get(&user_id).map(|v| *v)
    }

    pub fn voice_in_guild(&self, guild_id: u64) -> Vec<VoiceState> {
        self.voice
            .iter()
            .filter(|v| v.guild_id == guild_id)
            .map(|v| *v)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_while_any_session() {
        let r = Registry::default();
        assert_eq!(r.get(1).status, Status::Offline as i32);
        assert_eq!(
            r.connect(1, "a", Status::Online, &Custom::default())
                .unwrap()
                .status,
            Status::Online as i32
        );
        assert!(
            r.connect(1, "b", Status::Online, &Custom::default())
                .is_none(),
            "second session changes nothing"
        );
        assert!(r.disconnect(1, "a").is_none());
        assert_eq!(r.disconnect(1, "b").unwrap().status, Status::Offline as i32);
        assert_eq!(r.online_count(), 0);
    }

    #[test]
    fn invisible_looks_offline() {
        let r = Registry::default();
        r.connect(1, "a", Status::Online, &Custom::default());
        let p = r
            .set(1, Status::Invisible, &Custom::new("secret", "", 0))
            .unwrap();
        assert_eq!(p.status, Status::Offline as i32);
        assert_eq!(p.custom_status, "");
        assert!(!r.is_online(1));
        assert_eq!(
            r.set(1, Status::Dnd, &Custom::new("busy", "🛠", 0))
                .unwrap()
                .status,
            Status::Dnd as i32
        );
        assert_eq!(r.get(1).custom_status, "busy");
        assert_eq!(r.get(1).custom_emoji, "🛠");
    }

    #[test]
    fn set_without_session_is_noop() {
        let r = Registry::default();
        assert!(r.set(5, Status::Online, &Custom::default()).is_none());
        assert_eq!(r.online_count(), 0);
    }

    #[test]
    fn expired_custom_status_is_hidden() {
        let r = Registry::default();
        r.connect(1, "a", Status::Online, &Custom::new("old", "", 1));
        assert_eq!(
            r.get(1).custom_status,
            "",
            "expired at 1 ms after the epoch"
        );
        r.set(
            1,
            Status::Online,
            &Custom::new("later", "", now_ms() + 60_000),
        );
        assert_eq!(r.get(1).custom_status, "later");
        // A stored status is restored when the user comes back online.
        r.disconnect(1, "a");
        r.connect(1, "b", Status::Online, &Custom::new("stored", "x", 0));
        assert_eq!(r.get(1).custom_emoji, "x");
    }

    #[test]
    fn voice() {
        let r = Registry::default();
        let s = VoiceState {
            guild_id: 1,
            channel_id: 10,
            user_id: 7,
            ..Default::default()
        };
        assert!(r.voice_set(s).is_none());
        let moved = VoiceState {
            channel_id: 11,
            ..s
        };
        assert_eq!(r.voice_set(moved).unwrap().channel_id, 10);
        assert_eq!(r.voice_in_guild(1).len(), 1);
        let leave = VoiceState { channel_id: 0, ..s };
        assert_eq!(r.voice_set(leave).unwrap().channel_id, 11);
        assert!(r.voice_of(7).is_none());
    }
}
