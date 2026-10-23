//! NeoShard realtime Gateway. Behaviour spec: `spec/gateway.md`.
//! SPDX-License-Identifier: AGPL-3.0-or-later

pub mod conn;
pub mod hub;
pub mod session;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::extract::{ConnectInfo, Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use neoshard_server_common::SharedCtx;
use serde::Deserialize;

pub use hub::Hub;

#[derive(Deserialize)]
struct GatewayQuery {
    #[serde(default = "one")]
    v: u32,
    #[serde(default)]
    compress: String,
}

fn one() -> u32 {
    1
}

pub fn new_hub(ctx: SharedCtx, presence: Arc<neoshard_presence::Registry>) -> Arc<Hub> {
    Hub::new(ctx, presence)
}

/// `GET /gateway` (WebSocket upgrade) and `GET /gateway/metrics` (Prometheus text).
pub fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/gateway", get(upgrade))
        .route("/gateway/metrics", get(metrics))
        .with_state(hub)
}

async fn upgrade(
    State(hub): State<Arc<Hub>>,
    Query(q): Query<GatewayQuery>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let params = conn::ConnParams {
        compress: q.compress == "zstd",
        version: q.v,
        addr,
    };
    // Small per-socket buffers: 100k connections × the 128 KiB defaults would be ~25 GiB.
    ws.max_message_size(64 * 1024)
        .read_buffer_size(4 * 1024)
        .write_buffer_size(4 * 1024)
        .on_upgrade(move |socket| conn::handle(socket, hub, params))
}

async fn metrics(State(hub): State<Arc<Hub>>) -> String {
    let m = &hub.metrics;
    format!(
        "# TYPE neoshard_gateway_connections gauge\nneoshard_gateway_connections {}\n\
         # TYPE neoshard_gateway_sessions gauge\nneoshard_gateway_sessions {}\n\
         # TYPE neoshard_gateway_events_in_total counter\nneoshard_gateway_events_in_total {}\n\
         # TYPE neoshard_gateway_events_out_total counter\nneoshard_gateway_events_out_total {}\n\
         # TYPE neoshard_gateway_dropped_slow_total counter\nneoshard_gateway_dropped_slow_total {}\n\
         # TYPE neoshard_presence_online gauge\nneoshard_presence_online {}\n",
        m.connections.load(Ordering::Relaxed),
        hub.sessions.len(),
        m.events_in.load(Ordering::Relaxed),
        m.events_out.load(Ordering::Relaxed),
        m.dropped_slow.load(Ordering::Relaxed),
        hub.presence.online_count(),
    )
}
