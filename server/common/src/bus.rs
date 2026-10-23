//! Event bus between the API (producers) and gateway nodes (consumers).
//!
//! Every gateway node receives every event and routes it to its local
//! sessions via its own topic index, so the bus itself stays dumb:
//! * [`LocalBus`] — `tokio::broadcast` in one process (single-node installs);
//! * `NatsBus` (feature `nats`) — the same stream over NATS between nodes.

use std::sync::Arc;

use neoshard_proto::v1;
use prost::Message;
use tokio::sync::broadcast;

/// Who receives an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    /// Every session subscribed to the guild (members).
    Guild(u64),
    /// Every session of one user (DMs, read states, own profile).
    User(u64),
}

/// Extra per-session filter applied by the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    All,
    /// Only sessions whose user has `perms` (bits) in this channel. `VIEW_CHANNEL` is always required.
    Channel {
        channel_id: u64,
        perms: u64,
    },
    /// Guild-level permission bits (e.g. moderation events → BAN_MEMBERS).
    GuildPerm {
        perms: u64,
    },
}

#[derive(Debug, Clone)]
pub struct BusEvent {
    pub topic: Topic,
    pub scope: Scope,
    /// Intent bit this event belongs to (gateway filters by the session's intents).
    pub intent: u32,
    pub event: v1::Event,
}

impl BusEvent {
    pub fn guild(guild_id: u64, intent: v1::Intent, event: v1::event::Kind) -> Self {
        Self {
            topic: Topic::Guild(guild_id),
            scope: Scope::All,
            intent: intent as u32,
            event: v1::Event { kind: Some(event) },
        }
    }

    pub fn channel(
        guild_id: u64,
        channel_id: u64,
        intent: v1::Intent,
        event: v1::event::Kind,
    ) -> Self {
        Self {
            topic: Topic::Guild(guild_id),
            scope: Scope::Channel {
                channel_id,
                perms: neoshard_proto::permissions::Permissions::VIEW_CHANNEL.bits(),
            },
            intent: intent as u32,
            event: v1::Event { kind: Some(event) },
        }
    }

    pub fn user(user_id: u64, intent: v1::Intent, event: v1::event::Kind) -> Self {
        Self {
            topic: Topic::User(user_id),
            scope: Scope::All,
            intent: intent as u32,
            event: v1::Event { kind: Some(event) },
        }
    }

    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// Compact binary encoding for inter-node transport.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        let (tk, tid) = match self.topic {
            Topic::Guild(id) => (0u8, id),
            Topic::User(id) => (1u8, id),
        };
        out.push(tk);
        out.extend_from_slice(&tid.to_le_bytes());
        match self.scope {
            Scope::All => out.push(0),
            Scope::Channel { channel_id, perms } => {
                out.push(1);
                out.extend_from_slice(&channel_id.to_le_bytes());
                out.extend_from_slice(&perms.to_le_bytes());
            }
            Scope::GuildPerm { perms } => {
                out.push(2);
                out.extend_from_slice(&perms.to_le_bytes());
            }
        }
        out.extend_from_slice(&self.intent.to_le_bytes());
        self.event.encode(&mut out).expect("vec grows");
        out
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        let u64_at = |i: usize| {
            b.get(i..i + 8)
                .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        };
        let topic = match b.first()? {
            0 => Topic::Guild(u64_at(1)?),
            1 => Topic::User(u64_at(1)?),
            _ => return None,
        };
        let mut i = 9;
        let scope = match b.get(i)? {
            0 => {
                i += 1;
                Scope::All
            }
            1 => {
                let s = Scope::Channel {
                    channel_id: u64_at(i + 1)?,
                    perms: u64_at(i + 9)?,
                };
                i += 17;
                s
            }
            2 => {
                let s = Scope::GuildPerm {
                    perms: u64_at(i + 1)?,
                };
                i += 9;
                s
            }
            _ => return None,
        };
        let intent = u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?);
        let event = v1::Event::decode(b.get(i + 4..)?).ok()?;
        Some(Self {
            topic,
            scope,
            intent,
            event,
        })
    }
}

pub trait EventBus: Send + Sync + 'static {
    fn publish(&self, ev: BusEvent);
    /// Stream of all events reaching this node.
    fn subscribe(&self) -> broadcast::Receiver<Arc<BusEvent>>;
}

pub type SharedBus = Arc<dyn EventBus>;

pub struct LocalBus {
    tx: broadcast::Sender<Arc<BusEvent>>,
}

impl LocalBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(65_536);
        Self { tx }
    }
}

impl Default for LocalBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus for LocalBus {
    fn publish(&self, ev: BusEvent) {
        // No receivers (e.g. API-only node in tests) is fine.
        let _ = self.tx.send(Arc::new(ev));
    }

    fn subscribe(&self) -> broadcast::Receiver<Arc<BusEvent>> {
        self.tx.subscribe()
    }
}

#[cfg(feature = "nats")]
pub mod nats {
    use super::*;
    use futures_util::StreamExt;

    const SUBJECT: &str = "neoshard.events";

    /// Publishes to NATS; a background task feeds every received event
    /// (including our own) into the local broadcast.
    pub struct NatsBus {
        client: async_nats::Client,
        local: broadcast::Sender<Arc<BusEvent>>,
    }

    impl NatsBus {
        pub async fn connect(url: &str) -> anyhow::Result<Arc<Self>> {
            let client = async_nats::connect(url).await?;
            let (local, _) = broadcast::channel(65_536);
            let mut sub = client.subscribe(SUBJECT).await?;
            let tx = local.clone();
            tokio::spawn(async move {
                while let Some(msg) = sub.next().await {
                    if let Some(ev) = BusEvent::decode(&msg.payload) {
                        let _ = tx.send(Arc::new(ev));
                    }
                }
                tracing::error!("NATS subscription ended");
            });
            Ok(Arc::new(Self { client, local }))
        }
    }

    impl EventBus for NatsBus {
        fn publish(&self, ev: BusEvent) {
            let client = self.client.clone();
            let payload = ev.encode();
            tokio::spawn(async move {
                if let Err(e) = client.publish(SUBJECT, payload.into()).await {
                    tracing::warn!(error = %e, "NATS publish failed");
                }
            });
        }

        fn subscribe(&self) -> broadcast::Receiver<Arc<BusEvent>> {
            self.local.subscribe()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_roundtrip() {
        let ev = BusEvent::channel(
            5,
            7,
            v1::Intent::GuildMessages,
            v1::event::Kind::MessageDelete(v1::MessageDelete {
                id: 1,
                channel_id: 7,
                guild_id: 5,
            }),
        );
        let back = BusEvent::decode(&ev.encode()).unwrap();
        assert_eq!(back.topic, ev.topic);
        assert_eq!(back.scope, ev.scope);
        assert_eq!(back.intent, ev.intent);
        assert_eq!(back.event, ev.event);
        let ev2 = BusEvent::user(
            9,
            v1::Intent::None,
            v1::event::Kind::PinsUpdate(v1::PinsUpdate::default()),
        )
        .with_scope(Scope::GuildPerm { perms: 3 });
        let b2 = BusEvent::decode(&ev2.encode()).unwrap();
        assert_eq!(b2.scope, Scope::GuildPerm { perms: 3 });
        assert!(BusEvent::decode(&[9, 9]).is_none());
    }

    #[tokio::test]
    async fn local_bus_delivers() {
        let bus = LocalBus::new();
        let mut rx = bus.subscribe();
        bus.publish(BusEvent::user(
            1,
            v1::Intent::None,
            v1::event::Kind::PinsUpdate(v1::PinsUpdate::default()),
        ));
        assert_eq!(rx.recv().await.unwrap().topic, Topic::User(1));
    }
}
