//! The client core: commands in, updates out, effects for the driver.
//!
//! ```text
//!   UI ──Command──▶ Core ──Effect──▶ driver (WebSocket / HTTP / storage)
//!   UI ◀──Updates── Core ◀──input─── driver (frames / responses / ticks)
//! ```

use std::collections::HashMap;

use neoshard_proto::core_api::{
    self as api, AuthPhase, CacheSnapshot, ConnectionState, Update, UpdateBatch, command, update,
};
use neoshard_proto::v1::{self, event::Kind};
use prost::Message as _;

use crate::cache::Cache;
use crate::gateway::{Action, GatewayMachine};
use crate::outbox::{Outbox, OutboxEvent, Pending};

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub id: u64,
    pub method: &'static str,
    pub url: String,
    pub body: Option<String>,
    /// `Authorization` header value.
    pub auth: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub id: u64,
    /// 0 = network error
    pub status: u16,
    pub body: String,
    pub retry_after_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    WsConnect(String),
    WsSend(Vec<u8>),
    WsClose,
    Http(HttpRequest),
    /// Persist this `CacheSnapshot` (protobuf) to local storage.
    Persist(Vec<u8>),
}

#[cfg(feature = "e2ee")]
#[path = "e2ee_flow.rs"]
mod e2ee_flow;

#[derive(Debug, Clone)]
enum Purpose {
    Discovery,
    Auth,
    Api {
        request_id: u64,
    },
    Fire,
    Send {
        nonce: String,
    },
    History {
        channel_id: u64,
        limit: usize,
    },
    Plain {
        request_id: u64,
    },
    #[cfg(feature = "e2ee")]
    E2eeClaim {
        channel_id: u64,
        remaining: Vec<u64>,
        kps: Vec<Vec<u8>>,
    },
    #[cfg(feature = "e2ee")]
    E2eeWelcome {
        channel_id: u64,
    },
}

const PERSIST_EVERY_MS: u64 = 10_000;

pub struct Core {
    client_name: String,
    /// Instance base URL (`https://host`), the API lives at `{base}/api/v1`.
    base: String,
    mfa_ticket: Option<String>,
    api: String,
    gateway_url: String,
    token: String,
    machine: Option<GatewayMachine>,
    pub cache: Cache,
    outbox: Outbox,
    updates: Vec<Update>,
    effects: Vec<Effect>,
    http: HashMap<u64, Purpose>,
    next_http: u64,
    online: bool,
    selected: Option<u64>,
    dirty: bool,
    last_persist: u64,
    seed: u64,
    now: i64,
    #[cfg(feature = "e2ee")]
    e2ee: e2ee_flow::E2eeState,
    /// Builds without MLS keep the device state of an E2EE build untouched,
    /// so switching builds on one device never loses keys.
    #[cfg(not(feature = "e2ee"))]
    mls_passthrough: Vec<u8>,
}

fn upd(kind: update::Kind) -> Update {
    Update { kind: Some(kind) }
}

/// Minimal percent-encoding for path segments (emoji).
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Core {
    pub fn new(client_name: impl Into<String>, seed: u64) -> Self {
        Self {
            client_name: client_name.into(),
            base: String::new(),
            mfa_ticket: None,
            api: String::new(),
            gateway_url: String::new(),
            token: String::new(),
            machine: None,
            cache: Cache::default(),
            outbox: Outbox::default(),
            updates: Vec::new(),
            effects: Vec::new(),
            http: HashMap::new(),
            next_http: 1,
            online: true,
            selected: None,
            dirty: false,
            last_persist: 0,
            seed,
            now: 0,
            #[cfg(feature = "e2ee")]
            e2ee: Default::default(),
            #[cfg(not(feature = "e2ee"))]
            mls_passthrough: Vec::new(),
        }
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn drain_updates(&mut self) -> UpdateBatch {
        UpdateBatch {
            updates: std::mem::take(&mut self.updates),
        }
    }

    pub fn has_updates(&self) -> bool {
        !self.updates.is_empty()
    }

    /// Restore an offline snapshot: the UI gets a Ready immediately.
    pub fn restore(&mut self, bytes: &[u8]) -> bool {
        let Ok(s) = CacheSnapshot::decode(bytes) else {
            return false;
        };
        if s.token.is_empty() {
            return false;
        }
        self.cache.restore(&s);
        #[cfg(feature = "e2ee")]
        self.e2ee_import(&s.mls_state);
        #[cfg(not(feature = "e2ee"))]
        {
            self.mls_passthrough = s.mls_state.clone();
        }
        self.base = s.api_base.trim_end_matches('/').to_string();
        self.api = format!("{}/api/v1", self.base);
        self.gateway_url = s.gateway_url.clone();
        self.token = s.token.clone();
        if let Some(r) = s.ready {
            self.updates.push(upd(update::Kind::Ready(r)));
        }
        for h in s.histories {
            self.updates.push(upd(update::Kind::History(h)));
        }
        // Offline-first: show cached state, then reconnect with the stored session.
        self.logged_in(0);
        true
    }

    fn http(&mut self, purpose: Purpose, method: &'static str, path: &str, body: Option<String>) {
        let id = self.next_http;
        self.next_http += 1;
        self.http.insert(id, purpose);
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", self.api, path)
        };
        let auth = (!self.token.is_empty()).then(|| format!("Bearer {}", self.token));
        self.effects.push(Effect::Http(HttpRequest {
            id,
            method,
            url,
            body,
            auth,
        }));
    }

    fn run_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Connect(url) => self.effects.push(Effect::WsConnect(url)),
                Action::Send(b) => self.effects.push(Effect::WsSend(b)),
                Action::Close => self.effects.push(Effect::WsClose),
                Action::State(state, retry_in_ms) => {
                    self.updates
                        .push(upd(update::Kind::Connection(api::ConnectionStateChanged {
                            state: state as i32,
                            retry_in_ms,
                            reason: String::new(),
                        })));
                }
                Action::Ready(r) => {
                    self.cache.load_ready(&r);
                    self.dirty = true;
                    #[cfg(feature = "e2ee")]
                    self.e2ee_after_ready();
                    self.updates.push(upd(update::Kind::Ready(*r)));
                    self.emit_permissions();
                }
                Action::Events(events) => self.apply_events(events),
                Action::Fatal(reason) => {
                    // The server refused the session for good: say why instead of a blank screen.
                    self.updates
                        .push(upd(update::Kind::Connection(api::ConnectionStateChanged {
                            state: api::ConnectionState::Disconnected as i32,
                            retry_in_ms: 0,
                            reason,
                        })));
                }
            }
        }
    }

    fn apply_events(&mut self, events: Vec<v1::Event>) {
        let me = self.cache.me_id();
        let mut perms_changed = false;
        for e in events {
            #[allow(unused_mut)]
            let mut e = e;
            #[cfg(feature = "e2ee")]
            self.e2ee_event(&mut e);
            if !self.cache.apply(&e) {
                continue;
            }
            self.dirty = true;
            if let Some(Kind::MessageCreate(m)) = &e.kind
                && let Some(OutboxEvent::Sent { nonce, message }) =
                    self.outbox.on_gateway_message(m, me)
            {
                self.updates
                    .push(upd(update::Kind::PendingSent(api::PendingMessageSent {
                        nonce,
                        message: Some(*message),
                    })));
            }
            perms_changed |= matches!(
                e.kind,
                Some(
                    Kind::ChannelUpdate(_)
                        | Kind::RoleUpdate(_)
                        | Kind::RoleCreate(_)
                        | Kind::RoleDelete(_)
                        | Kind::MemberUpdate(_)
                        | Kind::GuildUpdate(_)
                )
            );
            self.updates.push(upd(update::Kind::Event(e)));
        }
        if perms_changed {
            self.emit_permissions();
        }
    }

    fn e2ee_channel_flag(&self, channel_id: u64) -> bool {
        self.cache.channel(channel_id).is_some_and(|c| c.e2ee)
    }

    fn emit_permissions(&mut self) {
        if let Some(cid) = self.selected {
            let p = self.cache.permissions(cid, self.now);
            self.updates
                .push(upd(update::Kind::Permissions(api::ChannelPermissions {
                    channel_id: cid,
                    permissions: p.bits(),
                })));
        }
    }

    fn auth_state(&mut self, phase: AuthPhase, error: impl Into<String>) {
        self.updates.push(upd(update::Kind::Auth(api::AuthState {
            phase: phase as i32,
            error: error.into(),
        })));
    }

    fn set_base(&mut self, base: &str) {
        self.base = base.trim_end_matches('/').to_string();
        self.api = format!("{}/api/v1", self.base);
    }

    fn logged_in(&mut self, now: u64) {
        self.mfa_ticket = None;
        self.auth_state(AuthPhase::LoggedIn, "");
        self.persist_now(now);
        let base = self.base.clone();
        self.http(
            Purpose::Discovery,
            "GET",
            &format!("{base}/.well-known/neoshard"),
            None,
        );
    }

    fn persist_now(&mut self, now: u64) {
        self.dirty = false;
        self.last_persist = now;
        let mut snap = self
            .cache
            .snapshot(&self.base, &self.gateway_url, &self.token);
        #[cfg(feature = "e2ee")]
        {
            snap.mls_state = self.e2ee_export();
        }
        #[cfg(not(feature = "e2ee"))]
        {
            snap.mls_state = self.mls_passthrough.clone();
        }
        self.effects.push(Effect::Persist(snap.encode_to_vec()));
    }

    fn result(&mut self, request_id: u64, ok: bool, error: String) {
        if request_id != 0 {
            self.updates
                .push(upd(update::Kind::Result(api::CommandResult {
                    request_id,
                    ok,
                    error,
                })));
        }
    }

    fn gateway_request(&mut self, kind: v1::client_request::Kind) {
        if let Some(Action::Send(b)) = self.machine.as_ref().and_then(|m| m.request(kind)) {
            self.effects.push(Effect::WsSend(b));
        }
    }

    pub fn command(&mut self, cmd: api::Command, now: u64) {
        self.now = now as i64;
        let rid = cmd.request_id;
        let Some(kind) = cmd.kind else { return };
        match kind {
            command::Kind::Connect(c) => {
                self.token = c.token;
                self.set_base(&c.api_base);
                if !c.gateway_url.is_empty() {
                    self.gateway_url = c.gateway_url;
                    self.start_gateway(now);
                } else {
                    let base = self.base.clone();
                    self.http(
                        Purpose::Discovery,
                        "GET",
                        &format!("{base}/.well-known/neoshard"),
                        None,
                    );
                }
                self.result(rid, true, String::new());
            }
            command::Kind::Login(l) => {
                self.set_base(&l.api_base);
                self.token.clear();
                self.auth_state(AuthPhase::Pending, "");
                let body = serde_json::json!({ "login": l.login, "password": l.password, "device": self.client_name }).to_string();
                self.http(Purpose::Auth, "POST", "/auth/login", Some(body));
            }
            command::Kind::Register(r) => {
                self.set_base(&r.api_base);
                self.token.clear();
                self.auth_state(AuthPhase::Pending, "");
                let mut body = serde_json::json!({ "username": r.username, "password": r.password, "device": self.client_name });
                if !r.email.is_empty() {
                    body["email"] = serde_json::json!(r.email);
                }
                self.http(
                    Purpose::Auth,
                    "POST",
                    "/auth/register",
                    Some(body.to_string()),
                );
            }
            command::Kind::Mfa(m) => {
                let Some(ticket) = self.mfa_ticket.clone() else {
                    self.auth_state(AuthPhase::LoggedOut, "сначала войдите по паролю");
                    return;
                };
                self.auth_state(AuthPhase::Pending, "");
                let body = serde_json::json!({ "ticket": ticket, "code": m.code }).to_string();
                self.http(Purpose::Auth, "POST", "/auth/mfa/totp", Some(body));
            }
            command::Kind::Api(a) => {
                let method = match a.method.to_ascii_uppercase().as_str() {
                    "POST" => "POST",
                    "PUT" => "PUT",
                    "PATCH" => "PATCH",
                    "DELETE" => "DELETE",
                    _ => "GET",
                };
                if !a.path.starts_with('/') {
                    self.updates.push(upd(update::Kind::Api(api::ApiResponse {
                        request_id: rid,
                        status: 0,
                        body_json: r#"{"message":"path must start with /"}"#.into(),
                    })));
                    return;
                }
                let body = (!a.body_json.is_empty()).then_some(a.body_json);
                self.http(Purpose::Api { request_id: rid }, method, &a.path, body);
            }
            command::Kind::Logout(_) => {
                if !self.token.is_empty() {
                    self.http(Purpose::Fire, "POST", "/auth/logout", None);
                }
                if let Some(m) = self.machine.as_mut() {
                    let a = m.shutdown();
                    self.run_actions(a);
                }
                self.machine = None;
                self.token.clear();
                self.cache = Cache::default();
                #[cfg(feature = "e2ee")]
                {
                    self.e2ee = Default::default();
                }
                #[cfg(not(feature = "e2ee"))]
                self.mls_passthrough.clear();
                self.outbox = Outbox::default();
                self.auth_state(AuthPhase::LoggedOut, "");
                self.persist_now(now);
            }
            command::Kind::Disconnect(_) => {
                if let Some(m) = self.machine.as_mut() {
                    let a = m.shutdown();
                    self.run_actions(a);
                }
                self.result(rid, true, String::new());
            }
            command::Kind::SendMessage(s) => {
                let content = s.content.trim_end().to_string();
                let has_files = !s.attachment_ids.is_empty();
                if (content.is_empty() && !has_files && s.sticker_id == 0)
                    || content.chars().count() > 4000
                {
                    self.result(rid, false, "message must be 1-4000 characters".into());
                    return;
                }
                if s.attachment_ids.len() > 10 {
                    self.result(rid, false, "at most 10 attachments".into());
                    return;
                }
                let nonce = self.outbox.new_nonce(now);
                let preview = v1::Message {
                    id: 0,
                    channel_id: s.channel_id,
                    guild_id: self
                        .cache
                        .channel(s.channel_id)
                        .map(|c| c.guild_id)
                        .unwrap_or(0),
                    author: self.cache.me.clone(),
                    content: content.clone(),
                    reply_to_id: s.reply_to_id,
                    nonce: nonce.clone(),
                    ..Default::default()
                };
                self.updates
                    .push(upd(update::Kind::PendingMessage(api::PendingMessage {
                        nonce: nonce.clone(),
                        message: Some(preview),
                    })));
                if self.e2ee_channel_flag(s.channel_id) {
                    #[cfg(feature = "e2ee")]
                    {
                        self.e2ee_send(
                            s.channel_id,
                            nonce,
                            content,
                            s.reply_to_id,
                            s.attachment_ids.clone(),
                            now,
                        );
                        self.result(rid, true, String::new());
                    }
                    #[cfg(not(feature = "e2ee"))]
                    {
                        let _ = content;
                        self.updates.push(upd(update::Kind::PendingFailed(
                            api::PendingMessageFailed {
                                nonce,
                                channel_id: s.channel_id,
                                reason: "this build has no end-to-end encryption".into(),
                                retryable: false,
                            },
                        )));
                        self.result(rid, false, "no e2ee in this build".into());
                    }
                    return;
                }
                self.outbox.enqueue(Pending {
                    nonce,
                    channel_id: s.channel_id,
                    content,
                    reply_to_id: s.reply_to_id,
                    e2ee_payload: None,
                    attachment_ids: s.attachment_ids.clone(),
                    sticker_id: s.sticker_id,
                    attempts: 0,
                    not_before: 0,
                    in_flight: false,
                    failed: false,
                });
                self.pump_outbox(now);
                self.result(rid, true, String::new());
            }
            command::Kind::RetryMessage(r) => {
                let ok = self.outbox.retry(&r.nonce);
                self.pump_outbox(now);
                self.result(
                    rid,
                    ok,
                    if ok {
                        String::new()
                    } else {
                        "nothing to retry".into()
                    },
                );
            }
            command::Kind::EditMessage(e) => {
                let body = serde_json::json!({ "content": e.content }).to_string();
                self.http(
                    Purpose::Plain { request_id: rid },
                    "PATCH",
                    &format!("/channels/{}/messages/{}", e.channel_id, e.message_id),
                    Some(body),
                );
            }
            command::Kind::DeleteMessage(d) => {
                self.http(
                    Purpose::Plain { request_id: rid },
                    "DELETE",
                    &format!("/channels/{}/messages/{}", d.channel_id, d.message_id),
                    None,
                );
            }
            command::Kind::React(r) => {
                let path = format!(
                    "/channels/{}/messages/{}/reactions/{}/@me",
                    r.channel_id,
                    r.message_id,
                    encode_segment(&r.emoji)
                );
                self.http(
                    Purpose::Plain { request_id: rid },
                    if r.add { "PUT" } else { "DELETE" },
                    &path,
                    None,
                );
            }
            command::Kind::LoadHistory(h) => {
                let limit = h.limit.clamp(1, 100) as usize;
                if let Some(hist) = self.cache.history(h.channel_id, h.before_id, limit) {
                    self.updates.push(upd(update::Kind::History(hist)));
                    self.result(rid, true, String::new());
                } else {
                    let mut path = format!("/channels/{}/messages?limit={limit}", h.channel_id);
                    if h.before_id != 0 {
                        path.push_str(&format!("&before={}", h.before_id));
                    }
                    self.http(
                        Purpose::History {
                            channel_id: h.channel_id,
                            limit,
                        },
                        "GET",
                        &path,
                        None,
                    );
                    self.result(rid, true, String::new());
                }
            }
            command::Kind::SetTyping(t) => {
                self.gateway_request(v1::client_request::Kind::Typing(v1::TypingSend {
                    channel_id: t.channel_id,
                }))
            }
            command::Kind::AckChannel(a) => {
                self.cache.read_states.insert(
                    a.channel_id,
                    v1::ReadState {
                        channel_id: a.channel_id,
                        last_read_message_id: a.message_id,
                        mention_count: 0,
                    },
                );
                self.gateway_request(v1::client_request::Kind::Ack(v1::AckChannel {
                    channel_id: a.channel_id,
                    message_id: a.message_id,
                }));
            }
            command::Kind::Members(m) => {
                let end = m.end.max(m.start).min(m.start + 200);
                self.gateway_request(v1::client_request::Kind::MemberList(
                    v1::MemberListRequest {
                        guild_id: m.guild_id,
                        start: m.start,
                        end,
                    },
                ));
            }
            command::Kind::Voice(v) => {
                self.gateway_request(v1::client_request::Kind::VoiceState(v1::VoiceStateSet {
                    guild_id: v.guild_id,
                    channel_id: v.channel_id,
                    self_mute: v.self_mute,
                    self_deaf: v.self_deaf,
                    video: v.video,
                    streaming: v.streaming,
                }));
            }
            command::Kind::SetPresence(p) => {
                self.gateway_request(v1::client_request::Kind::PresenceSet(v1::PresenceSet {
                    status: p.status,
                    custom_status: p.custom_status,
                    custom_emoji: p.custom_emoji,
                    custom_expires_at: p.custom_expires_at,
                }));
            }
            command::Kind::SelectChannel(s) => {
                self.selected = Some(s.channel_id).filter(|c| *c != 0);
                self.emit_permissions();
            }
            command::Kind::Network(n) => {
                self.online = n.online;
                if let Some(m) = self.machine.as_mut() {
                    let a = m.set_online(n.online, now);
                    self.run_actions(a);
                }
                self.pump_outbox(now);
            }
        }
    }

    fn start_gateway(&mut self, now: u64) {
        let mut m = GatewayMachine::new(
            self.gateway_url.clone(),
            self.token.clone(),
            self.client_name.clone(),
            self.seed ^ now,
        );
        let a = m.start();
        self.machine = Some(m);
        self.run_actions(a);
    }

    fn pump_outbox(&mut self, now: u64) {
        for ev in self.outbox.ready(now, self.online) {
            if let OutboxEvent::Send(p) = ev {
                let mut body = match &p.e2ee_payload {
                    Some(ct) => {
                        serde_json::json!({ "content": "", "e2ee_payload": ct, "nonce": p.nonce })
                    }
                    None => serde_json::json!({ "content": p.content, "nonce": p.nonce }),
                };
                if p.reply_to_id != 0 {
                    body["reply_to_id"] = serde_json::json!(p.reply_to_id.to_string());
                }
                if p.sticker_id != 0 {
                    body["sticker_id"] = serde_json::json!(p.sticker_id.to_string());
                }
                if !p.attachment_ids.is_empty() {
                    body["attachment_ids"] = serde_json::json!(
                        p.attachment_ids
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                    );
                }
                self.http(
                    Purpose::Send {
                        nonce: p.nonce.clone(),
                    },
                    "POST",
                    &format!("/channels/{}/messages", p.channel_id),
                    Some(body.to_string()),
                );
            }
        }
    }

    // ---------------------------------------------------------------- driver inputs

    pub fn on_ws_open(&mut self, now: u64) {
        if let Some(m) = self.machine.as_mut() {
            let a = m.on_open();
            self.run_actions(a);
        }
        let _ = now;
    }

    pub fn on_ws_frame(&mut self, bytes: &[u8], now: u64) {
        self.now = now as i64;
        if let Some(m) = self.machine.as_mut() {
            let a = m.on_frame(bytes, now);
            self.run_actions(a);
        }
    }

    pub fn on_ws_close(&mut self, now: u64) {
        if let Some(m) = self.machine.as_mut() {
            let a = m.on_close(now);
            self.run_actions(a);
        }
    }

    pub fn on_http(&mut self, r: HttpResponse, now: u64) {
        self.now = now as i64;
        let Some(purpose) = self.http.remove(&r.id) else {
            return;
        };
        match purpose {
            Purpose::Fire => {}
            #[cfg(feature = "e2ee")]
            p @ (Purpose::E2eeClaim { .. } | Purpose::E2eeWelcome { .. }) => {
                self.e2ee_on_http(p, r.status, &r.body, now)
            }
            Purpose::Auth => {
                let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
                if (200..300).contains(&r.status) {
                    if v["mfa"] == true {
                        self.mfa_ticket = v["ticket"].as_str().map(str::to_owned);
                        self.auth_state(AuthPhase::MfaRequired, "");
                    } else if let Some(t) = v["token"].as_str() {
                        self.token = t.to_string();
                        self.logged_in(now);
                    } else {
                        self.auth_state(AuthPhase::LoggedOut, "неожиданный ответ сервера");
                    }
                } else {
                    let msg = match (r.status, v["code"].as_str()) {
                        (0, _) => "сервер недоступен".to_string(),
                        (401, _) => "неверный логин, пароль или код".to_string(),
                        (409, _) => "имя пользователя или email уже заняты".to_string(),
                        (429, _) => "слишком много попыток, подождите".to_string(),
                        _ => v["message"].as_str().unwrap_or("ошибка").to_string(),
                    };
                    let phase = if self.mfa_ticket.is_some() && r.status == 401 {
                        AuthPhase::MfaRequired
                    } else {
                        AuthPhase::LoggedOut
                    };
                    self.auth_state(phase, msg);
                }
            }
            Purpose::Api { request_id } => {
                self.updates.push(upd(update::Kind::Api(api::ApiResponse {
                    request_id,
                    status: u32::from(r.status),
                    body_json: r.body,
                })));
            }
            Purpose::Discovery => {
                let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
                match (r.status, v["gateway"].as_str(), v["api"].as_str()) {
                    (200, Some(gw), Some(api)) => {
                        self.gateway_url = gw.to_string();
                        self.api = api.trim_end_matches('/').to_string();
                        self.start_gateway(now);
                    }
                    _ if !self.gateway_url.is_empty() => self.start_gateway(now), // offline: cached endpoints
                    _ => {
                        self.updates.push(upd(update::Kind::Connection(
                            api::ConnectionStateChanged {
                                state: ConnectionState::Disconnected as i32,
                                retry_in_ms: 0,
                                reason: "server discovery failed".into(),
                            },
                        )));
                    }
                }
            }
            Purpose::Send { nonce } => {
                for ev in self
                    .outbox
                    .on_response(&nonce, r.status, &r.body, r.retry_after_ms, now)
                {
                    match ev {
                        OutboxEvent::Sent { nonce, message } => {
                            #[allow(unused_mut)]
                            let mut message = message;
                            #[cfg(feature = "e2ee")]
                            self.e2ee_decrypt(&mut message);
                            self.cache.put_message((*message).clone());
                            self.updates.push(upd(update::Kind::PendingSent(
                                api::PendingMessageSent {
                                    nonce,
                                    message: Some(*message),
                                },
                            )));
                        }
                        OutboxEvent::Failed {
                            nonce,
                            channel_id,
                            reason,
                            retryable,
                        } => {
                            self.updates.push(upd(update::Kind::PendingFailed(
                                api::PendingMessageFailed {
                                    nonce,
                                    channel_id,
                                    reason,
                                    retryable,
                                },
                            )));
                        }
                        OutboxEvent::Send(_) => {}
                    }
                }
            }
            Purpose::History { channel_id, limit } => {
                if r.status == 200 {
                    #[allow(unused_mut)]
                    let mut page: Vec<v1::Message> =
                        serde_json::from_str(&r.body).unwrap_or_default();
                    #[cfg(feature = "e2ee")]
                    for m in page.iter_mut() {
                        self.e2ee_decrypt(m);
                    }
                    self.cache.merge_history(channel_id, page.clone(), limit);
                    let reached_start = page.len() < limit;
                    self.updates.push(upd(update::Kind::History(api::History {
                        channel_id,
                        messages: page,
                        reached_start,
                    })));
                }
            }
            Purpose::Plain { request_id } => {
                let ok = (200..300).contains(&r.status);
                let err = if ok {
                    String::new()
                } else {
                    serde_json::from_str::<serde_json::Value>(&r.body)
                        .ok()
                        .and_then(|v| v["message"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| format!("http {}", r.status))
                };
                self.result(request_id, ok, err);
            }
        }
    }

    /// Call every ~100–250 ms.
    pub fn tick(&mut self, now: u64) {
        self.now = now as i64;
        if let Some(m) = self.machine.as_mut() {
            let a = m.tick(now);
            self.run_actions(a);
        }
        self.pump_outbox(now);
        if self.dirty
            && !self.token.is_empty()
            && now.saturating_sub(self.last_persist) >= PERSIST_EVERY_MS
        {
            self.persist_now(now);
        }
    }

    pub fn is_ready(&self) -> bool {
        self.machine.as_ref().is_some_and(|m| m.is_ready())
    }

    pub fn pending_sends(&self) -> usize {
        self.outbox.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoshard_proto::framing;
    use neoshard_proto::v1::gateway_frame::Kind as F;

    fn cmd(kind: command::Kind) -> api::Command {
        api::Command {
            request_id: 1,
            kind: Some(kind),
        }
    }

    fn frame(kind: F) -> Vec<u8> {
        framing::encode(&v1::GatewayFrame { kind: Some(kind) }, false)
    }

    fn connected() -> Core {
        let mut c = Core::new("test", 1);
        c.command(
            cmd(command::Kind::Connect(api::Connect {
                api_base: "https://x.ru".into(),
                gateway_url: "wss://x.ru/gateway".into(),
                token: "tok".into(),
            })),
            0,
        );
        assert!(
            c.take_effects()
                .iter()
                .any(|e| matches!(e, Effect::WsConnect(u) if u.starts_with("wss://x.ru/gateway")))
        );
        c.on_ws_open(0);
        c.on_ws_frame(
            &frame(F::Hello(v1::Hello {
                heartbeat_interval_ms: 30_000,
                node: String::new(),
            })),
            0,
        );
        let me = v1::User {
            id: 5,
            username: "me".into(),
            ..Default::default()
        };
        let ch = v1::Channel {
            id: 10,
            guild_id: 1,
            r#type: 1,
            ..Default::default()
        };
        let g = v1::Guild {
            id: 1,
            channels: vec![ch],
            roles: vec![v1::Role {
                id: 1,
                guild_id: 1,
                permissions: neoshard_proto::permissions::Permissions::DEFAULT_EVERYONE.bits(),
                ..Default::default()
            }],
            self_member: Some(v1::Member {
                guild_id: 1,
                user: Some(me.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };
        c.on_ws_frame(
            &frame(F::Ready(v1::Ready {
                session_id: "s".into(),
                user: Some(me),
                guilds: vec![g],
                ..Default::default()
            })),
            0,
        );
        c.take_effects();
        c.drain_updates();
        c
    }

    #[test]
    fn discovery_then_gateway() {
        let mut c = Core::new("t", 1);
        c.command(
            cmd(command::Kind::Connect(api::Connect {
                api_base: "https://x.ru/".into(),
                gateway_url: String::new(),
                token: "t".into(),
            })),
            0,
        );
        let eff = c.take_effects();
        let Effect::Http(req) = &eff[0] else {
            panic!("{eff:?}")
        };
        assert_eq!(req.url, "https://x.ru/.well-known/neoshard");
        c.on_http(
            HttpResponse {
                id: req.id,
                status: 200,
                body: r#"{"gateway":"wss://gw.x.ru/gateway","api":"https://x.ru/api/v1"}"#.into(),
                retry_after_ms: 0,
            },
            1,
        );
        assert!(
            c.take_effects().iter().any(
                |e| matches!(e, Effect::WsConnect(u) if u.starts_with("wss://gw.x.ru/gateway"))
            )
        );
    }

    #[test]
    fn optimistic_send_resolves_via_http() {
        let mut c = connected();
        c.command(
            cmd(command::Kind::SendMessage(api::SendMessage {
                channel_id: 10,
                content: "hi".into(),
                reply_to_id: 0,
                attachment_ids: vec![],
                sticker_id: 0,
            })),
            10,
        );
        let ups = c.drain_updates().updates;
        let nonce = match &ups[0].kind {
            Some(update::Kind::PendingMessage(p)) => p.nonce.clone(),
            other => panic!("{other:?}"),
        };
        let eff = c.take_effects();
        let Effect::Http(req) = &eff[0] else { panic!() };
        assert_eq!(
            (req.method, req.url.as_str()),
            ("POST", "https://x.ru/api/v1/channels/10/messages")
        );
        assert_eq!(req.auth.as_deref(), Some("Bearer tok"));
        assert!(req.body.as_ref().unwrap().contains(&nonce));
        c.on_http(
            HttpResponse {
                id: req.id,
                status: 201,
                body: format!(
                    r#"{{"id":"77","channel_id":"10","content":"hi","nonce":"{nonce}"}}"#
                ),
                retry_after_ms: 0,
            },
            20,
        );
        let ups = c.drain_updates().updates;
        assert!(
            matches!(&ups[0].kind, Some(update::Kind::PendingSent(s)) if s.nonce == nonce && s.message.as_ref().unwrap().id == 77)
        );
        assert_eq!(c.pending_sends(), 0);
    }

    #[test]
    fn send_failure_reported() {
        let mut c = connected();
        c.command(
            cmd(command::Kind::SendMessage(api::SendMessage {
                channel_id: 10,
                content: "x".into(),
                reply_to_id: 0,
                attachment_ids: vec![],
                sticker_id: 0,
            })),
            0,
        );
        let Effect::Http(req) = c.take_effects().remove(0) else {
            panic!()
        };
        c.drain_updates();
        c.on_http(
            HttpResponse {
                id: req.id,
                status: 403,
                body: r#"{"code":"missing_permissions","message":"missing permissions"}"#.into(),
                retry_after_ms: 0,
            },
            1,
        );
        let ups = c.drain_updates().updates;
        assert!(
            matches!(&ups[0].kind, Some(update::Kind::PendingFailed(f)) if !f.retryable && f.reason == "missing permissions")
        );
    }

    #[test]
    fn fatal_gateway_error_reaches_the_ui() {
        let mut c = connected();
        c.drain_updates();
        c.on_ws_frame(
            &frame(F::Error(v1::GatewayError {
                code: v1::GatewayErrorCode::RateLimited as i32,
                message: "too many sessions".into(),
                reconnect: false,
            })),
            5,
        );
        let ups = c.drain_updates().updates;
        assert!(
            ups.iter().any(|u| matches!(&u.kind, Some(update::Kind::Connection(s)) if s.reason == "too many sessions" && s.state == api::ConnectionState::Disconnected as i32)),
            "{ups:?}"
        );
    }

    #[test]
    fn sends_attachments_without_text() {
        let mut c = connected();
        c.command(
            cmd(command::Kind::SendMessage(api::SendMessage {
                channel_id: 10,
                content: "".into(),
                reply_to_id: 0,
                attachment_ids: vec![501, 502],
                sticker_id: 0,
            })),
            10,
        );
        let eff = c.take_effects();
        let Effect::Http(req) = &eff[0] else {
            panic!("{eff:?}")
        };
        let body: serde_json::Value = serde_json::from_str(req.body.as_ref().unwrap()).unwrap();
        assert_eq!(body["attachment_ids"], serde_json::json!(["501", "502"]));
        assert_eq!(body["content"], "");
    }

    #[test]
    fn rejects_empty_messages_locally() {
        let mut c = connected();
        c.command(
            cmd(command::Kind::SendMessage(api::SendMessage {
                channel_id: 10,
                content: "   ".into(),
                reply_to_id: 0,
                attachment_ids: vec![],
                sticker_id: 0,
            })),
            0,
        );
        assert!(c.take_effects().is_empty());
        assert!(
            matches!(&c.drain_updates().updates[0].kind, Some(update::Kind::Result(r)) if !r.ok)
        );
    }

    #[test]
    fn events_flow_to_updates_and_cache() {
        let mut c = connected();
        let m = v1::Message {
            id: 3,
            channel_id: 10,
            guild_id: 1,
            content: "yo".into(),
            ..Default::default()
        };
        let batch = v1::DispatchBatch {
            items: vec![v1::Dispatch {
                seq: 1,
                event: Some(v1::Event {
                    kind: Some(Kind::MessageCreate(m)),
                }),
            }],
        };
        c.on_ws_frame(&frame(F::Dispatch(batch)), 5);
        let ups = c.drain_updates().updates;
        assert!(matches!(&ups[0].kind, Some(update::Kind::Event(_))));
        assert_eq!(c.cache.messages[&10].len(), 1);
    }

    #[test]
    fn history_served_from_cache_or_http() {
        let mut c = connected();
        c.command(
            cmd(command::Kind::LoadHistory(api::LoadHistory {
                channel_id: 10,
                before_id: 0,
                limit: 50,
            })),
            0,
        );
        let Effect::Http(req) = c.take_effects().remove(0) else {
            panic!()
        };
        assert!(req.url.ends_with("/channels/10/messages?limit=50"));
        c.on_http(
            HttpResponse {
                id: req.id,
                status: 200,
                body: r#"[{"id":"1","channel_id":"10"},{"id":"2","channel_id":"10"}]"#.into(),
                retry_after_ms: 0,
            },
            1,
        );
        let ups = c.drain_updates().updates;
        assert!(ups.iter().any(|u| matches!(&u.kind, Some(update::Kind::History(h)) if h.messages.len() == 2 && h.reached_start)));
        // Second time: from cache, no HTTP.
        c.command(
            cmd(command::Kind::LoadHistory(api::LoadHistory {
                channel_id: 10,
                before_id: 0,
                limit: 50,
            })),
            2,
        );
        assert!(c.take_effects().is_empty());
    }

    #[test]
    fn select_channel_emits_permissions() {
        let mut c = connected();
        c.command(
            cmd(command::Kind::SelectChannel(api::SelectChannel {
                channel_id: 10,
            })),
            0,
        );
        let ups = c.drain_updates().updates;
        let p = ups
            .iter()
            .find_map(|u| {
                if let Some(update::Kind::Permissions(p)) = &u.kind {
                    Some(p.permissions)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            neoshard_proto::permissions::Permissions::raw(p)
                .contains(neoshard_proto::permissions::Permissions::SEND_MESSAGES)
        );
    }

    #[test]
    fn persists_and_restores_offline() {
        let mut c = connected();
        let m = v1::Message {
            id: 3,
            channel_id: 10,
            guild_id: 1,
            content: "yo".into(),
            ..Default::default()
        };
        c.on_ws_frame(
            &frame(F::Dispatch(v1::DispatchBatch {
                items: vec![v1::Dispatch {
                    seq: 1,
                    event: Some(v1::Event {
                        kind: Some(Kind::MessageCreate(m)),
                    }),
                }],
            })),
            0,
        );
        c.tick(PERSIST_EVERY_MS + 1);
        let snap = c
            .take_effects()
            .into_iter()
            .find_map(|e| {
                if let Effect::Persist(b) = e {
                    Some(b)
                } else {
                    None
                }
            })
            .expect("persist");
        let mut c2 = Core::new("t", 2);
        assert!(c2.restore(&snap));
        let ups = c2.drain_updates().updates;
        assert!(matches!(&ups[0].kind, Some(update::Kind::Ready(r)) if r.guilds.len() == 1));
        assert!(ups.iter().any(|u| matches!(&u.kind, Some(update::Kind::Auth(a)) if a.phase == AuthPhase::LoggedIn as i32)));
        // Restoring reconnects with the stored session.
        assert!(
            c2.take_effects().iter().any(
                |e| matches!(e, Effect::Http(r) if r.url == "https://x.ru/.well-known/neoshard")
            )
        );
        assert!(
            ups.iter().any(
                |u| matches!(&u.kind, Some(update::Kind::History(h)) if h.messages.len() == 1)
            )
        );
    }

    /// A build without MLS must carry an E2EE build's keys through untouched.
    #[cfg(not(feature = "e2ee"))]
    #[test]
    fn keeps_foreign_mls_state() {
        let mut c = connected();
        c.tick(PERSIST_EVERY_MS + 1);
        let persisted = |c: &mut Core| {
            c.take_effects().into_iter().find_map(|e| {
                if let Effect::Persist(b) = e {
                    Some(b)
                } else {
                    None
                }
            })
        };
        let mut snap =
            CacheSnapshot::decode(persisted(&mut c).expect("persist").as_slice()).unwrap();
        snap.mls_state = vec![1, 2, 3];
        let mut c2 = Core::new("t", 2);
        assert!(c2.restore(&snap.encode_to_vec()));
        c2.persist_now(1);
        let again = CacheSnapshot::decode(persisted(&mut c2).expect("persist").as_slice()).unwrap();
        assert_eq!(again.mls_state, vec![1, 2, 3]);
    }

    #[test]
    fn login_with_mfa_then_logout() {
        let mut c = Core::new("t", 1);
        c.command(
            cmd(command::Kind::Login(api::Login {
                api_base: "https://x.ru".into(),
                login: "a".into(),
                password: "p".into(),
            })),
            0,
        );
        let Effect::Http(req) = c.take_effects().remove(0) else {
            panic!()
        };
        assert_eq!(req.url, "https://x.ru/api/v1/auth/login");
        assert!(req.auth.is_none());
        c.on_http(
            HttpResponse {
                id: req.id,
                status: 200,
                body: r#"{"mfa":true,"ticket":"nst_1"}"#.into(),
                retry_after_ms: 0,
            },
            1,
        );
        assert!(c.drain_updates().updates.iter().any(|u| matches!(&u.kind, Some(update::Kind::Auth(a)) if a.phase == AuthPhase::MfaRequired as i32)));
        c.command(
            cmd(command::Kind::Mfa(api::SubmitMfa {
                code: "123456".into(),
            })),
            2,
        );
        let Effect::Http(req) = c.take_effects().remove(0) else {
            panic!()
        };
        assert!(req.body.unwrap().contains("nst_1"));
        c.on_http(
            HttpResponse {
                id: req.id,
                status: 200,
                body: r#"{"token":"ns_tok","user":{}}"#.into(),
                retry_after_ms: 0,
            },
            3,
        );
        let eff = c.take_effects();
        assert!(
            eff.iter().any(|e| matches!(e, Effect::Persist(_))),
            "session persisted"
        );
        assert!(
            eff.iter()
                .any(|e| matches!(e, Effect::Http(r) if r.url.ends_with("/.well-known/neoshard")))
        );
        c.command(cmd(command::Kind::Logout(api::Logout {})), 4);
        let eff = c.take_effects();
        let snap = eff
            .iter()
            .find_map(|e| {
                if let Effect::Persist(b) = e {
                    Some(b.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            !Core::new("t", 1).restore(&snap),
            "logged-out snapshot has no session"
        );
    }

    #[test]
    fn api_passthrough() {
        let mut c = connected();
        c.command(
            api::Command {
                request_id: 9,
                kind: Some(command::Kind::Api(api::ApiRequest {
                    method: "post".into(),
                    path: "/guilds".into(),
                    body_json: r#"{"name":"x"}"#.into(),
                })),
            },
            0,
        );
        let Effect::Http(req) = c.take_effects().remove(0) else {
            panic!()
        };
        assert_eq!(
            (req.method, req.url.as_str(), req.auth.as_deref()),
            ("POST", "https://x.ru/api/v1/guilds", Some("Bearer tok"))
        );
        c.on_http(
            HttpResponse {
                id: req.id,
                status: 201,
                body: "{}".into(),
                retry_after_ms: 0,
            },
            1,
        );
        assert!(
            matches!(&c.drain_updates().updates[0].kind, Some(update::Kind::Api(a)) if a.request_id == 9 && a.status == 201)
        );
    }

    #[test]
    fn emoji_segments() {
        assert_eq!(encode_segment("👍"), "%F0%9F%91%8D");
        assert_eq!(encode_segment("party:123"), "party:123");
    }
}
