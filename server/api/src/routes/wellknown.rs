//! Instance discovery: `GET /.well-known/neoshard`.
//!
//! A client only needs the instance's base URL; everything else (gateway,
//! media, federation key, limits) comes from here. This is what makes
//! "connect to any server" and network-restriction workarounds (custom
//! endpoints on 443) possible without rebuilding the client.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/.well-known/neoshard", get(discovery))
        .route("/healthz", get(|| async { "ok" }))
}

async fn discovery(State(st): State<AppState>) -> Json<serde_json::Value> {
    let c = &st.ctx.cfg;
    Json(json!({
        "name": "NeoShard",
        "domain": c.domain,
        "protocol_version": neoshard_proto::PROTOCOL_VERSION,
        "server_version": env!("CARGO_PKG_VERSION"),
        "api": format!("{}/api/v1", c.public_url.trim_end_matches('/')),
        "gateway": c.gateway_public_url(),
        "media": { "livekit": c.livekit_url, "turn": c.turn_urls },
        "registration_open": c.registration_open,
        "max_upload_bytes": c.max_upload_bytes,
        "files": c.files_base(),
        "federation": {
            "enabled": c.federation_enabled,
            "public_key": neoshard_federation_key(&st),
        },
    }))
}

fn neoshard_federation_key(st: &AppState) -> Option<String> {
    crate::routes::federation_public_key(st)
}
