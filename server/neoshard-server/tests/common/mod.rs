//! Test harness: a real NeoShard node (API + Gateway) on 127.0.0.1:0 over a
//! fresh database per test (`#[sqlx::test]`), plus HTTP and Gateway clients.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use neoshard_proto::framing;
use neoshard_proto::v1::{self, event::Kind, gateway_frame::Kind as F};
use neoshard_server_common::bus::LocalBus;
use neoshard_server_common::{Config, Ctx, SharedCtx};
use reqwest::Method;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;

pub struct Server {
    pub addr: SocketAddr,
    pub ctx: SharedCtx,
    pub http: reqwest::Client,
}

pub async fn spawn(pool: sqlx::PgPool) -> Server {
    spawn_with(pool, |_| {}).await
}

pub async fn spawn_with(pool: sqlx::PgPool, f: impl FnOnce(&mut Config)) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut cfg = Config::for_tests("postgres://unused");
    cfg.public_url = format!("http://{addr}");
    cfg.domain = addr.to_string();
    f(&mut cfg);
    let ctx = Ctx::with_pool(cfg, pool, Arc::new(LocalBus::new())).unwrap();
    let node = neoshard_server::build(ctx.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            node.router
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Server {
        addr,
        ctx,
        http: reqwest::Client::new(),
    }
}

impl Server {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1{}", self.addr, path)
    }

    pub async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut req = self.http.request(method, self.url(path));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn get(&self, path: &str, token: &str) -> (u16, Value) {
        self.call(Method::GET, path, Some(token), None).await
    }
    pub async fn post(&self, path: &str, token: &str, body: Value) -> (u16, Value) {
        self.call(Method::POST, path, Some(token), Some(body)).await
    }
    pub async fn patch(&self, path: &str, token: &str, body: Value) -> (u16, Value) {
        self.call(Method::PATCH, path, Some(token), Some(body))
            .await
    }
    pub async fn put(&self, path: &str, token: &str, body: Value) -> (u16, Value) {
        self.call(Method::PUT, path, Some(token), Some(body)).await
    }
    pub async fn delete(&self, path: &str, token: &str) -> (u16, Value) {
        self.call(Method::DELETE, path, Some(token), None).await
    }

    /// Register a user; returns (token, user id).
    pub async fn register(&self, username: &str) -> (String, String) {
        let (s, v) = self
            .call(Method::POST, "/auth/register", None, Some(json!({ "username": username, "password": "correct horse battery", "email": format!("{username}@example.ru") })))
            .await;
        assert_eq!(s, 201, "register {username}: {v}");
        (
            v["token"].as_str().unwrap().to_string(),
            v["user"]["id"].as_str().unwrap().to_string(),
        )
    }

    /// Create a guild; returns (guild id, first text channel id, voice channel id).
    pub async fn guild(&self, token: &str, name: &str) -> (String, String, String) {
        let (s, g) = self.post("/guilds", token, json!({ "name": name })).await;
        assert_eq!(s, 201, "{g}");
        let chans = g["channels"].as_array().unwrap();
        let text = chans
            .iter()
            .find(|c| c["type"] == "CHANNEL_TYPE_TEXT")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let voice = chans
            .iter()
            .find(|c| c["type"] == "CHANNEL_TYPE_VOICE")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        (g["id"].as_str().unwrap().to_string(), text, voice)
    }

    /// Invite `joiner` into the guild of `channel` via a fresh invite.
    pub async fn join(&self, owner: &str, channel: &str, joiner: &str) {
        let (s, inv) = self
            .post(&format!("/channels/{channel}/invites"), owner, json!({}))
            .await;
        assert_eq!(s, 201, "{inv}");
        let (s, v) = self
            .post(
                &format!("/invites/{}", inv["code"].as_str().unwrap()),
                joiner,
                json!({}),
            )
            .await;
        assert_eq!(s, 200, "{v}");
    }

    pub async fn gateway(&self, token: &str) -> Gw {
        Gw::connect(self.addr, token, 0).await
    }
}

/// Gateway test client with a background heartbeat.
pub struct Gw {
    pub ready: v1::Ready,
    pub last_seq: Arc<Mutex<u64>>,
    rx: mpsc::UnboundedReceiver<v1::GatewayFrame>,
    sink: Arc<
        Mutex<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
        >,
    >,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

pub fn encode(kind: F) -> Message {
    Message::Binary(framing::encode(&v1::GatewayFrame { kind: Some(kind) }, true).into())
}

impl Gw {
    pub async fn connect(addr: SocketAddr, token: &str, intents: u32) -> Gw {
        Self::open(
            addr,
            F::Identify(v1::Identify {
                token: token.into(),
                intents,
                properties: None,
                initial_status: 1,
            }),
        )
        .await
        .0
        .unwrap()
    }

    /// Open a connection and send `first`; returns the client (if Ready/Resumed) and the first non-hello frame.
    pub async fn open(addr: SocketAddr, first: F) -> (Option<Gw>, v1::GatewayFrame) {
        let (ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/gateway?v=1&compress=zstd"))
                .await
                .unwrap();
        let (mut sink, mut stream) = ws.split();
        let hello = next_frame(&mut stream).await.unwrap();
        let Some(F::Hello(h)) = hello.kind else {
            panic!("expected hello, got {hello:?}")
        };
        assert!(h.heartbeat_interval_ms > 0);
        sink.send(encode(first)).await.unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let last_seq = Arc::new(Mutex::new(0u64));
        let reader_seq = last_seq.clone();
        let first_frame = next_frame(&mut stream)
            .await
            .expect("gateway closed during handshake");
        let reader = tokio::spawn(async move {
            while let Some(f) = next_frame(&mut stream).await {
                if let Some(F::Dispatch(b)) = &f.kind
                    && let Some(last) = b.items.last()
                {
                    *reader_seq.lock().await = last.seq;
                }
                if tx.send(f).is_err() {
                    break;
                }
            }
        });
        let sink = Arc::new(Mutex::new(sink));
        let hb_sink = sink.clone();
        let hb_seq = last_seq.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let seq = *hb_seq.lock().await;
                if hb_sink
                    .lock()
                    .await
                    .send(encode(F::Heartbeat(v1::Heartbeat { last_seq: seq })))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let ready = match &first_frame.kind {
            Some(F::Ready(r)) => r.clone(),
            Some(F::Resumed(_)) | Some(F::Dispatch(_)) => v1::Ready::default(),
            _ => {
                reader.abort();
                heartbeat.abort();
                return (None, first_frame);
            }
        };
        let mut gw = Gw {
            ready,
            last_seq,
            rx,
            sink,
            tasks: vec![reader, heartbeat],
        };
        if let Some(F::Dispatch(b)) = &first_frame.kind {
            // Resume: replayed dispatches arrive before Resumed; push them back for the test.
            let (tx2, rx2) = mpsc::unbounded_channel();
            tx2.send(first_frame.clone()).unwrap();
            let mut old = std::mem::replace(&mut gw.rx, rx2);
            let fwd = tokio::spawn(async move {
                while let Some(f) = old.recv().await {
                    if tx2.send(f).is_err() {
                        break;
                    }
                }
            });
            gw.tasks.push(fwd);
            let _ = b;
        }
        (Some(gw), first_frame)
    }

    pub async fn send(&self, kind: F) {
        self.sink.lock().await.send(encode(kind)).await.unwrap();
    }

    pub async fn request(&self, r: v1::client_request::Kind) {
        self.send(F::Request(v1::ClientRequest { kind: Some(r) }))
            .await;
    }

    /// Next frame of any kind except heartbeat acks.
    pub async fn frame(&mut self, timeout: Duration) -> Option<v1::GatewayFrame> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let f = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .ok()??;
            if !matches!(f.kind, Some(F::HeartbeatAck(_))) {
                return Some(f);
            }
        }
    }

    /// Wait for the first event matching `pred`.
    pub async fn wait<T>(&mut self, mut pred: impl FnMut(&Kind) -> Option<T>) -> T {
        self.try_wait(Duration::from_secs(5), &mut pred)
            .await
            .expect("expected event did not arrive")
    }

    pub async fn try_wait<T>(
        &mut self,
        timeout: Duration,
        pred: &mut impl FnMut(&Kind) -> Option<T>,
    ) -> Option<T> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let f = self.frame(deadline - now).await?;
            if let Some(F::Dispatch(b)) = f.kind {
                for d in b.items {
                    if let Some(k) = d.event.and_then(|e| e.kind)
                        && let Some(t) = pred(&k)
                    {
                        return Some(t);
                    }
                }
            }
        }
    }

    pub async fn close(self) {
        for t in &self.tasks {
            t.abort();
        }
        let _ = self.sink.lock().await.close().await;
    }
}

async fn next_frame<S>(stream: &mut S) -> Option<v1::GatewayFrame>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match stream.next().await? {
            Ok(Message::Binary(b)) => return framing::decode(&b).ok(),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

pub fn id(v: &Value) -> u64 {
    v.as_str()
        .map(|s| s.parse().unwrap())
        .or_else(|| v.as_u64())
        .unwrap()
}
