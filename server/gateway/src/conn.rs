//! One WebSocket connection: handshake (Hello → Identify/Resume), heartbeats,
//! client requests, and a writer task. Session state lives in the actor.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use neoshard_proto::framing;
use neoshard_proto::v1::{self, GatewayErrorCode as Code, gateway_frame::Kind as F};
use neoshard_server_common::repo;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::hub::{Hub, SessionHandle};
use crate::session::{Ctl, Session, SessionInit, build_ready};

/// Maximum client frame size.
const MAX_CLIENT_FRAME: usize = 64 * 1024;
/// Client frames allowed per minute (heartbeats included).
const CLIENT_FRAMES_PER_MIN: u32 = 120;
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);

static GENERATION: AtomicU64 = AtomicU64::new(1);

pub struct ConnParams {
    pub compress: bool,
    pub version: u32,
    pub addr: SocketAddr,
}

fn frame(kind: F) -> v1::GatewayFrame {
    v1::GatewayFrame { kind: Some(kind) }
}

fn bin(f: &v1::GatewayFrame, compress: bool) -> Message {
    Message::Binary(framing::encode(f, compress).into())
}

async fn close_with(ws: &mut WebSocket, compress: bool, code: Code, msg: &str, reconnect: bool) {
    let err = frame(F::Error(v1::GatewayError {
        code: code as i32,
        message: msg.to_string(),
        reconnect,
    }));
    let _ = ws.send(bin(&err, compress)).await;
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code: 4000 + code as u16,
            reason: msg.to_string().into(),
        })))
        .await;
}

async fn invalid_session(ws: &mut WebSocket, compress: bool) {
    let _ = ws
        .send(bin(
            &frame(F::InvalidSession(v1::InvalidSession { resumable: false })),
            compress,
        ))
        .await;
}

fn session_id() -> String {
    let b: [u8; 16] = rand::random();
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, b)
}

/// Handshake: Hello → Identify/Resume → Ready/Resumed. Returns the socket, the
/// session's control channel and the connection plumbing, or None if it ended.
async fn handshake(
    mut ws: WebSocket,
    hub: &Arc<Hub>,
    p: &ConnParams,
) -> Option<(
    WebSocket,
    mpsc::Sender<Ctl>,
    u64,
    Arc<Notify>,
    mpsc::Sender<Bytes>,
    mpsc::Receiver<Bytes>,
)> {
    let compress = p.compress;
    if p.version != neoshard_proto::PROTOCOL_VERSION {
        close_with(
            &mut ws,
            compress,
            Code::InvalidVersion,
            "unsupported protocol version",
            false,
        )
        .await;
        return None;
    }
    let interval = hub.ctx.cfg.heartbeat_interval_ms;
    let hello = frame(F::Hello(v1::Hello {
        heartbeat_interval_ms: interval,
        node: hub.node.clone(),
    }));
    if ws.send(bin(&hello, compress)).await.is_err() {
        return None;
    }

    // ---- handshake
    let first = tokio::time::timeout(IDENTIFY_TIMEOUT, async {
        loop {
            match ws.recv().await {
                Some(Ok(Message::Binary(b))) => return framing::decode(&b).ok(),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return None,
            }
        }
    })
    .await;
    let Ok(Some(first)) = first else {
        close_with(
            &mut ws,
            compress,
            Code::NotAuthenticated,
            "identify timeout",
            true,
        )
        .await;
        return None;
    };

    let epoch = GENERATION.fetch_add(1, Ordering::Relaxed);
    let kill = Arc::new(Notify::new());
    let (out_tx, out_rx) = mpsc::channel::<Bytes>(512);
    let ctl = match first.kind {
        Some(F::Identify(id)) => {
            let auth = match hub.ctx.sessions.authenticate(&hub.ctx.db, &id.token).await {
                Ok(Some(a)) => a,
                _ => {
                    close_with(
                        &mut ws,
                        compress,
                        Code::AuthFailed,
                        "authentication failed",
                        false,
                    )
                    .await;
                    return None;
                }
            };
            // A fresh Identify with the same login means the client lost its session
            // (page reload, app restart): its old sessions waiting for Resume are
            // superseded. If the per-user cap is still hit, the oldest waiting
            // sessions of any login go first; live connections are never evicted.
            hub.evict_detached(auth.user_id, Some(auth.session_id));
            let mut reserved = hub.reserve_session(auth.user_id);
            if !reserved && hub.evict_detached(auth.user_id, None) > 0 {
                for _ in 0..50 {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    if hub.reserve_session(auth.user_id) {
                        reserved = true;
                        break;
                    }
                }
            }
            if !reserved {
                close_with(
                    &mut ws,
                    compress,
                    Code::RateLimited,
                    "too many sessions",
                    false,
                )
                .await;
                return None;
            }
            let sid = session_id();
            let status = v1::Status::try_from(id.initial_status).unwrap_or(v1::Status::Online);
            let custom = sqlx::query!(
                "SELECT custom_status, custom_status_emoji, custom_status_expires_at FROM users WHERE id = $1",
                auth.user_id as i64
            )
            .fetch_optional(&hub.ctx.db)
            .await
            .ok()
            .flatten()
            .map(|r| {
                neoshard_presence::Custom::new(
                    &r.custom_status,
                    &r.custom_status_emoji,
                    r.custom_status_expires_at.map(|t| t.timestamp_millis()).unwrap_or(0),
                )
            })
            .unwrap_or_default();
            let custom_for_ready = custom.clone();
            let attached = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let init = SessionInit {
                id: sid.clone(),
                user_id: auth.user_id,
                auth_session_id: auth.session_id,
                bot: auth.bot,
                intents: id.intents,
                status,
                custom,
                attached: attached.clone(),
            };
            // Subscribe first, then build Ready: events in between queue in the inbox.
            let guild_ids = repo::guild_ids_for_user(&hub.ctx.db, auth.user_id)
                .await
                .unwrap_or_default();
            let prepared = Session::prepare(hub.clone(), init, &guild_ids);
            let ready = match build_ready(hub, auth.user_id, &sid, &guild_ids).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "ready failed");
                    prepared.abort(&guild_ids);
                    close_with(&mut ws, compress, Code::Unknown, "internal error", true).await;
                    return None;
                }
            };
            // The user's own presence (incl. Invisible and the stored custom status),
            // so a new device shows and keeps what was set elsewhere.
            let mut ready = ready;
            let (own_status, own_custom) = hub
                .presence
                .get_self(auth.user_id)
                .unwrap_or((status, custom_for_ready.clone()));
            ready.presences.push(v1::Presence {
                user_id: auth.user_id,
                status: own_status as i32,
                custom_status: own_custom.text,
                custom_emoji: own_custom.emoji,
                custom_expires_at: own_custom.expires_at,
            });
            let ctl = prepared.start(&ready.guilds);
            hub.sessions.insert(
                sid.clone(),
                SessionHandle {
                    user_id: auth.user_id,
                    auth_session_id: auth.session_id,
                    ctl: ctl.clone(),
                    attached: attached.clone(),
                },
            );
            let (tx, rx) = oneshot::channel();
            let _ = ctl
                .send(Ctl::Attach {
                    epoch,
                    out: out_tx.clone(),
                    compress,
                    last_seq: None,
                    kill: kill.clone(),
                    reply: tx,
                })
                .await;
            let _ = rx.await;
            // Ready goes out before anything the actor queued (the writer starts after this).
            if ws
                .send(bin(&frame(F::Ready(ready)), compress))
                .await
                .is_err()
            {
                let _ = ctl.send(Ctl::Detach { epoch }).await;
                return None;
            }
            ctl
        }
        Some(F::Resume(r)) => {
            let auth = match hub.ctx.sessions.authenticate(&hub.ctx.db, &r.token).await {
                Ok(Some(a)) => a,
                _ => {
                    close_with(
                        &mut ws,
                        compress,
                        Code::AuthFailed,
                        "authentication failed",
                        false,
                    )
                    .await;
                    return None;
                }
            };
            let Some(h) = hub
                .sessions
                .get(&r.session_id)
                .map(|h| h.clone())
                .filter(|h| h.user_id == auth.user_id)
            else {
                invalid_session(&mut ws, compress).await;
                return None;
            };
            let (tx, rx) = oneshot::channel();
            if h.ctl
                .send(Ctl::Attach {
                    epoch,
                    out: out_tx.clone(),
                    compress,
                    last_seq: Some(r.last_seq),
                    kill: kill.clone(),
                    reply: tx,
                })
                .await
                .is_err()
            {
                invalid_session(&mut ws, compress).await;
                return None;
            }
            match rx.await {
                Ok(Ok(replayed)) => {
                    // Replayed dispatches are already queued in out_tx; Resumed follows them.
                    let _ = out_tx
                        .send(Bytes::from(framing::encode(
                            &frame(F::Resumed(v1::Resumed { replayed })),
                            compress,
                        )))
                        .await;
                    h.ctl.clone()
                }
                _ => {
                    let _ = h.ctl.send(Ctl::Close).await;
                    invalid_session(&mut ws, compress).await;
                    return None;
                }
            }
        }
        _ => {
            close_with(
                &mut ws,
                compress,
                Code::NotAuthenticated,
                "identify or resume first",
                true,
            )
            .await;
            return None;
        }
    };
    Some((ws, ctl, epoch, kill, out_tx, out_rx))
}

pub async fn handle(ws: WebSocket, hub: Arc<Hub>, p: ConnParams) {
    let compress = p.compress;
    let interval = hub.ctx.cfg.heartbeat_interval_ms;
    // Boxed so the handshake's large state (DB queries, Ready) is freed afterwards.
    let Some((ws, ctl, epoch, kill, out_tx, mut out_rx)) = Box::pin(handshake(ws, &hub, &p)).await
    else {
        return;
    };
    hub.metrics.connections.fetch_add(1, Ordering::Relaxed);

    // ---- steady state
    let (mut sink, mut stream) = ws.split();
    let writer = tokio::spawn(async move {
        while let Some(bytes) = out_rx.recv().await {
            if sink.send(Message::Binary(bytes)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let deadline = Duration::from_millis(u64::from(interval) * 3 / 2 + 5_000);
    let mut window_start = tokio::time::Instant::now();
    let mut frames_in_window = 0u32;
    let send_err = |code: Code, msg: &str, reconnect: bool| {
        Bytes::from(framing::encode(
            &frame(F::Error(v1::GatewayError {
                code: code as i32,
                message: msg.into(),
                reconnect,
            })),
            compress,
        ))
    };
    loop {
        let msg = tokio::select! {
            m = tokio::time::timeout(deadline, stream.next()) => match m {
                Ok(Some(Ok(m))) => m,
                _ => break, // closed, error, or no heartbeat (zombie client)
            },
            // The session dropped this connection (revoked, overflow, replaced by a resume).
            _ = kill.notified() => break,
        };
        let bytes = match msg {
            Message::Binary(b) => b,
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Text(_) => {
                let _ = out_tx
                    .send(send_err(Code::Decode, "binary frames only", true))
                    .await;
                break;
            }
        };
        if window_start.elapsed() > Duration::from_secs(60) {
            window_start = tokio::time::Instant::now();
            frames_in_window = 0;
        }
        frames_in_window += 1;
        if frames_in_window > CLIENT_FRAMES_PER_MIN || bytes.len() > MAX_CLIENT_FRAME {
            let _ = out_tx
                .send(send_err(Code::RateLimited, "too many frames", true))
                .await;
            break;
        }
        let Ok(f) = framing::decode(&bytes) else {
            let _ = out_tx
                .send(send_err(Code::Decode, "decode error", true))
                .await;
            break;
        };
        match f.kind {
            Some(F::Heartbeat(_)) => {
                let ack = Bytes::from(framing::encode(
                    &frame(F::HeartbeatAck(v1::HeartbeatAck {})),
                    compress,
                ));
                if out_tx.send(ack).await.is_err() {
                    break;
                }
            }
            Some(F::Request(r)) => {
                if ctl.send(Ctl::Request(r)).await.is_err() {
                    break;
                }
            }
            Some(F::Identify(_) | F::Resume(_)) => {
                let _ = out_tx
                    .send(send_err(
                        Code::AlreadyAuthenticated,
                        "already authenticated",
                        false,
                    ))
                    .await;
                break;
            }
            _ => {}
        }
    }
    let _ = ctl.send(Ctl::Detach { epoch }).await;
    drop(out_tx);
    // Give the writer a moment to flush a final error frame.
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
    hub.metrics.connections.fetch_sub(1, Ordering::Relaxed);
    let _ = p.addr;
}
