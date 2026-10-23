//! Assembles the services of one NeoShard node.
//! SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::Arc;

use axum::Router;
use neoshard_server_common::SharedCtx;
use tower_http::trace::TraceLayer;

pub struct Node {
    pub router: Router,
    pub hub: Option<Arc<neoshard_gateway::Hub>>,
}

/// Build the HTTP router for the services enabled in `ctx.cfg.only`.
pub fn build(ctx: SharedCtx) -> Node {
    let mut router = Router::new();
    if ctx.cfg.runs("api") {
        router = router.merge(neoshard_api::router(neoshard_api::AppState::new(
            ctx.clone(),
        )));
    }
    let mut hub = None;
    if ctx.cfg.runs("gateway") {
        let presence = Arc::new(neoshard_presence::Registry::default());
        let h = neoshard_gateway::new_hub(ctx.clone(), presence);
        router = router.merge(neoshard_gateway::router(h.clone()));
        hub = Some(h);
    }
    Node {
        router: router.layer(TraceLayer::new_for_http()),
        hub,
    }
}
