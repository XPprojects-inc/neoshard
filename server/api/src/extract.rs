//! Request extractors: authentication, client IP, lenient ID deserialization.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use neoshard_server_common::auth::AuthUser;
use serde::{Deserialize, Deserializer};

use crate::AppState;
use crate::error::ApiError;
use crate::ratelimit::rates;

/// Authenticated caller. Accepts `Authorization: Bearer <token>` and `Bot <token>`.
#[derive(Debug, Clone, Copy)]
pub struct Auth(pub AuthUser);

impl Auth {
    pub fn id(&self) -> u64 {
        self.0.user_id
    }
}

pub fn bearer(parts: &Parts) -> Option<&str> {
    let h = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    h.strip_prefix("Bearer ")
        .or_else(|| h.strip_prefix("Bot "))
        .map(str::trim)
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let token = bearer(parts).ok_or(ApiError::Unauthorized)?;
        let user = st
            .ctx
            .sessions
            .authenticate(&st.ctx.db, token)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        st.limiter
            .check(&format!("g:{}", user.user_id), rates::GLOBAL_USER)?;
        Ok(Auth(user))
    }
}

/// Client IP; honours `X-Real-IP` / first `X-Forwarded-For` only with `trust_proxy`.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        if st.ctx.cfg.trust_proxy {
            let hdr = |n: &str| {
                parts
                    .headers
                    .get(n)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            };
            // The rightmost X-Forwarded-For entry is the one our own proxy appended;
            // everything to its left is client-controlled.
            let forwarded = hdr("x-real-ip").or_else(|| {
                hdr("x-forwarded-for")
                    .and_then(|v| v.rsplit(',').next().map(|s| s.trim().to_owned()))
            });
            if let Some(ip) = forwarded.and_then(|v| v.parse().ok()) {
                return Ok(ClientIp(ip));
            }
        }
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        Ok(ClientIp(ip))
    }
}

/// Deserialize a snowflake from a JSON string ("123") or number.
pub fn id<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        N(u64),
    }
    match Raw::deserialize(d)? {
        Raw::S(s) => s.parse().map_err(serde::de::Error::custom),
        Raw::N(n) => Ok(n),
    }
}

pub fn opt_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        N(u64),
        Null,
    }
    match Option::<Raw>::deserialize(d)? {
        None | Some(Raw::Null) => Ok(None),
        Some(Raw::S(s)) if s.is_empty() || s == "0" => Ok(None),
        Some(Raw::S(s)) => s.parse().map(Some).map_err(serde::de::Error::custom),
        Some(Raw::N(0)) => Ok(None),
        Some(Raw::N(n)) => Ok(Some(n)),
    }
}

pub fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    #[derive(Deserialize)]
    struct W(#[serde(deserialize_with = "id")] u64);
    Ok(Vec::<W>::deserialize(d)?.into_iter().map(|w| w.0).collect())
}

/// Permission bits as a decimal string or number.
pub fn bits<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    id(d)
}

pub fn opt_bits<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        N(u64),
    }
    match Option::<Raw>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::S(s)) => s.parse().map(Some).map_err(serde::de::Error::custom),
        Some(Raw::N(n)) => Ok(Some(n)),
    }
}
