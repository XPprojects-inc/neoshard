//! NeoShard REST API. Spec: `spec/openapi.yaml`.
//! SPDX-License-Identifier: AGPL-3.0-or-later

pub mod authz;
pub mod error;
pub mod events;
pub mod extract;
pub mod mail;
pub mod ratelimit;
pub mod routes;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use neoshard_server_common::SharedCtx;
use tower_http::cors::{Any, CorsLayer};

/// State shared by API handlers.
#[derive(Clone)]
pub struct AppState {
    pub ctx: SharedCtx,
    pub limiter: Arc<ratelimit::RateLimiter>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(ctx: SharedCtx) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent(concat!(
                "NeoShardBot/",
                env!("CARGO_PKG_VERSION"),
                " (+link previews)"
            ))
            .build()
            .expect("reqwest client");
        let st = Self {
            ctx,
            limiter: Default::default(),
            http,
        };
        let limiter = st.limiter.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(60));
            loop {
                t.tick().await;
                limiter.sweep();
            }
        });
        st
    }
}

/// Router with every API route mounted under `/api/v1`, plus `/.well-known/neoshard`.
pub fn router(st: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers([axum::http::header::RETRY_AFTER]);
    Router::new()
        .nest("/api/v1", routes::v1())
        .merge(routes::wellknown::router())
        .merge(routes::files::serve_router())
        .merge(routes::federation::router())
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(cors)
        .with_state(st)
}
