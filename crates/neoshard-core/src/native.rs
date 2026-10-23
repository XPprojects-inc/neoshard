//! Native driver: runs a [`Core`] on tokio with real WebSocket/HTTP and a
//! SQLite cache. Used by the CLI, the Tauri desktop app and mobile (uniffi).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use neoshard_proto::core_api::{Command, UpdateBatch};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::core::{Core, Effect, HttpRequest, HttpResponse};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Key/value store for the offline snapshot.
pub trait Store: Send + Sync + 'static {
    fn load(&self) -> Option<Vec<u8>>;
    fn save(&self, bytes: &[u8]);
}

pub struct MemoryStore(Mutex<Option<Vec<u8>>>);

impl MemoryStore {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for MemoryStore {
    fn load(&self) -> Option<Vec<u8>> {
        self.0.lock().ok()?.clone()
    }
    fn save(&self, bytes: &[u8]) {
        if let Ok(mut g) = self.0.lock() {
            *g = Some(bytes.to_vec());
        }
    }
}

pub struct SqliteStore(Mutex<rusqlite::Connection>);

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let c = rusqlite::Connection::open(path)?;
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v BLOB NOT NULL);")?;
        Ok(Self(Mutex::new(c)))
    }
}

impl Store for SqliteStore {
    fn load(&self) -> Option<Vec<u8>> {
        let c = self.0.lock().ok()?;
        c.query_row("SELECT v FROM kv WHERE k = 'snapshot'", [], |r| r.get(0))
            .ok()
    }
    fn save(&self, bytes: &[u8]) {
        if let Ok(c) = self.0.lock() {
            let _ = c.execute("INSERT INTO kv (k, v) VALUES ('snapshot', ?1) ON CONFLICT(k) DO UPDATE SET v = excluded.v", [bytes]);
        }
    }
}

enum Input {
    Cmd(Command),
    WsOpen(u64),
    WsFrame(u64, Vec<u8>),
    WsClosed(u64),
    Http(HttpResponse),
}

/// Handle to a running core. Drop it (or send Disconnect) to stop.
pub struct NativeClient {
    tx: mpsc::UnboundedSender<Input>,
    pub updates: mpsc::UnboundedReceiver<UpdateBatch>,
}

impl NativeClient {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Input::Cmd(cmd));
    }

    /// Spawn on the current tokio runtime.
    pub fn spawn(client_name: &str, store: Arc<dyn Store>) -> NativeClient {
        let (tx, mut rx) = mpsc::unbounded_channel::<Input>();
        let (utx, updates) = mpsc::unbounded_channel();
        let mut core = Core::new(client_name, now_ms());
        if let Some(snap) = store.load() {
            core.restore(&snap);
        }
        let itx = tx.clone();
        tokio::spawn(async move {
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client");
            let mut ws_out: Option<mpsc::UnboundedSender<Option<Vec<u8>>>> = None;
            let mut generation = 0u64;
            let mut ticker = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    input = rx.recv() => {
                        let Some(input) = input else { break };
                        let now = now_ms();
                        match input {
                            Input::Cmd(c) => core.command(c, now),
                            Input::WsOpen(g) if g == generation => core.on_ws_open(now),
                            Input::WsFrame(g, b) if g == generation => core.on_ws_frame(&b, now),
                            Input::WsClosed(g) if g == generation => { ws_out = None; core.on_ws_close(now) }
                            Input::Http(r) => core.on_http(r, now),
                            _ => {} // stale connection
                        }
                    }
                    _ = ticker.tick() => core.tick(now_ms()),
                }
                for eff in core.take_effects() {
                    match eff {
                        Effect::WsConnect(url) => {
                            if let Some(old) = ws_out.take() {
                                let _ = old.send(None);
                            }
                            generation += 1;
                            ws_out = Some(spawn_ws(url, generation, itx.clone()));
                        }
                        Effect::WsSend(b) => {
                            if let Some(o) = &ws_out {
                                let _ = o.send(Some(b));
                            }
                        }
                        Effect::WsClose => {
                            if let Some(o) = ws_out.take() {
                                let _ = o.send(None);
                            }
                            // The socket task reports WsClosed for the current generation; bump so
                            // that the machine only sees the close it asked for once.
                            generation += 1;
                        }
                        Effect::Http(req) => spawn_http(http.clone(), req, itx.clone()),
                        Effect::Persist(bytes) => {
                            let store = store.clone();
                            tokio::task::spawn_blocking(move || store.save(&bytes));
                        }
                    }
                }
                if core.has_updates() && utx.send(core.drain_updates()).is_err() {
                    break;
                }
            }
        });
        NativeClient { tx, updates }
    }
}

fn spawn_ws(
    url: String,
    generation: u64,
    tx: mpsc::UnboundedSender<Input>,
) -> mpsc::UnboundedSender<Option<Vec<u8>>> {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
    tokio::spawn(async move {
        let Ok((ws, _)) = tokio_tungstenite::connect_async(url.as_str()).await else {
            let _ = tx.send(Input::WsClosed(generation));
            return;
        };
        let _ = tx.send(Input::WsOpen(generation));
        let (mut sink, mut stream) = ws.split();
        loop {
            tokio::select! {
                out = out_rx.recv() => match out {
                    Some(Some(b)) => if sink.send(Message::Binary(b.into())).await.is_err() { break },
                    _ => { let _ = sink.send(Message::Close(None)).await; break }
                },
                msg = stream.next() => match msg {
                    Some(Ok(Message::Binary(b))) => { let _ = tx.send(Input::WsFrame(generation, b.to_vec())); }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Frame(_))) => {}
                    _ => break,
                },
            }
        }
        let _ = tx.send(Input::WsClosed(generation));
    });
    out_tx
}

fn spawn_http(http: reqwest::Client, req: HttpRequest, tx: mpsc::UnboundedSender<Input>) {
    tokio::spawn(async move {
        let method =
            reqwest::Method::from_bytes(req.method.as_bytes()).unwrap_or(reqwest::Method::GET);
        let mut b = http.request(method, &req.url);
        if let Some(a) = &req.auth {
            b = b.header("authorization", a);
        }
        if let Some(body) = req.body {
            b = b.header("content-type", "application/json").body(body);
        }
        let resp = match b.send().await {
            Ok(r) => {
                let status = r.status().as_u16();
                let retry_after_ms = r
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|s| (s * 1000.0) as u64)
                    .unwrap_or(0);
                HttpResponse {
                    id: req.id,
                    status,
                    body: r.text().await.unwrap_or_default(),
                    retry_after_ms,
                }
            }
            Err(_) => HttpResponse {
                id: req.id,
                status: 0,
                body: String::new(),
                retry_after_ms: 0,
            },
        };
        let _ = tx.send(Input::Http(resp));
    });
}
