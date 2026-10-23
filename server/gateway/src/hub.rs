//! Node-local routing: topic index → session inboxes, guild snapshot cache.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use dashmap::DashMap;
use neoshard_proto::v1::event::Kind;
use neoshard_server_common::SharedCtx;
use neoshard_server_common::bus::{BusEvent, Topic};
use neoshard_server_common::repo::GuildSnapshot;
use tokio::sync::mpsc;

use crate::session::Ctl;

/// Bounded per-session inbox. A session that cannot keep up loses events and
/// is closed without the possibility to Resume (the client re-identifies).
pub const INBOX: usize = 1024;
/// Gateway sessions per user (devices × tabs).
pub const MAX_SESSIONS_PER_USER: usize = 20;

#[derive(Clone)]
pub struct SessionHandle {
    pub user_id: u64,
    /// The login session (token) this gateway session belongs to.
    pub auth_session_id: u64,
    pub ctl: mpsc::Sender<Ctl>,
    /// A connection is attached right now (false while waiting for Resume).
    pub attached: Arc<AtomicBool>,
}

pub struct Hub {
    pub ctx: SharedCtx,
    pub presence: Arc<neoshard_presence::Registry>,
    pub node: String,
    /// topic → session key → inbox
    topics: DashMap<Topic, HashMap<u64, mpsc::Sender<Arc<BusEvent>>>>,
    /// session id (client-visible) → handle
    pub sessions: DashMap<String, SessionHandle>,
    pub per_user: DashMap<u64, usize>,
    /// session key → control channel
    ctls: DashMap<u64, mpsc::Sender<Ctl>>,
    next_key: AtomicU64,
    snapshots: DashMap<u64, Arc<GuildSnapshot>>,
    generations: DashMap<u64, u64>,
    loading: DashMap<u64, Arc<tokio::sync::Mutex<()>>>,
    pub metrics: Metrics,
}

#[derive(Default)]
pub struct Metrics {
    pub connections: AtomicU64,
    pub events_in: AtomicU64,
    pub events_out: AtomicU64,
    pub dropped_slow: AtomicU64,
    pub snapshot_loads: AtomicU64,
}

impl Hub {
    pub fn new(ctx: SharedCtx, presence: Arc<neoshard_presence::Registry>) -> Arc<Self> {
        let node = format!("node-{}", ctx.cfg.worker_id);
        let hub = Arc::new(Self {
            ctx,
            presence,
            node,
            topics: DashMap::new(),
            sessions: DashMap::new(),
            per_user: DashMap::new(),
            ctls: DashMap::new(),
            next_key: AtomicU64::new(1),
            snapshots: DashMap::new(),
            generations: DashMap::new(),
            loading: DashMap::new(),
            metrics: Metrics::default(),
        });
        hub.clone().spawn_router();
        hub
    }

    pub fn new_key(&self) -> u64 {
        self.next_key.fetch_add(1, Ordering::Relaxed)
    }

    pub fn subscribe(&self, topic: Topic, key: u64, tx: mpsc::Sender<Arc<BusEvent>>) {
        self.topics.entry(topic).or_default().insert(key, tx);
    }

    pub fn unsubscribe(&self, topic: Topic, key: u64) {
        let empty = match self.topics.get_mut(&topic) {
            Some(mut m) => {
                m.remove(&key);
                m.is_empty()
            }
            None => false,
        };
        if empty {
            self.topics.remove_if(&topic, |_, m| m.is_empty());
        }
    }

    pub fn register_ctl(&self, key: u64, ctl: mpsc::Sender<Ctl>) {
        self.ctls.insert(key, ctl);
    }

    pub fn unregister_ctl(&self, key: u64) {
        self.ctls.remove(&key);
    }

    pub fn subscriber_count(&self, topic: Topic) -> usize {
        self.topics.get(&topic).map(|m| m.len()).unwrap_or(0)
    }

    /// Try to reserve a session slot for a user.
    pub fn reserve_session(&self, user_id: u64) -> bool {
        let mut n = self.per_user.entry(user_id).or_insert(0);
        if *n >= MAX_SESSIONS_PER_USER {
            return false;
        }
        *n += 1;
        true
    }

    /// Close gateway sessions of `user_id` that wait for a Resume (no connection
    /// attached), optionally only those of one login session. Live tabs are never
    /// touched. Returns how many were asked to close.
    pub fn evict_detached(&self, user_id: u64, auth_session_id: Option<u64>) -> usize {
        let victims: Vec<mpsc::Sender<Ctl>> = self
            .sessions
            .iter()
            .filter(|s| {
                s.user_id == user_id
                    && !s.attached.load(Ordering::Relaxed)
                    && auth_session_id.is_none_or(|a| a == s.auth_session_id)
            })
            .map(|s| s.ctl.clone())
            .collect();
        for v in &victims {
            let _ = v.try_send(Ctl::Close);
        }
        victims.len()
    }

    pub fn release_session(&self, user_id: u64) {
        let remove = match self.per_user.get_mut(&user_id) {
            Some(mut n) => {
                *n = n.saturating_sub(1);
                *n == 0
            }
            None => false,
        };
        if remove {
            self.per_user.remove_if(&user_id, |_, n| *n == 0);
        }
    }

    /// Structural events change permissions: drop the cached snapshot first.
    fn invalidate_for(&self, ev: &BusEvent) {
        let gid = match &ev.event.kind {
            Some(Kind::ChannelCreate(c) | Kind::ChannelUpdate(c)) => c.guild_id,
            Some(Kind::ChannelDelete(d)) => d.guild_id,
            Some(Kind::RoleCreate(r) | Kind::RoleUpdate(r)) => r.guild_id,
            Some(Kind::RoleDelete(d)) => d.guild_id,
            Some(Kind::GuildUpdate(g)) => g.id,
            Some(Kind::GuildDelete(g)) => g.id,
            _ => return,
        };
        if gid != 0 {
            // Bump and drop under the generation entry lock (see `snapshot`).
            let mut g = self.generations.entry(gid).or_insert(0);
            *g += 1;
            self.snapshots.remove(&gid);
        }
    }

    fn spawn_router(self: Arc<Self>) {
        let mut rx = self.ctx.bus.subscribe();
        tokio::spawn(async move {
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Events are lost for everyone on this node: no session may Resume.
                        tracing::error!(
                            lost = n,
                            "gateway router lagged behind the event bus; invalidating sessions"
                        );
                        for c in self.ctls.iter() {
                            let _ = c.try_send(Ctl::Overflow);
                        }
                        continue;
                    }
                    Err(_) => break,
                };
                self.metrics.events_in.fetch_add(1, Ordering::Relaxed);
                self.invalidate_for(&ev);
                let targets: Vec<(u64, mpsc::Sender<Arc<BusEvent>>)> =
                    match self.topics.get(&ev.topic) {
                        Some(m) => m.iter().map(|(k, v)| (*k, v.clone())).collect(),
                        None => continue,
                    };
                for (key, tx) in targets {
                    if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(ev.clone()) {
                        self.metrics.dropped_slow.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(key, "session inbox full; closing slow session");
                        if let Some(ctl) = self.ctls.get(&key) {
                            let _ = ctl.try_send(Ctl::Overflow);
                        }
                    }
                }
            }
        });
    }

    /// Permission snapshot of a guild, cached until a structural event.
    /// Single-flight: concurrent callers share one load.
    pub async fn snapshot(&self, guild_id: u64) -> Option<Arc<GuildSnapshot>> {
        if let Some(s) = self.snapshots.get(&guild_id) {
            return Some(s.clone());
        }
        let lock = self
            .loading
            .entry(guild_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        if let Some(s) = self.snapshots.get(&guild_id) {
            return Some(s.clone());
        }
        let gen_before = self.generations.get(&guild_id).map(|g| *g).unwrap_or(0);
        self.metrics.snapshot_loads.fetch_add(1, Ordering::Relaxed);
        let snap = Arc::new(GuildSnapshot::load(&self.ctx.db, guild_id).await.ok()??);
        {
            let g = self.generations.entry(guild_id).or_insert(0);
            if *g == gen_before {
                self.snapshots.insert(guild_id, snap.clone());
            }
        }
        drop(_guard);
        self.loading
            .remove_if(&guild_id, |_, l| Arc::strong_count(l) <= 2);
        Some(snap)
    }
}
