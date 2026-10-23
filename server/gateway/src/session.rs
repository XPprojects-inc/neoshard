//! Session actor. A session outlives its WebSocket connection: while
//! detached it keeps buffering events (bounded by count and bytes, for
//! [`RESUME_TTL`]) so that a client can Resume without losing anything.
//! If events are ever dropped (slow consumer, router lag) the session is
//! marked lost and Resume is refused — a gap is never papered over.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use neoshard_presence::Custom;
use neoshard_proto::ProstMessage;
use neoshard_proto::permissions::Permissions as P;
use neoshard_proto::v1::{self, Intent, event::Kind, gateway_frame};
use neoshard_server_common::bus::{BusEvent, Scope, Topic};
use neoshard_server_common::repo::{self, GuildSnapshot};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::hub::{Hub, INBOX};

pub const REPLAY_CAP: usize = 1024;
/// Encoded bytes kept per session for Resume (100k sessions × 128 KiB worst case).
pub const REPLAY_BYTES: usize = 128 * 1024;

/// Ephemeral events are dispatched with `seq = 0`: they are not buffered for
/// Resume (a stale "typing…" or presence replayed later would be wrong anyway).
fn ephemeral(e: &v1::Event) -> bool {
    matches!(
        e.kind,
        Some(Kind::TypingStart(_) | Kind::PresenceUpdate(_) | Kind::MemberListChunk(_))
    )
}
pub const RESUME_TTL: Duration = Duration::from_secs(120);
/// Guilds above this size get no presence broadcasts (O(members²) otherwise);
/// clients see presence through the lazy member list (spec/gateway.md).
pub const LARGE_GUILD: u32 = 250;

/// Outgoing frames to the connection writer.
pub type Outgoing = mpsc::Sender<Bytes>;

pub enum Ctl {
    /// Attach a connection (generation `epoch`). `last_seq` = Some for Resume.
    /// Replies with the number of replayed events, or Err if the session cannot resume.
    Attach {
        epoch: u64,
        out: Outgoing,
        compress: bool,
        last_seq: Option<u64>,
        kill: Arc<Notify>,
        reply: oneshot::Sender<Result<u64, ()>>,
    },
    /// The connection of generation `epoch` went away (stale generations are ignored).
    Detach {
        epoch: u64,
    },
    Request(v1::ClientRequest),
    /// Events were dropped for this session: close the connection, refuse Resume.
    Overflow,
    Close,
}

struct GuildView {
    member: v1::Member,
    guild_perms: P,
    channels: HashMap<u64, P>,
    /// Threads are not part of visibility diffs.
    threads: HashSet<u64>,
    large: bool,
}

pub struct SessionInit {
    pub id: String,
    pub user_id: u64,
    pub auth_session_id: u64,
    pub bot: bool,
    pub intents: u32,
    pub status: v1::Status,
    /// Stored custom status, restored when the user comes online.
    pub custom: Custom,
    /// Shared with the hub's SessionHandle: whether a connection is attached.
    pub attached: Arc<std::sync::atomic::AtomicBool>,
}

pub struct Session {
    hub: Arc<Hub>,
    key: u64,
    id: String,
    user_id: u64,
    auth_session_id: u64,
    bot: bool,
    intents: u32,
    seq: u64,
    /// (seq, encoded Event) — compact; decoded only on Resume.
    replay: VecDeque<(u64, Vec<u8>)>,
    replay_bytes: usize,
    guilds: HashMap<u64, GuildView>,
    events_tx: mpsc::Sender<Arc<BusEvent>>,
    out: Option<Outgoing>,
    kill: Option<Arc<Notify>>,
    epoch: u64,
    compress: bool,
    lost: bool,
    revoked: bool,
    typing_last: HashMap<u64, std::time::Instant>,
    status: v1::Status,
    custom: Custom,
    attached: Arc<std::sync::atomic::AtomicBool>,
}

/// A session that is subscribed but not yet running (Ready is being built).
pub struct Prepared {
    session: Session,
    events_rx: mpsc::Receiver<Arc<BusEvent>>,
    ctl_rx: mpsc::Receiver<Ctl>,
    pub ctl: mpsc::Sender<Ctl>,
}

fn now_ms() -> i64 {
    repo::now_ms()
}

fn compute_view(snap: &GuildSnapshot, member: v1::Member) -> GuildView {
    let now = now_ms();
    let guild_perms = snap.guild_perms(&member, now);
    let channels = snap
        .channels
        .keys()
        .map(|cid| (*cid, snap.channel_perms(&member, *cid, now)))
        .collect();
    let threads = snap
        .channels
        .iter()
        .filter(|(_, c)| c.2 == v1::ChannelType::Thread as i32)
        .map(|(id, _)| *id)
        .collect();
    GuildView {
        member,
        guild_perms,
        channels,
        threads,
        large: false,
    }
}

impl Session {
    /// Subscribe to the user's topics *before* Ready is built so nothing is
    /// missed; events queue in the inbox until [`Prepared::start`].
    pub fn prepare(hub: Arc<Hub>, init: SessionInit, guild_ids: &[u64]) -> Prepared {
        let (events_tx, events_rx) = mpsc::channel(INBOX);
        let (ctl_tx, ctl_rx) = mpsc::channel(64);
        let key = hub.new_key();
        hub.register_ctl(key, ctl_tx.clone());
        hub.subscribe(Topic::User(init.user_id), key, events_tx.clone());
        for gid in guild_ids {
            hub.subscribe(Topic::Guild(*gid), key, events_tx.clone());
        }
        let session = Session {
            hub,
            key,
            id: init.id,
            user_id: init.user_id,
            auth_session_id: init.auth_session_id,
            bot: init.bot,
            intents: init.intents,
            seq: 0,
            replay: VecDeque::new(),
            replay_bytes: 0,
            guilds: HashMap::new(),
            events_tx,
            out: None,
            kill: None,
            epoch: 0,
            compress: false,
            lost: false,
            revoked: false,
            typing_last: HashMap::new(),
            status: init.status,
            custom: init.custom,
            attached: init.attached,
        };
        Prepared {
            session,
            events_rx,
            ctl_rx,
            ctl: ctl_tx,
        }
    }

    fn broadcast_presence(&self, p: v1::Presence) {
        for gid in self.guilds.iter().filter(|(_, v)| !v.large).map(|(g, _)| g) {
            self.hub.ctx.bus.publish(BusEvent::guild(
                *gid,
                Intent::Presence,
                Kind::PresenceUpdate(p.clone()),
            ));
        }
        // Friends without a shared server learn it through their own user topic.
        let (hub, uid, fp) = (self.hub.clone(), self.user_id, p.clone());
        tokio::spawn(async move {
            if let Ok(friends) = repo::friend_ids(&hub.ctx.db, uid).await {
                for f in friends {
                    hub.ctx.bus.publish(BusEvent::user(
                        f,
                        Intent::Presence,
                        Kind::PresenceUpdate(fp.clone()),
                    ));
                }
            }
        });
        self.hub.ctx.bus.publish(BusEvent::user(
            self.user_id,
            Intent::Presence,
            Kind::PresenceUpdate(p),
        ));
    }

    async fn run(
        mut self,
        mut events: mpsc::Receiver<Arc<BusEvent>>,
        mut ctl: mpsc::Receiver<Ctl>,
    ) {
        let mut detached_since: Option<tokio::time::Instant> = Some(tokio::time::Instant::now());
        loop {
            let expiry = async {
                match detached_since {
                    Some(t) => tokio::time::sleep_until(t + RESUME_TTL).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                ev = events.recv() => {
                    let Some(ev) = ev else { break };
                    let mut batch = Vec::new();
                    // Boxed: the (rare, DB-touching) handler state must not inflate
                    // this long-lived future for every one of 100k sessions.
                    Box::pin(self.handle_event(&ev, &mut batch)).await;
                    // Coalesce whatever else is already queued into the same frame.
                    let mut n = 0;
                    while n < 256 && !self.revoked {
                        match events.try_recv() {
                            Ok(ev) => { Box::pin(self.handle_event(&ev, &mut batch)).await; n += 1; }
                            Err(_) => break,
                        }
                    }
                    self.flush(batch);
                    if self.out.is_none() && detached_since.is_none() {
                        detached_since = Some(tokio::time::Instant::now());
                    }
                }
                c = ctl.recv() => {
                    match c {
                        None | Some(Ctl::Close) => break,
                        Some(Ctl::Attach { epoch, out, compress, last_seq, kill, reply }) => {
                            let r = self.attach(epoch, out, compress, last_seq, kill);
                            if r.is_ok() { detached_since = None; }
                            self.attached.store(self.out.is_some(), std::sync::atomic::Ordering::Relaxed);
                            let _ = reply.send(r);
                        }
                        Some(Ctl::Detach { epoch }) => {
                            if epoch == self.epoch && self.out.is_some() {
                                self.drop_connection();
                                detached_since = Some(tokio::time::Instant::now());
                            }
                        }
                        Some(Ctl::Overflow) => {
                            self.lost = true;
                            self.drop_connection();
                            detached_since = Some(tokio::time::Instant::now());
                        }
                        Some(Ctl::Request(r)) => Box::pin(self.handle_request(r)).await,
                    }
                }
                _ = expiry => break,
            }
            if self.revoked {
                break;
            }
        }
        self.shutdown();
    }

    /// Forget the current connection and make its socket close.
    fn drop_connection(&mut self) {
        self.out = None;
        self.attached
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(k) = self.kill.take() {
            k.notify_one();
        }
    }

    fn shutdown(&mut self) {
        self.drop_connection();
        let hub = self.hub.clone();
        hub.unsubscribe(Topic::User(self.user_id), self.key);
        for gid in self.guilds.keys() {
            hub.unsubscribe(Topic::Guild(*gid), self.key);
        }
        hub.unregister_ctl(self.key);
        hub.sessions.remove(&self.id);
        hub.release_session(self.user_id);
        if let Some(p) = hub.presence.disconnect(self.user_id, &self.id) {
            self.broadcast_presence(p);
        }
        // Leave voice when the last session goes away.
        if !hub.presence.is_online(self.user_id) {
            self.leave_voice(None);
        }
    }

    fn leave_voice(&self, only_guild: Option<u64>) {
        if let Some(vs) = self.hub.presence.voice_of(self.user_id) {
            if only_guild.is_some_and(|g| g != vs.guild_id) {
                return;
            }
            let leave = v1::VoiceState {
                channel_id: 0,
                ..vs
            };
            self.hub.presence.voice_set(leave);
            self.publish_voice(&vs, leave);
        }
    }

    /// Voice updates are scoped to the channel: hidden voice channels stay hidden.
    fn publish_voice(&self, scope_from: &v1::VoiceState, state: v1::VoiceState) {
        if scope_from.channel_id == 0 {
            return;
        }
        if scope_from.guild_id == 0 {
            // DM call: every participant (including our other devices) hears about it
            // through their own user topic; this is what rings the other side.
            let (hub, cid) = (self.hub.clone(), scope_from.channel_id);
            tokio::spawn(async move {
                if let Ok(Some(ch)) = repo::channel(&hub.ctx.db, cid).await {
                    for u in ch.recipient_ids {
                        hub.ctx.bus.publish(BusEvent::user(
                            u,
                            Intent::VoiceStates,
                            Kind::VoiceStateUpdate(state),
                        ));
                    }
                }
            });
            return;
        }
        self.hub.ctx.bus.publish(BusEvent::channel(
            scope_from.guild_id,
            scope_from.channel_id,
            Intent::VoiceStates,
            Kind::VoiceStateUpdate(state),
        ));
    }

    fn attach(
        &mut self,
        epoch: u64,
        out: Outgoing,
        compress: bool,
        last_seq: Option<u64>,
        kill: Arc<Notify>,
    ) -> Result<u64, ()> {
        let mut replayed = 0;
        if let Some(last) = last_seq {
            if self.lost {
                return Err(());
            }
            let oldest = self.replay.front().map(|(s, _)| *s).unwrap_or(self.seq + 1);
            if last > self.seq || (last + 1 < oldest && last < self.seq) {
                return Err(());
            }
            let items: Vec<v1::Dispatch> = self
                .replay
                .iter()
                .filter(|(s, _)| *s > last)
                .map(|(s, bytes)| v1::Dispatch {
                    seq: *s,
                    event: v1::Event::decode(bytes.as_slice()).ok(),
                })
                .collect();
            replayed = items.len() as u64;
            self.drop_connection();
            self.install(epoch, out, compress, kill);
            for chunk in items.chunks(256) {
                let f = v1::GatewayFrame {
                    kind: Some(gateway_frame::Kind::Dispatch(v1::DispatchBatch {
                        items: chunk.to_vec(),
                    })),
                };
                self.send_frame(&f);
            }
        } else {
            self.drop_connection();
            self.install(epoch, out, compress, kill);
        }
        Ok(replayed)
    }

    fn install(&mut self, epoch: u64, out: Outgoing, compress: bool, kill: Arc<Notify>) {
        self.epoch = epoch;
        self.out = Some(out);
        self.kill = Some(kill);
        self.compress = compress;
    }

    fn send_frame(&mut self, f: &v1::GatewayFrame) {
        if let Some(out) = &self.out {
            let bytes = Bytes::from(neoshard_proto::framing::encode(f, self.compress));
            if out.try_send(bytes).is_err() {
                // Writer gone or saturated. Events are in the replay buffer: the client can Resume.
                self.drop_connection();
            }
        }
    }

    fn flush(&mut self, batch: Vec<v1::Event>) {
        if batch.is_empty() {
            return;
        }
        let mut items = Vec::with_capacity(batch.len());
        for e in batch {
            if ephemeral(&e) {
                items.push(v1::Dispatch {
                    seq: 0,
                    event: Some(e),
                });
                continue;
            }
            self.seq += 1;
            let bytes = e.encode_to_vec();
            self.replay_bytes += bytes.len();
            self.replay.push_back((self.seq, bytes));
            while self.replay.len() > REPLAY_CAP || self.replay_bytes > REPLAY_BYTES {
                match self.replay.pop_front() {
                    Some((_, b)) => self.replay_bytes -= b.len(),
                    None => break,
                }
            }
            items.push(v1::Dispatch {
                seq: self.seq,
                event: Some(e),
            });
        }
        self.hub
            .metrics
            .events_out
            .fetch_add(items.len() as u64, std::sync::atomic::Ordering::Relaxed);
        let f = v1::GatewayFrame {
            kind: Some(gateway_frame::Kind::Dispatch(v1::DispatchBatch { items })),
        };
        self.send_frame(&f);
    }

    fn wants(&self, intent: u32) -> bool {
        intent == 0 || self.intents == 0 || self.intents & intent != 0
    }

    async fn refresh_guild(&mut self, gid: u64) {
        let Some(view) = self.guilds.get(&gid) else {
            return;
        };
        let member = view.member.clone();
        if let Some(snap) = self.hub.snapshot(gid).await {
            let large = self.guilds.get(&gid).is_some_and(|v| v.large);
            let mut view = compute_view(&snap, member);
            view.large = large;
            self.guilds.insert(gid, view);
        }
    }

    fn visible_set(&self, gid: u64) -> HashSet<u64> {
        self.guilds
            .get(&gid)
            .map(|v| {
                v.channels
                    .iter()
                    .filter(|(c, p)| p.contains(P::VIEW_CHANNEL) && !v.threads.contains(c))
                    .map(|(c, _)| *c)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Recompute permissions and tell the client which channels appeared or
    /// disappeared for it (role/overwrite/member changes). Threads are excluded:
    /// clients learn about them through their own create/update events.
    async fn refresh_and_diff(&mut self, gid: u64, out: &mut Vec<v1::Event>) {
        if !self.guilds.contains_key(&gid) {
            return;
        }
        let before = self.visible_set(gid);
        self.refresh_guild(gid).await;
        let after = self.visible_set(gid);
        if !self.wants(Intent::Guilds as u32) {
            return;
        }
        for cid in before.difference(&after) {
            out.push(v1::Event {
                kind: Some(Kind::ChannelDelete(v1::ChannelDelete {
                    id: *cid,
                    guild_id: gid,
                })),
            });
        }
        for cid in after.difference(&before) {
            if let Ok(Some(ch)) = repo::channel(&self.hub.ctx.db, *cid).await {
                out.push(v1::Event {
                    kind: Some(Kind::ChannelCreate(ch)),
                });
            }
        }
    }

    fn visible(&self, gid: u64, cid: u64) -> bool {
        self.guilds
            .get(&gid)
            .and_then(|v| v.channels.get(&cid))
            .is_some_and(|p| p.contains(P::VIEW_CHANNEL))
    }

    fn init_views(&mut self, guilds: &[v1::Guild]) {
        for g in guilds {
            if let Some(m) = &g.self_member {
                let snap = GuildSnapshot::from_parts(g.id, g.owner_id, &g.roles, &g.channels);
                let mut view = compute_view(&snap, m.clone());
                view.large = g.member_count > LARGE_GUILD;
                self.guilds.insert(g.id, view);
            }
        }
    }

    async fn join_guild(&mut self, g: &v1::Guild) {
        let Some(m) = g.self_member.clone() else {
            return;
        };
        let snap = match self.hub.snapshot(g.id).await {
            Some(s) => s,
            None => Arc::new(GuildSnapshot::from_parts(
                g.id,
                g.owner_id,
                &g.roles,
                &g.channels,
            )),
        };
        let mut view = compute_view(&snap, m);
        view.large = g.member_count > LARGE_GUILD;
        self.guilds.insert(g.id, view);
        self.hub
            .subscribe(Topic::Guild(g.id), self.key, self.events_tx.clone());
    }

    fn leave_guild(&mut self, gid: u64) {
        if self.guilds.remove(&gid).is_some() {
            self.hub.unsubscribe(Topic::Guild(gid), self.key);
        }
        self.leave_voice(Some(gid));
    }

    /// Update internal state, then decide what (if anything) the client gets.
    async fn handle_event(&mut self, ev: &BusEvent, out: &mut Vec<v1::Event>) {
        let Some(kind) = &ev.event.kind else { return };
        // ---- state first (independent of intents)
        match kind {
            Kind::SessionRevoked(r) => {
                let hit = (r.all && r.except_session_id != self.auth_session_id)
                    || r.session_ids.contains(&self.auth_session_id);
                if hit {
                    let err = v1::GatewayFrame {
                        kind: Some(gateway_frame::Kind::Error(v1::GatewayError {
                            code: v1::GatewayErrorCode::AuthFailed as i32,
                            message: "session revoked".into(),
                            reconnect: false,
                        })),
                    };
                    self.send_frame(&err);
                    self.revoked = true;
                    self.lost = true;
                }
                return; // internal event, never forwarded
            }
            Kind::GuildCreate(g) if matches!(ev.topic, Topic::User(_)) => self.join_guild(g).await,
            Kind::GuildDelete(d) => {
                if matches!(ev.topic, Topic::User(_)) || self.guilds.contains_key(&d.id) {
                    self.leave_guild(d.id);
                }
            }
            Kind::MemberUpdate(m) if m.user.as_ref().map(|u| u.id) == Some(self.user_id) => {
                if let Some(view) = self.guilds.get_mut(&m.guild_id) {
                    view.member = m.clone();
                }
                self.refresh_and_diff(m.guild_id, out).await;
            }
            Kind::MemberRemove(r) if r.user_id == self.user_id => self.leave_guild(r.guild_id),
            Kind::ChannelUpdate(c) if c.guild_id != 0 => {
                // Overwrite changes: the diff emits create/delete when visibility flips;
                // the update itself is only delivered if the channel stays visible.
                let before = self.visible(c.guild_id, c.id);
                self.refresh_and_diff(c.guild_id, out).await;
                if !(before && self.visible(c.guild_id, c.id)) {
                    return;
                }
            }
            Kind::ChannelDelete(d) if d.guild_id != 0 => {
                let was_visible = self.visible(d.guild_id, d.id);
                self.refresh_guild(d.guild_id).await;
                // The channel is gone from the refreshed view, so decide here.
                if was_visible && self.wants(ev.intent) {
                    out.push(ev.event.clone());
                }
                return;
            }
            Kind::ChannelCreate(c) if c.guild_id != 0 => self.refresh_guild(c.guild_id).await,
            Kind::RoleCreate(r) | Kind::RoleUpdate(r) => {
                self.refresh_and_diff(r.guild_id, out).await
            }
            Kind::RoleDelete(r) => {
                if let Some(v) = self.guilds.get_mut(&r.guild_id) {
                    v.member.role_ids.retain(|id| *id != r.id);
                }
                self.refresh_and_diff(r.guild_id, out).await;
            }
            Kind::GuildUpdate(g) => self.refresh_and_diff(g.id, out).await,
            _ => {}
        }
        // ---- delivery filters
        if !self.wants(ev.intent) {
            return;
        }
        if let Topic::Guild(gid) = ev.topic {
            let Some(view) = self.guilds.get(&gid) else {
                if matches!(kind, Kind::GuildDelete(_)) {
                    out.push(ev.event.clone());
                }
                return;
            };
            match ev.scope {
                Scope::All => {}
                Scope::Channel { channel_id, perms } => {
                    let need = P::raw(perms) | P::VIEW_CHANNEL;
                    if !view
                        .channels
                        .get(&channel_id)
                        .is_some_and(|p| p.contains(need))
                    {
                        return;
                    }
                }
                Scope::GuildPerm { perms } => {
                    if !view.guild_perms.contains(P::raw(perms)) {
                        return;
                    }
                }
            }
        }
        if let Kind::TypingStart(t) = kind
            && t.user_id == self.user_id
        {
            return;
        }
        // Bots without the MESSAGE_CONTENT intent only see content addressed to them.
        if self.bot
            && self.intents & Intent::MessageContent as u32 == 0
            && let Kind::MessageCreate(m) | Kind::MessageUpdate(m) = kind
        {
            let addressed = m.mention_user_ids.contains(&self.user_id)
                || m.guild_id == 0
                || m.author.as_ref().map(|a| a.id) == Some(self.user_id);
            if !addressed {
                let mut m = m.clone();
                m.content.clear();
                m.embeds.clear();
                m.attachments.clear();
                let k = if matches!(kind, Kind::MessageCreate(_)) {
                    Kind::MessageCreate(m)
                } else {
                    Kind::MessageUpdate(m)
                };
                out.push(v1::Event { kind: Some(k) });
                return;
            }
        }
        out.push(ev.event.clone());
    }

    /// Channel + effective permissions for this session's user, re-evaluating timeouts.
    async fn channel_access(&mut self, cid: u64) -> Option<(v1::Channel, P)> {
        let in_guild = self
            .guilds
            .iter()
            .find(|(_, v)| v.channels.contains_key(&cid))
            .map(|(g, _)| *g);
        if let Some(gid) = in_guild {
            // Timeouts expire on their own: recompute if one is set.
            if self
                .guilds
                .get(&gid)
                .is_some_and(|v| v.member.timeout_until != 0)
            {
                self.refresh_guild(gid).await;
            }
            let p = *self.guilds.get(&gid)?.channels.get(&cid)?;
            if !p.contains(P::VIEW_CHANNEL) {
                return None;
            }
            let ch = repo::channel(&self.hub.ctx.db, cid).await.ok()??;
            return (ch.guild_id == gid).then_some((ch, p));
        }
        let ch = repo::channel(&self.hub.ctx.db, cid).await.ok()??;
        if ch.guild_id == 0 && ch.recipient_ids.contains(&self.user_id) {
            let dm = P::VIEW_CHANNEL
                | P::SEND_MESSAGES
                | P::READ_MESSAGE_HISTORY
                | P::CONNECT
                | P::SPEAK
                | P::VIDEO
                | P::STREAM;
            return Some((ch, dm));
        }
        None
    }

    async fn handle_request(&mut self, req: v1::ClientRequest) {
        use v1::client_request::Kind as R;
        let Some(kind) = req.kind else { return };
        match kind {
            R::PresenceSet(p) => {
                let status = v1::Status::try_from(p.status).unwrap_or(v1::Status::Online);
                self.status = status;
                let custom = Custom::new(&p.custom_status, &p.custom_emoji, p.custom_expires_at);
                if custom != self.custom {
                    // The custom status survives reconnects and other devices: persist it.
                    self.custom = custom.clone();
                    let db = self.hub.ctx.db.clone();
                    let (uid, c) = (self.user_id, custom.clone());
                    tokio::spawn(async move {
                        let expires = (c.expires_at > 0)
                            .then(|| chrono::DateTime::from_timestamp_millis(c.expires_at))
                            .flatten();
                        if let Err(e) = sqlx::query!(
                            "UPDATE users SET custom_status = $2, custom_status_emoji = $3, custom_status_expires_at = $4 WHERE id = $1",
                            uid as i64,
                            c.text,
                            c.emoji,
                            expires
                        )
                        .execute(&db)
                        .await
                        {
                            tracing::warn!(error = %e, "persist custom status");
                        }
                    });
                }
                if let Some(pr) = self.hub.presence.set(self.user_id, status, &custom) {
                    self.broadcast_presence(pr);
                }
            }
            R::Typing(t) => {
                let now = std::time::Instant::now();
                if self
                    .typing_last
                    .get(&t.channel_id)
                    .is_some_and(|l| now.duration_since(*l) < Duration::from_secs(8))
                {
                    return;
                }
                let Some((ch, perms)) = self.channel_access(t.channel_id).await else {
                    return;
                };
                if !perms.contains(P::SEND_MESSAGES) {
                    return;
                }
                if self.typing_last.len() > 256 {
                    self.typing_last.clear();
                }
                self.typing_last.insert(t.channel_id, now);
                let ev = Kind::TypingStart(v1::TypingStart {
                    channel_id: ch.id,
                    guild_id: ch.guild_id,
                    user_id: self.user_id,
                    at: now_ms(),
                });
                if ch.guild_id != 0 {
                    self.hub.ctx.bus.publish(BusEvent::channel(
                        ch.guild_id,
                        ch.id,
                        Intent::GuildTyping,
                        ev,
                    ));
                } else {
                    for u in &ch.recipient_ids {
                        self.hub.ctx.bus.publish(BusEvent::user(
                            *u,
                            Intent::DirectMessages,
                            ev.clone(),
                        ));
                    }
                }
            }
            R::VoiceState(v) => self.voice_state(v).await,
            R::MemberList(m) => {
                if !self.guilds.contains_key(&m.guild_id) || m.end <= m.start {
                    return;
                }
                let limit = (m.end - m.start).min(200) as i64;
                let start = i64::from(m.start.min(100_000));
                let Ok(members) =
                    repo::members_page(&self.hub.ctx.db, m.guild_id, start, limit).await
                else {
                    return;
                };
                let total = repo::member_count(&self.hub.ctx.db, m.guild_id)
                    .await
                    .unwrap_or(0) as u32;
                let presences = members
                    .iter()
                    .filter_map(|x| x.user.as_ref())
                    .map(|u| self.hub.presence.get(u.id))
                    .collect();
                let chunk = v1::MemberListChunk {
                    guild_id: m.guild_id,
                    start: m.start,
                    total,
                    members,
                    presences,
                };
                self.flush(vec![v1::Event {
                    kind: Some(Kind::MemberListChunk(chunk)),
                }]);
            }
            R::Ack(a) => {
                if self.channel_access(a.channel_id).await.is_none() {
                    return;
                }
                let r = sqlx::query!(
                    "INSERT INTO read_states (user_id, channel_id, last_read_message_id, mention_count) VALUES ($1, $2, $3, 0)
                     ON CONFLICT (user_id, channel_id) DO UPDATE
                       SET last_read_message_id = GREATEST(read_states.last_read_message_id, EXCLUDED.last_read_message_id), mention_count = 0
                     RETURNING last_read_message_id",
                    self.user_id as i64,
                    a.channel_id as i64,
                    a.message_id as i64
                )
                .fetch_one(&self.hub.ctx.db)
                .await;
                if let Ok(r) = r {
                    let rs = v1::ReadState {
                        channel_id: a.channel_id,
                        last_read_message_id: r.last_read_message_id as u64,
                        mention_count: 0,
                    };
                    self.hub.ctx.bus.publish(BusEvent::user(
                        self.user_id,
                        Intent::None,
                        Kind::ReadStateUpdate(rs),
                    ));
                }
            }
        }
    }

    async fn voice_state(&mut self, v: v1::VoiceStateSet) {
        let prev = self.hub.presence.voice_of(self.user_id);
        if v.channel_id == 0 {
            // Leave: scope and guild come from the stored state, never from the request.
            self.leave_voice(None);
            return;
        }
        let Some((ch, perms)) = self.channel_access(v.channel_id).await else {
            return;
        };
        let voice_ok = ch.r#type == v1::ChannelType::Voice as i32 || ch.guild_id == 0;
        if !voice_ok || !perms.contains(P::CONNECT) || ch.guild_id != v.guild_id {
            return;
        }
        if (v.video && !perms.contains(P::VIDEO)) || (v.streaming && !perms.contains(P::STREAM)) {
            return;
        }
        // A block on either side stops 1:1 calls, like it stops messages.
        if ch.guild_id == 0 && ch.recipient_ids.len() == 2 {
            let peer = ch
                .recipient_ids
                .iter()
                .copied()
                .find(|&u| u != self.user_id);
            if let Some(peer) = peer
                && repo::blocked_between(&self.hub.ctx.db, self.user_id, peer)
                    .await
                    .unwrap_or(true)
            {
                return;
            }
        }
        // Server mute/deafen are set by moderators and survive client updates.
        let (server_mute, server_deaf) = prev
            .as_ref()
            .filter(|p| p.guild_id == ch.guild_id)
            .map(|p| (p.server_mute, p.server_deaf))
            .unwrap_or((false, false));
        let state = v1::VoiceState {
            guild_id: ch.guild_id,
            channel_id: ch.id,
            user_id: self.user_id,
            self_mute: v.self_mute,
            self_deaf: v.self_deaf,
            server_mute,
            server_deaf,
            video: v.video,
            streaming: v.streaming,
        };
        self.hub.presence.voice_set(state);
        if let Some(p) = prev.filter(|p| p.channel_id != state.channel_id) {
            // Moved: tell the old channel's audience.
            self.publish_voice(&p, v1::VoiceState { channel_id: 0, ..p });
        }
        self.publish_voice(&state, state);
    }
}

impl Prepared {
    /// Initialize permission views from Ready and start the actor.
    pub fn start(mut self, guilds: &[v1::Guild]) -> mpsc::Sender<Ctl> {
        self.session.init_views(guilds);
        if let Some(p) = self.session.hub.presence.connect(
            self.session.user_id,
            &self.session.id,
            self.session.status,
            &self.session.custom,
        ) {
            self.session.broadcast_presence(p);
        }
        tokio::spawn(self.session.run(self.events_rx, self.ctl_rx));
        self.ctl
    }

    /// Abort before starting (Ready failed): undo subscriptions.
    pub fn abort(self, guild_ids: &[u64]) {
        let s = self.session;
        s.hub.unsubscribe(Topic::User(s.user_id), s.key);
        for gid in guild_ids {
            s.hub.unsubscribe(Topic::Guild(*gid), s.key);
        }
        s.hub.unregister_ctl(s.key);
        s.hub.release_session(s.user_id);
    }
}

/// Build the Ready payload for a user. Voice states are filtered by channel visibility.
pub async fn build_ready(
    hub: &Hub,
    user_id: u64,
    session_id: &str,
    guild_ids: &[u64],
) -> anyhow::Result<v1::Ready> {
    let db = &hub.ctx.db;
    let user = repo::user(db, user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user vanished"))?
        .private();
    let mut guilds = Vec::new();
    for gid in guild_ids {
        if let Some(mut g) = repo::guild_full(db, *gid, user_id).await? {
            if g.self_member.is_none() {
                continue;
            }
            let visible: HashSet<u64> = g.channels.iter().map(|c| c.id).collect();
            g.voice_states = hub
                .presence
                .voice_in_guild(*gid)
                .into_iter()
                .filter(|v| visible.contains(&v.channel_id))
                .collect();
            guilds.push(g);
        }
    }
    let private_channels = repo::private_channels(db, user_id).await?;
    // Presences: DM recipients and members of small guilds (≤ 250); larger guilds use the lazy member list.
    let mut users: HashSet<u64> = private_channels
        .iter()
        .flat_map(|c| c.recipient_ids.iter().copied())
        .collect();
    let small: Vec<i64> = guilds
        .iter()
        .filter(|g| g.member_count <= 250)
        .map(|g| g.id as i64)
        .collect();
    if !small.is_empty() {
        for uid in sqlx::query_scalar!(
            "SELECT DISTINCT user_id FROM members WHERE guild_id = ANY($1)",
            &small
        )
        .fetch_all(db)
        .await?
        {
            users.insert(uid as u64);
        }
    }
    let relationships = repo::relationships(db, user_id).await?;
    users.extend(
        relationships
            .iter()
            .filter(|r| r.kind == v1::RelationshipKind::Friend as i32)
            .filter_map(|r| r.user.as_ref().map(|u| u.id)),
    );
    users.remove(&user_id);
    let presences = users
        .into_iter()
        .map(|u| hub.presence.get(u))
        .filter(|p| p.status != v1::Status::Offline as i32)
        .collect();
    Ok(v1::Ready {
        session_id: session_id.to_string(),
        resume_url: hub.ctx.cfg.gateway_public_url(),
        user: Some(user),
        guilds,
        private_channels,
        read_states: repo::read_states(db, user_id).await?,
        presences,
        protocol_version: neoshard_proto::PROTOCOL_VERSION,
        files_base: hub.ctx.cfg.files_base(),
        relationships,
    })
}
