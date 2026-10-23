use axum::Router;

use crate::AppState;

pub mod apps;
pub mod auth;
pub mod automod;
pub mod channels;
pub mod discovery;
pub mod federation;
pub mod files;
pub mod guilds;
pub mod media;
pub mod messages;
pub mod previews;
pub mod profiles;
pub mod relationships;
pub mod uploads;
pub mod users;
pub mod webhooks;
pub mod wellknown;

pub fn v1() -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(files::upload_router())
        .merge(guilds::router())
        .merge(channels::router())
        .merge(discovery::router())
        .merge(messages::router())
        .merge(automod::router())
        .merge(uploads::router())
        .merge(webhooks::router())
        .merge(users::router())
        .merge(profiles::router())
        .merge(relationships::router())
        .merge(apps::router())
        .merge(media::router())
        .merge(federation::api_router())
}

/// Base64 Ed25519 public key of this instance, when federation is configured.
pub fn federation_public_key(st: &AppState) -> Option<String> {
    let seed = st.ctx.cfg.federation_key.as_deref()?;
    neoshard_federation::InstanceKey::from_seed_b64(seed)
        .ok()
        .map(|k| k.public_b64())
}
