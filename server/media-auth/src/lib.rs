//! Media authorization (ADR-0012).
//!
//! NeoShard never relays media. It decides *who may do what* in a voice
//! channel and encodes that into a LiveKit access token (JWT, HS256), plus
//! short-lived TURN credentials in the coturn REST API format.
//! SPDX-License-Identifier: AGPL-3.0-or-later

use base64::Engine;
use hmac::{Hmac, Mac};
use jsonwebtoken::{EncodingKey, Header, encode};
use neoshard_proto::permissions::Permissions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoGrant {
    pub room: String,
    pub room_join: bool,
    pub can_publish: bool,
    pub can_subscribe: bool,
    pub can_publish_data: bool,
    pub can_publish_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    pub name: String,
    pub nbf: u64,
    pub exp: u64,
    pub video: VideoGrant,
    /// JSON metadata visible to other participants.
    pub metadata: String,
}

pub fn room_name(channel_id: u64) -> String {
    format!("voice:{channel_id}")
}

/// Compute grants from channel permissions. `None` if the user may not connect.
pub fn grants(channel_id: u64, perms: Permissions) -> Option<VideoGrant> {
    if !perms.contains(Permissions::CONNECT) {
        return None;
    }
    let mut sources = Vec::new();
    if perms.contains(Permissions::SPEAK) {
        sources.push("microphone".to_string());
    }
    if perms.contains(Permissions::VIDEO) {
        sources.push("camera".to_string());
    }
    if perms.contains(Permissions::STREAM) {
        sources.push("screen_share".to_string());
        sources.push("screen_share_audio".to_string());
    }
    Some(VideoGrant {
        room: room_name(channel_id),
        room_join: true,
        can_publish: !sources.is_empty(),
        can_subscribe: true,
        can_publish_data: true,
        can_publish_sources: sources,
    })
}

pub struct LiveKitKeys<'a> {
    pub api_key: &'a str,
    pub api_secret: &'a str,
}

pub fn livekit_token(
    keys: &LiveKitKeys<'_>,
    user_id: u64,
    display: &str,
    grant: VideoGrant,
    now_secs: u64,
    ttl_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = Claims {
        iss: keys.api_key.to_string(),
        sub: user_id.to_string(),
        name: display.to_string(),
        nbf: now_secs.saturating_sub(10),
        exp: now_secs + ttl_secs,
        video: grant,
        metadata: format!("{{\"user_id\":\"{user_id}\"}}"),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(keys.api_secret.as_bytes()),
    )
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TurnCredentials {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
    pub ttl: u64,
}

/// coturn `use-auth-secret` credentials: username = "{expiry}:{user}",
/// credential = base64(HMAC-SHA1(secret, username)).
pub fn turn_credentials(
    secret: &str,
    urls: &[String],
    user_id: u64,
    now_secs: u64,
    ttl: u64,
) -> TurnCredentials {
    let username = format!("{}:{}", now_secs + ttl, user_id);
    let mut mac =
        Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(username.as_bytes());
    let credential = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    TurnCredentials {
        urls: urls.to_vec(),
        username,
        credential,
        ttl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Permissions as P;
    use jsonwebtoken::{DecodingKey, Validation, decode};

    #[test]
    fn no_connect_no_grant() {
        assert!(grants(1, P::VIEW_CHANNEL | P::SPEAK).is_none());
    }

    #[test]
    fn listen_only() {
        let g = grants(1, P::CONNECT).unwrap();
        assert!(!g.can_publish);
        assert!(g.can_subscribe);
        assert!(g.can_publish_sources.is_empty());
    }

    #[test]
    fn stream_needs_stream_bit() {
        let g = grants(9, P::CONNECT | P::SPEAK | P::VIDEO).unwrap();
        assert_eq!(g.room, "voice:9");
        assert_eq!(g.can_publish_sources, vec!["microphone", "camera"]);
        let g = grants(9, P::CONNECT | P::STREAM).unwrap();
        assert!(g.can_publish_sources.contains(&"screen_share".to_string()));
    }

    #[test]
    fn token_is_valid_jwt() {
        let keys = LiveKitKeys {
            api_key: "key",
            api_secret: "secret-secret-secret-secret-secret",
        };
        let now = 2_000_000_000;
        let tok = livekit_token(
            &keys,
            42,
            "alice",
            grants(1, P::CONNECT | P::SPEAK).unwrap(),
            now,
            3600,
        )
        .unwrap();
        let mut v = Validation::default();
        v.validate_exp = false;
        v.validate_nbf = false;
        let data = decode::<Claims>(
            &tok,
            &DecodingKey::from_secret(keys.api_secret.as_bytes()),
            &v,
        )
        .unwrap();
        assert_eq!(data.claims.sub, "42");
        assert_eq!(data.claims.iss, "key");
        assert_eq!(data.claims.exp, now + 3600);
        let json = serde_json::to_value(&data.claims).unwrap();
        assert_eq!(json["video"]["roomJoin"], true);
    }

    #[test]
    fn turn_credentials_format() {
        let c = turn_credentials(
            "s3cr3t",
            &["turns:turn.example:443?transport=tcp".into()],
            7,
            1000,
            86400,
        );
        assert_eq!(c.username, "87400:7");
        // Known-answer: HMAC-SHA1("s3cr3t", "87400:7"), base64.
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"s3cr3t").unwrap();
        mac.update(b"87400:7");
        assert_eq!(
            c.credential,
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        );
    }
}
