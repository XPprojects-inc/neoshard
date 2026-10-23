//! Gateway connection state machine (sans-IO).
//!
//! Inputs: socket opened/closed, frames received, clock ticks, network
//! online/offline. Outputs: [`Action`]s the driver executes. No sockets,
//! no timers, no threads here — so this logic is identical on native, web
//! and mobile and is unit-tested without a network.

use neoshard_proto::core_api::ConnectionState;
use neoshard_proto::framing;
use neoshard_proto::v1::{self, gateway_frame::Kind as F};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Open a WebSocket to this URL.
    Connect(String),
    /// Send a binary frame.
    Send(Vec<u8>),
    /// Close the current socket (non-1000 code so the server keeps the session).
    Close,
    /// Connection state changed (for the UI).
    State(ConnectionState, u32),
    /// A non-dispatch frame worth surfacing (Ready).
    Ready(Box<v1::Ready>),
    /// Server events, in order.
    Events(Vec<v1::Event>),
    /// Server asked us to stop for good (auth failure).
    Fatal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting,
    AwaitHello,
    Identifying,
    Resuming,
    Ready,
    Waiting { until: u64 },
    Offline,
    Dead,
}

/// Small deterministic PRNG (xorshift64*) — keeps the core free of OS randomness.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    pub fn next_f64(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub struct GatewayMachine {
    pub url: String,
    token: String,
    intents: u32,
    client: String,
    phase: Phase,
    heartbeat_ms: u64,
    next_heartbeat: u64,
    acked: bool,
    pub session_id: Option<String>,
    resume_url: Option<String>,
    pub last_seq: u64,
    attempts: u32,
    rng: Rng,
}

pub const MAX_BACKOFF_MS: u64 = 30_000;

impl GatewayMachine {
    pub fn new(
        url: impl Into<String>,
        token: impl Into<String>,
        client: impl Into<String>,
        seed: u64,
    ) -> Self {
        Self {
            url: url.into(),
            token: token.into(),
            intents: 0,
            client: client.into(),
            phase: Phase::Idle,
            heartbeat_ms: 30_000,
            next_heartbeat: u64::MAX,
            acked: true,
            session_id: None,
            resume_url: None,
            last_seq: 0,
            attempts: 0,
            rng: Rng::new(seed),
        }
    }

    pub fn with_intents(mut self, intents: u32) -> Self {
        self.intents = intents;
        self
    }

    pub fn is_ready(&self) -> bool {
        self.phase == Phase::Ready
    }

    fn connect_url(&self) -> String {
        let base = self
            .resume_url
            .as_deref()
            .filter(|_| self.session_id.is_some())
            .unwrap_or(&self.url);
        let sep = if base.contains('?') { '&' } else { '?' };
        format!("{base}{sep}v=1&compress=zstd")
    }

    fn frame(kind: F) -> Vec<u8> {
        // Client → server frames are small; compression is not worth it.
        framing::encode(&v1::GatewayFrame { kind: Some(kind) }, false)
    }

    pub fn start(&mut self) -> Vec<Action> {
        if matches!(self.phase, Phase::Dead | Phase::Offline) {
            return vec![];
        }
        self.phase = Phase::Connecting;
        vec![
            Action::State(ConnectionState::Connecting, 0),
            Action::Connect(self.connect_url()),
        ]
    }

    pub fn on_open(&mut self) -> Vec<Action> {
        if self.phase == Phase::Connecting {
            self.phase = Phase::AwaitHello;
        }
        vec![]
    }

    pub fn on_frame(&mut self, bytes: &[u8], now: u64) -> Vec<Action> {
        let Ok(frame) = framing::decode(bytes) else {
            return self.reconnect(now);
        };
        match frame.kind {
            Some(F::Hello(h)) => {
                self.heartbeat_ms = u64::from(h.heartbeat_interval_ms.max(1_000));
                // First heartbeat after interval * jitter, so reconnect storms spread out.
                self.next_heartbeat = now + (self.heartbeat_ms as f64 * self.rng.next_f64()) as u64;
                self.acked = true;
                if let Some(sid) = self.session_id.clone() {
                    self.phase = Phase::Resuming;
                    vec![
                        Action::State(ConnectionState::Resuming, 0),
                        Action::Send(Self::frame(F::Resume(v1::Resume {
                            token: self.token.clone(),
                            session_id: sid,
                            last_seq: self.last_seq,
                        }))),
                    ]
                } else {
                    self.phase = Phase::Identifying;
                    let props = v1::ClientProperties {
                        os: std::env::consts::OS.into(),
                        client: self.client.clone(),
                        core_version: env!("CARGO_PKG_VERSION").into(),
                    };
                    vec![
                        Action::State(ConnectionState::Identifying, 0),
                        Action::Send(Self::frame(F::Identify(v1::Identify {
                            token: self.token.clone(),
                            intents: self.intents,
                            properties: Some(props),
                            initial_status: v1::Status::Online as i32,
                        }))),
                    ]
                }
            }
            Some(F::HeartbeatAck(_)) => {
                self.acked = true;
                vec![]
            }
            Some(F::Ready(r)) => {
                self.session_id = Some(r.session_id.clone());
                self.resume_url = Some(r.resume_url.clone()).filter(|u| !u.is_empty());
                self.last_seq = 0;
                self.phase = Phase::Ready;
                self.attempts = 0;
                vec![
                    Action::State(ConnectionState::Ready, 0),
                    Action::Ready(Box::new(r)),
                ]
            }
            Some(F::Resumed(_)) => {
                self.phase = Phase::Ready;
                self.attempts = 0;
                vec![Action::State(ConnectionState::Ready, 0)]
            }
            Some(F::Dispatch(batch)) => {
                let mut events = Vec::with_capacity(batch.items.len());
                for d in batch.items {
                    // seq 0 = ephemeral (typing, presence): delivered, never resumed.
                    if d.seq != 0 {
                        // Replays after resume may overlap what we already have.
                        if d.seq <= self.last_seq {
                            continue;
                        }
                        self.last_seq = d.seq;
                    }
                    if let Some(e) = d.event {
                        events.push(e);
                    }
                }
                if events.is_empty() {
                    vec![]
                } else {
                    vec![Action::Events(events)]
                }
            }
            Some(F::InvalidSession(i)) => {
                if !i.resumable {
                    self.session_id = None;
                    self.resume_url = None;
                    self.last_seq = 0;
                }
                let mut a = vec![Action::Close];
                // Wait 1–5 s before a fresh Identify (spec/gateway.md).
                let wait = 1_000 + (4_000.0 * self.rng.next_f64()) as u64;
                self.phase = Phase::Waiting { until: now + wait };
                a.push(Action::State(
                    ConnectionState::WaitingReconnect,
                    wait as u32,
                ));
                a
            }
            Some(F::Reconnect(_)) => self.reconnect(now),
            Some(F::Error(e)) => {
                let code =
                    v1::GatewayErrorCode::try_from(e.code).unwrap_or(v1::GatewayErrorCode::Unknown);
                if code == v1::GatewayErrorCode::AuthFailed || !e.reconnect {
                    self.phase = Phase::Dead;
                    let state = if code == v1::GatewayErrorCode::AuthFailed {
                        ConnectionState::AuthFailed
                    } else {
                        ConnectionState::Disconnected
                    };
                    return vec![
                        Action::Close,
                        Action::State(state, 0),
                        Action::Fatal(e.message),
                    ];
                }
                self.reconnect(now)
            }
            _ => vec![],
        }
    }

    /// Socket closed (by server, network or us).
    pub fn on_close(&mut self, now: u64) -> Vec<Action> {
        match self.phase {
            Phase::Dead | Phase::Offline | Phase::Waiting { .. } => vec![],
            _ => self.schedule_reconnect(now),
        }
    }

    fn reconnect(&mut self, now: u64) -> Vec<Action> {
        let mut a = vec![Action::Close];
        a.extend(self.schedule_reconnect(now));
        a
    }

    fn schedule_reconnect(&mut self, now: u64) -> Vec<Action> {
        // Exponential backoff with full jitter: random(0, min(cap, 1s * 2^attempts)).
        let ceiling = (1_000u64 << self.attempts.min(5)).min(MAX_BACKOFF_MS);
        let wait = if self.attempts == 0 {
            0
        } else {
            (ceiling as f64 * self.rng.next_f64()) as u64 + 250
        };
        self.attempts += 1;
        self.next_heartbeat = u64::MAX;
        self.phase = Phase::Waiting { until: now + wait };
        vec![Action::State(
            ConnectionState::WaitingReconnect,
            wait as u32,
        )]
    }

    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        match self.phase {
            Phase::Waiting { until } if now >= until => self.start(),
            Phase::AwaitHello | Phase::Identifying | Phase::Resuming | Phase::Ready
                if now >= self.next_heartbeat =>
            {
                if !self.acked {
                    // Zombie connection: no ack since the last heartbeat.
                    return self.reconnect(now);
                }
                self.acked = false;
                self.next_heartbeat = now + self.heartbeat_ms;
                vec![Action::Send(Self::frame(F::Heartbeat(v1::Heartbeat {
                    last_seq: self.last_seq,
                })))]
            }
            _ => vec![],
        }
    }

    pub fn set_online(&mut self, online: bool, now: u64) -> Vec<Action> {
        match (online, self.phase) {
            (false, Phase::Dead) => vec![],
            (false, _) => {
                self.phase = Phase::Offline;
                vec![
                    Action::Close,
                    Action::State(ConnectionState::WaitingReconnect, 0),
                ]
            }
            (true, Phase::Offline) => {
                self.attempts = 0;
                self.phase = Phase::Waiting { until: now };
                self.tick(now)
            }
            _ => vec![],
        }
    }

    pub fn shutdown(&mut self) -> Vec<Action> {
        self.phase = Phase::Dead;
        vec![
            Action::Close,
            Action::State(ConnectionState::Disconnected, 0),
        ]
    }

    /// Encode a client request (typing, presence, voice state, ack, member list).
    pub fn request(&self, kind: v1::client_request::Kind) -> Option<Action> {
        self.is_ready().then(|| {
            Action::Send(Self::frame(F::Request(v1::ClientRequest {
                kind: Some(kind),
            })))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(kind: F) -> Vec<u8> {
        framing::encode(&v1::GatewayFrame { kind: Some(kind) }, true)
    }

    fn sent(actions: &[Action]) -> Vec<v1::GatewayFrame> {
        actions
            .iter()
            .filter_map(|a| {
                if let Action::Send(b) = a {
                    framing::decode(b).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    fn hello() -> Vec<u8> {
        server(F::Hello(v1::Hello {
            heartbeat_interval_ms: 10_000,
            node: "n".into(),
        }))
    }

    fn ready(sid: &str) -> Vec<u8> {
        server(F::Ready(v1::Ready {
            session_id: sid.into(),
            resume_url: "wss://resume.example/gateway".into(),
            ..Default::default()
        }))
    }

    fn dispatch(seq: u64) -> Vec<u8> {
        server(F::Dispatch(v1::DispatchBatch {
            items: vec![v1::Dispatch {
                seq,
                event: Some(v1::Event::default()),
            }],
        }))
    }

    fn connected(m: &mut GatewayMachine) {
        assert!(matches!(m.start()[1], Action::Connect(_)));
        m.on_open();
        let a = m.on_frame(&hello(), 0);
        assert!(matches!(sent(&a)[0].kind, Some(F::Identify(_))));
        let a = m.on_frame(&ready("s1"), 0);
        assert!(a.contains(&Action::State(ConnectionState::Ready, 0)));
    }

    #[test]
    fn identify_flow_and_url() {
        let mut m = GatewayMachine::new("wss://x.example/gateway", "tok", "test", 42);
        let a = m.start();
        assert_eq!(
            a[1],
            Action::Connect("wss://x.example/gateway?v=1&compress=zstd".into())
        );
        connected(&mut m);
        assert_eq!(m.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn heartbeats_carry_seq_and_detect_zombies() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 7);
        connected(&mut m);
        m.on_frame(&dispatch(1), 0);
        m.on_frame(&dispatch(2), 0);
        let a = m.tick(10_000);
        match &sent(&a)[0].kind {
            Some(F::Heartbeat(h)) => assert_eq!(h.last_seq, 2),
            other => panic!("expected heartbeat, got {other:?}"),
        }
        // No ack before the next heartbeat → zombie → close and reconnect.
        let a = m.tick(20_000);
        assert_eq!(a[0], Action::Close);
        assert!(matches!(
            a[1],
            Action::State(ConnectionState::WaitingReconnect, _)
        ));
    }

    #[test]
    fn acked_heartbeats_keep_connection() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 7);
        connected(&mut m);
        for i in 1..5 {
            let a = m.tick(i * 10_000);
            assert!(matches!(sent(&a)[0].kind, Some(F::Heartbeat(_))));
            m.on_frame(
                &server(F::HeartbeatAck(v1::HeartbeatAck {})),
                i * 10_000 + 50,
            );
        }
    }

    #[test]
    fn resume_after_drop_uses_resume_url_and_last_seq() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 9);
        connected(&mut m);
        m.on_frame(&dispatch(5), 0);
        m.on_close(1_000);
        // First retry is immediate.
        let a = m.tick(1_000);
        assert_eq!(
            a[1],
            Action::Connect("wss://resume.example/gateway?v=1&compress=zstd".into())
        );
        m.on_open();
        let a = m.on_frame(&hello(), 1_100);
        match &sent(&a)[0].kind {
            Some(F::Resume(r)) => {
                assert_eq!(r.session_id, "s1");
                assert_eq!(r.last_seq, 5);
            }
            other => panic!("expected resume, got {other:?}"),
        }
        // Replayed events overlapping what we have are dropped.
        let a = m.on_frame(
            &server(F::Dispatch(v1::DispatchBatch {
                items: vec![
                    v1::Dispatch {
                        seq: 5,
                        event: Some(v1::Event::default()),
                    },
                    v1::Dispatch {
                        seq: 6,
                        event: Some(v1::Event::default()),
                    },
                ],
            })),
            1_200,
        );
        match &a[0] {
            Action::Events(e) => assert_eq!(e.len(), 1),
            other => panic!("{other:?}"),
        }
        let a = m.on_frame(&server(F::Resumed(v1::Resumed { replayed: 1 })), 1_300);
        assert!(a.contains(&Action::State(ConnectionState::Ready, 0)));
    }

    #[test]
    fn ephemeral_events_do_not_move_seq() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 9);
        connected(&mut m);
        m.on_frame(&dispatch(3), 0);
        let a = m.on_frame(&dispatch(0), 0);
        assert!(matches!(&a[0], Action::Events(e) if e.len() == 1));
        assert_eq!(m.last_seq, 3);
    }

    #[test]
    fn invalid_session_forces_fresh_identify() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 3);
        connected(&mut m);
        let a = m.on_frame(
            &server(F::InvalidSession(v1::InvalidSession { resumable: false })),
            0,
        );
        assert_eq!(a[0], Action::Close);
        assert!(m.session_id.is_none());
        let a = m.tick(5_001);
        assert_eq!(
            a[1],
            Action::Connect("wss://x/gateway?v=1&compress=zstd".into())
        );
        m.on_open();
        assert!(matches!(
            sent(&m.on_frame(&hello(), 5_100))[0].kind,
            Some(F::Identify(_))
        ));
    }

    #[test]
    fn auth_failure_is_fatal() {
        let mut m = GatewayMachine::new("wss://x/gateway", "bad", "c", 1);
        m.start();
        m.on_open();
        m.on_frame(&hello(), 0);
        let a = m.on_frame(
            &server(F::Error(v1::GatewayError {
                code: v1::GatewayErrorCode::AuthFailed as i32,
                message: "no".into(),
                reconnect: false,
            })),
            0,
        );
        assert!(a.contains(&Action::State(ConnectionState::AuthFailed, 0)));
        assert!(m.on_close(10).is_empty());
        assert!(m.tick(100_000).is_empty());
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 11);
        m.start();
        let mut now = 0;
        let mut waits = vec![];
        for _ in 0..10 {
            let a = m.on_close(now);
            if let Some(Action::State(_, w)) = a.first() {
                waits.push(*w as u64);
                now += *w as u64;
            }
            m.tick(now);
        }
        assert_eq!(waits[0], 0);
        assert!(waits.iter().all(|w| *w <= MAX_BACKOFF_MS + 250));
    }

    #[test]
    fn offline_then_online() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 5);
        connected(&mut m);
        let a = m.set_online(false, 0);
        assert_eq!(a[0], Action::Close);
        assert!(m.on_close(1).is_empty(), "no reconnect while offline");
        assert!(m.tick(60_000).is_empty());
        let a = m.set_online(true, 60_000);
        assert!(a.iter().any(|x| matches!(x, Action::Connect(_))));
    }

    #[test]
    fn garbage_frame_reconnects() {
        let mut m = GatewayMachine::new("wss://x/gateway", "t", "c", 5);
        connected(&mut m);
        let a = m.on_frame(&[0xFF, 0xFF, 0xFF], 0);
        assert_eq!(a[0], Action::Close);
    }

    #[test]
    fn rng_in_unit_interval() {
        let mut r = Rng::new(123);
        for _ in 0..1000 {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }
}
