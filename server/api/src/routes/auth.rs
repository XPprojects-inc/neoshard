//! Accounts: registration, login, 2FA, sessions, recovery, deletion (ADR-0017).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use neoshard_proto::v1;
use neoshard_server_common::auth::{self, totp};
use neoshard_server_common::repo;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::{Auth, ClientIp};
use crate::ratelimit::rates;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/mfa/totp", post(mfa_totp))
        .route("/auth/mfa/totp/setup", post(totp_setup))
        .route("/auth/mfa/totp/enable", post(totp_enable))
        .route("/auth/mfa/totp/disable", post(totp_disable))
        .route("/auth/logout", post(logout))
        .route("/auth/password", post(change_password))
        .route("/auth/recover", post(recover))
        .route("/auth/recover/confirm", post(recover_confirm))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{id}", delete(revoke_session))
        .route("/users/@me/delete", post(delete_account))
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: v1::User,
}

fn device(headers: &HeaderMap, explicit: Option<String>) -> String {
    explicit
        .or_else(|| {
            headers
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .unwrap_or_default()
        .chars()
        .take(128)
        .collect()
}

async fn create_session(st: &AppState, user_id: u64, device: &str, ip: &str) -> ApiResult<String> {
    let (token, hash) = auth::new_token("ns_");
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, token_hash, device, ip, expires_at) VALUES ($1, $2, $3, $4, $5, now() + interval '180 days')",
        st.ctx.id() as i64,
        user_id as i64,
        hash,
        device,
        ip
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(token)
}

async fn me(st: &AppState, user_id: u64) -> ApiResult<v1::User> {
    Ok(repo::user(&st.ctx.db, user_id)
        .await?
        .ok_or(ApiError::NotFound)?
        .private())
}

pub fn valid_username(u: &str) -> bool {
    // "deleted_*" is reserved for anonymized accounts.
    !u.starts_with("deleted_")
        && (2..=32).contains(&u.len())
        && u.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

fn valid_email(e: &str) -> bool {
    e.len() <= 254
        && e.split_once('@').is_some_and(|(l, d)| {
            !l.is_empty()
                && d.contains('.')
                && !d.starts_with('.')
                && !e.contains(char::is_whitespace)
        })
}

// ------------------------------------------------------------------ register / login

#[derive(Deserialize)]
struct RegisterReq {
    username: String,
    password: String,
    email: Option<String>,
    display_name: Option<String>,
    device: Option<String>,
}

async fn register(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(r): Json<RegisterReq>,
) -> ApiResult<(StatusCode, Json<AuthResponse>)> {
    if !st.ctx.cfg.registration_open {
        return Err(ApiError::Forbidden(
            "registration is closed on this instance",
        ));
    }
    st.limiter.check(&format!("reg:{ip}"), rates::REGISTER_IP)?;
    let username = r.username.trim().to_lowercase();
    if !valid_username(&username) {
        return Err(bad("username must be 2-32 chars of a-z, 0-9, _ and ."));
    }
    if !auth::password_ok(&r.password) {
        return Err(bad("password must be 8-128 characters"));
    }
    let email = r
        .email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    if let Some(e) = &email
        && !valid_email(e)
    {
        return Err(bad("invalid email"));
    }
    let display = r
        .display_name
        .unwrap_or_default()
        .trim()
        .chars()
        .take(64)
        .collect::<String>();
    let pw = r.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&pw))
        .await
        .map_err(anyhow::Error::from)??;
    let id = st.ctx.id();
    let res = sqlx::query!(
        "INSERT INTO users (id, username, display_name, email, password_hash) VALUES ($1, $2, $3, $4, $5)",
        id as i64,
        username,
        display,
        email,
        hash
    )
    .execute(&st.ctx.db)
    .await;
    match res {
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {
            return Err(ApiError::Conflict("username or email already taken"));
        }
        other => {
            other?;
        }
    }
    let token = create_session(&st, id, &device(&headers, r.device), &ip.to_string()).await?;
    Ok((
        StatusCode::CREATED,
        Json(AuthResponse {
            token,
            user: me(&st, id).await?,
        }),
    ))
}

#[derive(Deserialize)]
struct LoginReq {
    /// Username or email.
    login: String,
    password: String,
    device: Option<String>,
}

async fn login(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(r): Json<LoginReq>,
) -> ApiResult<Json<serde_json::Value>> {
    st.limiter.check(&format!("auth:{ip}"), rates::AUTH_IP)?;
    let login: String = r.login.trim().to_lowercase().chars().take(254).collect();
    // Per-account limit counts failures only, so nobody can lock a user out
    // with requests that never reach the password check.
    let acct_key = format!("auth-acct:{}", login.chars().take(64).collect::<String>());
    if st.limiter.is_limited(&acct_key, rates::LOGIN_FAILURES) {
        return Err(ApiError::RateLimited(std::time::Duration::from_secs(60)));
    }
    let row = sqlx::query!(
        "SELECT id, password_hash, mfa_enabled FROM users
         WHERE deleted_at IS NULL AND NOT bot AND instance = '' AND (username = $1 OR lower(email) = $1)",
        login
    )
    .fetch_optional(&st.ctx.db)
    .await?;
    let (hash, found) = match &row {
        Some(u) => (u.password_hash.clone().unwrap_or_default(), true),
        None => (auth::dummy_hash().to_string(), false),
    };
    let pw = r.password;
    let ok = tokio::task::spawn_blocking(move || auth::verify_password(&hash, &pw))
        .await
        .map_err(anyhow::Error::from)?;
    let Some(u) = row.filter(|_| ok && found) else {
        let _ = st.limiter.check(&acct_key, rates::LOGIN_FAILURES);
        return Err(ApiError::InvalidCredentials);
    };
    let dev = device(&headers, r.device);
    if u.mfa_enabled {
        let (ticket, th) = auth::new_token("nst_");
        sqlx::query!(
            "INSERT INTO mfa_tickets (token_hash, user_id, device, expires_at) VALUES ($1, $2, $3, now() + interval '5 minutes')",
            th,
            u.id,
            dev
        )
        .execute(&st.ctx.db)
        .await?;
        return Ok(Json(json!({ "mfa": true, "ticket": ticket })));
    }
    let token = create_session(&st, u.id as u64, &dev, &ip.to_string()).await?;
    Ok(Json(
        json!({ "mfa": false, "token": token, "user": me(&st, u.id as u64).await? }),
    ))
}

#[derive(Deserialize)]
struct MfaReq {
    ticket: String,
    code: String,
}

async fn mfa_totp(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(r): Json<MfaReq>,
) -> ApiResult<Json<AuthResponse>> {
    st.limiter.check(&format!("mfa:{ip}"), rates::AUTH_IP)?;
    let th = auth::hash_token(&r.ticket);
    // Count the attempt atomically; tickets die after 5 attempts.
    let t = sqlx::query!(
        "UPDATE mfa_tickets SET attempts = attempts + 1
         WHERE token_hash = $1 AND expires_at > now() AND attempts < 5
         RETURNING user_id, device",
        th
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::InvalidCredentials)?;
    let ok = verify_second_factor(&st, t.user_id as u64, &r.code).await?;
    if !ok {
        return Err(ApiError::InvalidCredentials);
    }
    sqlx::query!("DELETE FROM mfa_tickets WHERE token_hash = $1", th)
        .execute(&st.ctx.db)
        .await?;
    let token = create_session(&st, t.user_id as u64, &t.device, &ip.to_string()).await?;
    Ok(Json(AuthResponse {
        token,
        user: me(&st, t.user_id as u64).await?,
    }))
}

/// TOTP code or one-time backup code.
async fn verify_second_factor(st: &AppState, user_id: u64, code: &str) -> ApiResult<bool> {
    let enc = sqlx::query_scalar!(
        "SELECT mfa_secret_enc FROM users WHERE id = $1 AND mfa_enabled",
        user_id as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .flatten();
    let Some(enc) = enc else { return Ok(false) };
    let code = code.trim();
    if code.len() == 6 {
        let secret = auth::decrypt_secret(&st.ctx.secret_key, &enc)
            .ok_or_else(|| anyhow::anyhow!("cannot decrypt TOTP secret"))?;
        let now = (neoshard_server_common::repo::now_ms() / 1000) as u64;
        let Some(step) = totp::matching_step(&secret, code, now) else {
            return Ok(false);
        };
        // Each time step is accepted once: a sniffed code cannot be replayed.
        let fresh = sqlx::query!(
            "UPDATE users SET mfa_last_step = $2 WHERE id = $1 AND (mfa_last_step IS NULL OR mfa_last_step < $2) RETURNING id",
            user_id as i64,
            step as i64
        )
        .fetch_optional(&st.ctx.db)
        .await?;
        return Ok(fresh.is_some());
    }
    let normalized = code.replace('-', "").to_lowercase();
    let used = sqlx::query!(
        "UPDATE mfa_backup_codes SET used_at = now() WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL RETURNING user_id",
        user_id as i64,
        auth::hash_token(&normalized)
    )
    .fetch_optional(&st.ctx.db)
    .await?;
    Ok(used.is_some())
}

/// Re-authentication for sensitive actions. Failures are limited per account, so a
/// stolen session token cannot be used to guess the password.
async fn check_password(st: &AppState, user_id: u64, password: &str) -> ApiResult<()> {
    let key = format!("reauth:{user_id}");
    if st.limiter.is_limited(&key, rates::REAUTH_FAILURES) {
        return Err(ApiError::RateLimited(std::time::Duration::from_secs(300)));
    }
    let stored = sqlx::query_scalar!(
        "SELECT password_hash FROM users WHERE id = $1",
        user_id as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    let has_password = stored.is_some();
    let hash = stored.unwrap_or_else(|| auth::dummy_hash().to_string());
    let pw = password.to_string();
    let ok = tokio::task::spawn_blocking(move || auth::verify_password(&hash, &pw))
        .await
        .map_err(anyhow::Error::from)?;
    // Accounts without a password (bots, federated shadows) never pass, whatever is sent.
    if ok && has_password {
        Ok(())
    } else {
        let _ = st.limiter.check(&key, rates::REAUTH_FAILURES);
        Err(ApiError::InvalidCredentials)
    }
}

/// When 2FA is on, sensitive actions also need a current code.
async fn require_second_factor_if_enabled(
    st: &AppState,
    user_id: u64,
    code: Option<&str>,
) -> ApiResult<()> {
    let mfa = sqlx::query_scalar!(
        "SELECT mfa_enabled FROM users WHERE id = $1",
        user_id as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if !mfa {
        return Ok(());
    }
    let key = format!("reauth:{user_id}");
    if st.limiter.is_limited(&key, rates::REAUTH_FAILURES) {
        return Err(ApiError::RateLimited(std::time::Duration::from_secs(300)));
    }
    let Some(code) = code else {
        return Err(bad("2FA code required"));
    };
    if verify_second_factor(st, user_id, code).await? {
        Ok(())
    } else {
        let _ = st.limiter.check(&key, rates::REAUTH_FAILURES);
        Err(ApiError::InvalidCredentials)
    }
}

// ------------------------------------------------------------------ TOTP management

async fn totp_setup(State(st): State<AppState>, a: Auth) -> ApiResult<Json<serde_json::Value>> {
    let u = repo::user(&st.ctx.db, a.id())
        .await?
        .ok_or(ApiError::NotFound)?;
    if u.mfa_enabled {
        return Err(ApiError::Conflict("2FA already enabled"));
    }
    let secret = totp::generate_secret();
    let enc = auth::encrypt_secret(&st.ctx.secret_key, &secret);
    sqlx::query!(
        "UPDATE users SET mfa_secret_enc = $2, updated_at = now() WHERE id = $1",
        a.id() as i64,
        enc
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(Json(json!({
        "secret": totp::base32(&secret),
        "otpauth_url": totp::otpauth_url(&secret, &u.username, "NeoShard"),
    })))
}

#[derive(Deserialize)]
struct TotpChange {
    code: String,
    password: String,
}

async fn totp_enable(
    State(st): State<AppState>,
    a: Auth,
    Json(r): Json<TotpChange>,
) -> ApiResult<Json<serde_json::Value>> {
    check_password(&st, a.id(), &r.password).await?;
    let enc = sqlx::query_scalar!(
        "SELECT mfa_secret_enc FROM users WHERE id = $1 AND NOT mfa_enabled",
        a.id() as i64
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .flatten()
    .ok_or(bad("call /auth/mfa/totp/setup first"))?;
    let secret =
        auth::decrypt_secret(&st.ctx.secret_key, &enc).ok_or_else(|| anyhow::anyhow!("decrypt"))?;
    if !totp::verify(&secret, &r.code) {
        return Err(ApiError::InvalidCredentials);
    }
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "UPDATE users SET mfa_enabled = TRUE, updated_at = now() WHERE id = $1",
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM mfa_backup_codes WHERE user_id = $1",
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?;
    let mut codes = Vec::with_capacity(10);
    for _ in 0..10 {
        let raw = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            auth::random_bytes::<6>(),
        )
        .to_lowercase()
        .replace(['-', '_'], "x");
        let code = format!("{}-{}", &raw[..4], &raw[4..8]);
        sqlx::query!(
            "INSERT INTO mfa_backup_codes (user_id, code_hash) VALUES ($1, $2)",
            a.id() as i64,
            auth::hash_token(&code.replace('-', ""))
        )
        .execute(&mut *tx)
        .await?;
        codes.push(code);
    }
    // Other sessions must re-login with 2FA.
    sqlx::query!(
        "DELETE FROM sessions WHERE user_id = $1 AND id <> $2",
        a.id() as i64,
        a.0.session_id as i64
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    st.revoke_sessions(a.id(), vec![], true, a.0.session_id);
    Ok(Json(json!({ "backup_codes": codes })))
}

async fn totp_disable(
    State(st): State<AppState>,
    a: Auth,
    Json(r): Json<TotpChange>,
) -> ApiResult<StatusCode> {
    check_password(&st, a.id(), &r.password).await?;
    require_second_factor_if_enabled(&st, a.id(), Some(&r.code)).await?;
    sqlx::query!("UPDATE users SET mfa_enabled = FALSE, mfa_secret_enc = NULL, updated_at = now() WHERE id = $1", a.id() as i64)
        .execute(&st.ctx.db)
        .await?;
    sqlx::query!(
        "DELETE FROM mfa_backup_codes WHERE user_id = $1",
        a.id() as i64
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ sessions

async fn logout(State(st): State<AppState>, a: Auth) -> ApiResult<StatusCode> {
    sqlx::query!("DELETE FROM sessions WHERE id = $1", a.0.session_id as i64)
        .execute(&st.ctx.db)
        .await?;
    st.revoke_sessions(a.id(), vec![a.0.session_id], false, 0);
    Ok(StatusCode::NO_CONTENT)
}

async fn list_sessions(State(st): State<AppState>, a: Auth) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query!(
        "SELECT id, device, ip, created_at, last_seen_at FROM sessions WHERE user_id = $1 ORDER BY last_seen_at DESC",
        a.id() as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    let list: Vec<_> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "device": r.device,
                "ip": r.ip,
                "created_at": r.created_at.timestamp_millis(),
                "last_seen_at": r.last_seen_at.timestamp_millis(),
                "current": r.id as u64 == a.0.session_id,
            })
        })
        .collect();
    Ok(Json(json!(list)))
}

async fn revoke_session(
    State(st): State<AppState>,
    a: Auth,
    Path(id): Path<u64>,
) -> ApiResult<StatusCode> {
    let r = sqlx::query!(
        "DELETE FROM sessions WHERE id = $1 AND user_id = $2",
        id as i64,
        a.id() as i64
    )
    .execute(&st.ctx.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    st.revoke_sessions(a.id(), vec![id], false, 0);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ChangePassword {
    password: String,
    new_password: String,
    code: Option<String>,
}

async fn change_password(
    State(st): State<AppState>,
    a: Auth,
    Json(r): Json<ChangePassword>,
) -> ApiResult<StatusCode> {
    check_password(&st, a.id(), &r.password).await?;
    require_second_factor_if_enabled(&st, a.id(), r.code.as_deref()).await?;
    if !auth::password_ok(&r.new_password) {
        return Err(bad("password must be 8-128 characters"));
    }
    let pw = r.new_password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&pw))
        .await
        .map_err(anyhow::Error::from)??;
    sqlx::query!(
        "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1",
        a.id() as i64,
        hash
    )
    .execute(&st.ctx.db)
    .await?;
    sqlx::query!(
        "DELETE FROM sessions WHERE user_id = $1 AND id <> $2",
        a.id() as i64,
        a.0.session_id as i64
    )
    .execute(&st.ctx.db)
    .await?;
    st.revoke_sessions(a.id(), vec![], true, a.0.session_id);
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ recovery

#[derive(Deserialize)]
struct RecoverReq {
    email: String,
}

/// Always 202, whether or not the email exists (no account enumeration).
async fn recover(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(r): Json<RecoverReq>,
) -> ApiResult<StatusCode> {
    st.limiter.check(&format!("recover:{ip}"), rates::AUTH_IP)?;
    let email = r.email.trim().to_lowercase();
    let user = sqlx::query!(
        "SELECT id FROM users WHERE lower(email) = $1 AND deleted_at IS NULL AND NOT bot",
        email
    )
    .fetch_optional(&st.ctx.db)
    .await?;
    if let Some(u) = user {
        let (token, th) = auth::new_token("nsr_");
        sqlx::query!(
            "INSERT INTO password_resets (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '30 minutes')",
            th,
            u.id
        )
        .execute(&st.ctx.db)
        .await?;
        let link = format!(
            "{}/recover#token={}",
            st.ctx.cfg.public_url.trim_end_matches('/'),
            token
        );
        // In the background: response time must not reveal whether the email exists.
        let st2 = st.clone();
        tokio::spawn(async move {
            crate::mail::send(&st2, &email, "Восстановление доступа NeoShard", &format!("Ссылка для сброса пароля (действует 30 минут):\n{link}\n\nЕсли это были не вы — просто проигнорируйте письмо.")).await;
        });
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
struct RecoverConfirm {
    token: String,
    password: String,
}

async fn recover_confirm(
    State(st): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(r): Json<RecoverConfirm>,
) -> ApiResult<StatusCode> {
    st.limiter.check(&format!("recover:{ip}"), rates::AUTH_IP)?;
    if !auth::password_ok(&r.password) {
        return Err(bad("password must be 8-128 characters"));
    }
    let th = auth::hash_token(&r.token);
    let row = sqlx::query!(
        "UPDATE password_resets SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING user_id",
        th
    )
    .fetch_optional(&st.ctx.db)
    .await?
    .ok_or(ApiError::InvalidCredentials)?;
    let pw = r.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&pw))
        .await
        .map_err(anyhow::Error::from)??;
    sqlx::query!(
        "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1",
        row.user_id,
        hash
    )
    .execute(&st.ctx.db)
    .await?;
    sqlx::query!("DELETE FROM sessions WHERE user_id = $1", row.user_id)
        .execute(&st.ctx.db)
        .await?;
    st.revoke_sessions(row.user_id as u64, vec![], true, 0);
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ deletion

#[derive(Deserialize)]
struct DeleteReq {
    password: String,
    code: Option<String>,
}

/// Deletes personal data (152-ФЗ / GDPR): profile, email, sessions, keys.
/// Messages stay (they belong to conversations) but point to an anonymized account.
async fn delete_account(
    State(st): State<AppState>,
    a: Auth,
    Json(r): Json<DeleteReq>,
) -> ApiResult<StatusCode> {
    check_password(&st, a.id(), &r.password).await?;
    require_second_factor_if_enabled(&st, a.id(), r.code.as_deref()).await?;
    let owns = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM guilds WHERE owner_id = $1"#,
        a.id() as i64
    )
    .fetch_one(&st.ctx.db)
    .await?;
    if owns > 0 {
        return Err(ApiError::Conflict("transfer or delete your guilds first"));
    }
    let mut tx = st.ctx.db.begin().await?;
    sqlx::query!(
        "UPDATE users SET username = 'deleted_' || id::text, display_name = '', email = NULL, password_hash = NULL,
                avatar = '', mfa_secret_enc = NULL, mfa_enabled = FALSE, custom_status = '', deleted_at = now(),
                pronouns = '', bio = '', banner = '', banner_color = NULL, theme_colors = '{}',
                avatar_decoration = '', profile_effect = '', nameplate = '', name_font = '', name_effect = '',
                name_colors = '{}', custom_status_emoji = '', custom_status_expires_at = NULL, primary_guild_id = NULL
         WHERE id = $1",
        a.id() as i64
    )
    .execute(&mut *tx)
    .await?;
    for q in [
        "DELETE FROM sessions WHERE user_id = $1",
        "DELETE FROM members WHERE user_id = $1",
        "DELETE FROM mls_key_packages WHERE user_id = $1",
        "DELETE FROM push_subscriptions WHERE user_id = $1",
        "DELETE FROM mfa_backup_codes WHERE user_id = $1",
        "DELETE FROM read_states WHERE user_id = $1",
        "DELETE FROM user_links WHERE user_id = $1",
        "DELETE FROM user_notes WHERE owner_id = $1 OR target_id = $1",
        "DELETE FROM relationships WHERE user_id = $1 OR peer_id = $1",
    ] {
        sqlx::query(q).bind(a.id() as i64).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    st.revoke_sessions(a.id(), vec![], true, 0);
    Ok(StatusCode::NO_CONTENT)
}

/// Used by tests and the user route.
pub async fn load_me(st: &AppState, id: u64) -> ApiResult<v1::User> {
    me(st, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(valid_username("alice_01"));
        assert!(valid_username("a.b"));
        assert!(!valid_username("A"));
        assert!(!valid_username("Alice"));
        assert!(!valid_username("al ice"));
        assert!(!valid_username(&"a".repeat(33)));
    }

    #[test]
    fn emails() {
        assert!(valid_email("a@b.ru"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("@b.ru"));
        assert!(!valid_email("a b@c.ru"));
    }
}
