//! Authentication primitives (ADR-0017): argon2id passwords, opaque bearer
//! tokens stored as SHA-256, TOTP, secret encryption at rest.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dashmap::DashMap;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

fn argon() -> Argon2<'static> {
    // OWASP 2024 recommendation for argon2id: m=19 MiB, t=2, p=1.
    let params = Params::new(19 * 1024, 2, 1, None).expect("valid argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(argon()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_string())
}

/// Verify a password against a PHC hash. Unknown accounts should still call
/// this with [`dummy_hash`] so that response time does not reveal existence.
pub fn verify_password(hash: &str, password: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => argon()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

static DUMMY: LazyLock<String> =
    LazyLock::new(|| hash_password("neoshard-dummy-password").expect("hash"));

pub fn dummy_hash() -> &'static str {
    &DUMMY
}

/// Password policy: length only (NIST 800-63B), 8..=128 chars.
pub fn password_ok(password: &str) -> bool {
    (8..=128).contains(&password.chars().count())
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    OsRng.fill_bytes(&mut b);
    b
}

/// New bearer token with a prefix (`ns_` user, `nsb_` bot, `nsw_` webhook, …)
/// and its storage hash.
pub fn new_token(prefix: &str) -> (String, Vec<u8>) {
    let token = format!("{prefix}{}", URL_SAFE_NO_PAD.encode(random_bytes::<32>()));
    let hash = hash_token(&token);
    (token, hash)
}

pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Constant-time byte comparison.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ------------------------------------------------------------------ secrets at rest

pub fn encrypt_secret(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce_bytes = random_bytes::<12>();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .expect("aes-gcm encrypt");
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ct);
    out
}

pub fn decrypt_secret(key: &[u8; 32], blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < 12 {
        return None;
    }
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(&blob[..12]), &blob[12..])
        .ok()
}

// ------------------------------------------------------------------ TOTP (RFC 6238)

pub mod totp {
    use totp_rs::{Algorithm, Secret, TOTP};

    pub fn generate_secret() -> Vec<u8> {
        super::random_bytes::<20>().to_vec()
    }

    fn build(secret: &[u8], account: &str, issuer: &str) -> Option<TOTP> {
        TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret.to_vec(),
            Some(issuer.to_string()),
            account.to_string(),
        )
        .ok()
    }

    /// Verify a 6-digit code with a ±1 step window.
    pub fn verify(secret: &[u8], code: &str) -> bool {
        let code = code.trim();
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        build(secret, "x", "NeoShard")
            .map(|t| t.check_current(code).unwrap_or(false))
            .unwrap_or(false)
    }

    pub fn current_code(secret: &[u8]) -> String {
        build(secret, "x", "NeoShard")
            .and_then(|t| t.generate_current().ok())
            .unwrap_or_default()
    }

    pub fn code_at(secret: &[u8], unix_secs: u64) -> String {
        build(secret, "x", "NeoShard")
            .map(|t| t.generate(unix_secs))
            .unwrap_or_default()
    }

    /// The 30-second step (−1, 0, +1 around `now`) whose code matches, if any.
    /// Callers store the step to reject replays of the same code.
    pub fn matching_step(secret: &[u8], code: &str, now_secs: u64) -> Option<u64> {
        let code = code.trim();
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let t = build(secret, "x", "NeoShard")?;
        [now_secs.saturating_sub(30), now_secs, now_secs + 30]
            .into_iter()
            .find(|&ts| super::ct_eq(t.generate(ts).as_bytes(), code.as_bytes()))
            .map(|ts| ts / 30)
    }

    pub fn otpauth_url(secret: &[u8], account: &str, issuer: &str) -> String {
        build(secret, account, issuer)
            .map(|t| t.get_url())
            .unwrap_or_default()
    }

    pub fn base32(secret: &[u8]) -> String {
        Secret::Raw(secret.to_vec()).to_encoded().to_string()
    }
}

// ------------------------------------------------------------------ session lookup

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthUser {
    pub user_id: u64,
    pub session_id: u64,
    pub bot: bool,
}

/// Short-TTL cache of token → session. Revocation on this node is immediate
/// ([`SessionCache::evict_user`]); other nodes converge within [`CACHE_TTL`].
pub const CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct SessionCache {
    map: DashMap<Vec<u8>, (AuthUser, Instant)>,
}

impl SessionCache {
    pub async fn authenticate(
        &self,
        db: &PgPool,
        token: &str,
    ) -> Result<Option<AuthUser>, sqlx::Error> {
        if token.is_empty() || token.len() > 128 {
            return Ok(None);
        }
        let hash = hash_token(token);
        if let Some(e) = self.map.get(&hash)
            && e.1.elapsed() < CACHE_TTL
        {
            return Ok(Some(e.0));
        }
        let row = sqlx::query!(
            r#"UPDATE sessions s SET last_seen_at = now()
               FROM users u
               WHERE s.token_hash = $1 AND u.id = s.user_id AND u.deleted_at IS NULL
                 AND (s.expires_at IS NULL OR s.expires_at > now())
               RETURNING s.id, s.user_id, u.bot"#,
            hash
        )
        .fetch_optional(db)
        .await?;
        let Some(r) = row else {
            self.map.remove(&hash);
            return Ok(None);
        };
        let au = AuthUser {
            user_id: r.user_id as u64,
            session_id: r.id as u64,
            bot: r.bot,
        };
        if self.map.len() > 200_000 {
            self.map.retain(|_, v| v.1.elapsed() < CACHE_TTL);
        }
        self.map.insert(hash, (au, Instant::now()));
        Ok(Some(au))
    }

    pub fn evict_session(&self, session_id: u64) {
        self.map.retain(|_, v| v.0.session_id != session_id);
    }

    pub fn evict_user(&self, user_id: u64) {
        self.map.retain(|_, v| v.0.user_id != user_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password(&h, "correct horse battery"));
        assert!(!verify_password(&h, "wrong"));
        assert!(!verify_password("not-a-hash", "x"));
        assert!(!verify_password(dummy_hash(), "anything"));
    }

    #[test]
    fn policy() {
        assert!(!password_ok("short"));
        assert!(password_ok("eight888"));
        assert!(!password_ok(&"x".repeat(129)));
    }

    #[test]
    fn tokens_unique_and_hashed() {
        let (a, ha) = new_token("ns_");
        let (b, _) = new_token("ns_");
        assert_ne!(a, b);
        assert!(a.starts_with("ns_"));
        assert_eq!(hash_token(&a), ha);
        assert!(ct_eq(&ha, &hash_token(&a)));
        assert!(!ct_eq(&ha, &hash_token(&b)));
    }

    #[test]
    fn secret_encryption() {
        let key = [7u8; 32];
        let blob = encrypt_secret(&key, b"totp-secret");
        assert_eq!(decrypt_secret(&key, &blob).unwrap(), b"totp-secret");
        assert!(decrypt_secret(&[8u8; 32], &blob).is_none());
        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decrypt_secret(&key, &tampered).is_none());
    }

    #[test]
    fn totp_verify() {
        let s = totp::generate_secret();
        let code = totp::current_code(&s);
        assert!(totp::verify(&s, &code));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(
            totp::matching_step(&s, &totp::code_at(&s, now + 30), now),
            Some((now + 30) / 30)
        );
        assert_eq!(
            totp::matching_step(&s, &totp::code_at(&s, now + 300), now),
            None
        );
        assert!(!totp::verify(&s, "12345"));
        assert!(!totp::verify(&s, "abcdef"));
        assert!(totp::otpauth_url(&s, "alice", "NeoShard").starts_with("otpauth://totp/"));
    }
}
