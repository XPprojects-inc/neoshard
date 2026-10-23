//! Publishing helpers: every mutation publishes exactly one protocol event.

use neoshard_proto::permissions::Permissions;
use neoshard_proto::v1::{self, Intent, event::Kind};
use neoshard_server_common::bus::{BusEvent, Scope};
use neoshard_server_common::repo;

use crate::AppState;

impl AppState {
    pub fn publish(&self, ev: BusEvent) {
        self.ctx.bus.publish(ev);
    }

    /// Event about a channel: guild channels go to the guild topic filtered by
    /// view permission; DM channels go to each recipient.
    pub async fn publish_channel_event(&self, channel: &v1::Channel, intent: Intent, kind: Kind) {
        if channel.guild_id != 0 {
            self.publish(BusEvent::channel(
                channel.guild_id,
                channel.id,
                intent,
                kind,
            ));
            return;
        }
        let recipients = if channel.recipient_ids.is_empty() {
            repo::recipients(&self.ctx.db, channel.id)
                .await
                .unwrap_or_default()
        } else {
            channel.recipient_ids.clone()
        };
        for uid in recipients {
            let dm_intent = if matches!(intent, Intent::GuildMessages) {
                Intent::DirectMessages
            } else {
                intent
            };
            self.publish(BusEvent::user(uid, dm_intent, kind.clone()));
        }
    }

    pub fn publish_guild(&self, guild_id: u64, intent: Intent, kind: Kind) {
        self.publish(BusEvent::guild(guild_id, intent, kind));
    }

    pub fn publish_moderation(&self, guild_id: u64, perms: Permissions, kind: Kind) {
        self.publish(
            BusEvent::guild(guild_id, Intent::GuildModeration, kind).with_scope(Scope::GuildPerm {
                perms: perms.bits(),
            }),
        );
    }

    pub fn publish_user(&self, user_id: u64, intent: Intent, kind: Kind) {
        self.publish(BusEvent::user(user_id, intent, kind));
    }

    /// Close Gateway connections of revoked sessions (all nodes) and drop cached auth here.
    pub fn revoke_sessions(
        &self,
        user_id: u64,
        session_ids: Vec<u64>,
        all: bool,
        except_session_id: u64,
    ) {
        if all {
            self.ctx.sessions.evict_user(user_id);
        }
        for s in &session_ids {
            self.ctx.sessions.evict_session(*s);
        }
        self.publish_user(
            user_id,
            Intent::None,
            Kind::SessionRevoked(v1::SessionRevoked {
                session_ids,
                all,
                except_session_id,
            }),
        );
    }
}
