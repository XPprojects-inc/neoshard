//! Uploads to S3-compatible storage via presigned PUT URLs. The server never
//! proxies file bytes: the client uploads directly to Object Storage.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use neoshard_proto::permissions::Permissions as P;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::authz::{channel_access, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::Auth;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{cid}/attachments", post(attachment))
        .route("/uploads/{kind}", post(user_asset))
        .route("/guilds/{gid}/uploads/{kind}", post(guild_asset))
}

const ALLOWED_IMAGE: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned
    }
}

fn bucket(st: &AppState) -> ApiResult<(Bucket, Credentials)> {
    let c = &st.ctx.cfg;
    let (Some(ep), Some(ak), Some(sk)) = (&c.s3_endpoint, &c.s3_access_key, &c.s3_secret_key)
    else {
        return Err(ApiError::Unavailable(
            "object storage is not configured on this instance",
        ));
    };
    let url = ep
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid S3_ENDPOINT"))?;
    let b = Bucket::new(
        url,
        UrlStyle::Path,
        c.s3_bucket.clone(),
        c.s3_region.clone(),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok((b, Credentials::new(ak.clone(), sk.clone())))
}

pub fn public_url(st: &AppState, key: &str) -> String {
    format!("{}/{key}", st.ctx.cfg.files_base())
}

/// Upload URL: S3 presigned PUT, or the local signed PUT (`NEOSHARD_FILES_DIR`).
fn presign_put(st: &AppState, key: &str, max: u64, content_type: &str) -> ApiResult<String> {
    let c = &st.ctx.cfg;
    if (c.s3_endpoint.is_none() || c.s3_access_key.is_none())
        && let Some(url) = crate::routes::files::signed_put_url(st, key, max, content_type)?
    {
        return Ok(url);
    }
    let (b, creds) = bucket(st)?;
    Ok(b.put_object(Some(&creds), key)
        .sign(Duration::from_secs(900))
        .to_string())
}

#[derive(Deserialize)]
struct UploadReq {
    filename: String,
    content_type: String,
    size: u64,
}

async fn attachment(
    State(st): State<AppState>,
    a: Auth,
    Path(cid): Path<u64>,
    Json(r): Json<UploadReq>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let ca = channel_access(&st, cid, a.id()).await?;
    ca.require(P::ATTACH_FILES)?;
    if r.size == 0 || r.size > st.ctx.cfg.max_upload_bytes {
        return Err(bad(format!(
            "size must be 1..={} bytes",
            st.ctx.cfg.max_upload_bytes
        )));
    }
    if r.content_type.len() > 100 || !r.content_type.contains('/') {
        return Err(bad("invalid content_type"));
    }
    let id = st.ctx.id();
    let name = sanitize_filename(&r.filename);
    let key = format!("attachments/{cid}/{id}/{name}");
    let upload_url = presign_put(&st, &key, r.size, &r.content_type)?;
    sqlx::query!(
        "INSERT INTO attachments (id, uploader_id, channel_id, filename, content_type, size, object_key) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        id as i64,
        a.id() as i64,
        cid as i64,
        name,
        r.content_type,
        r.size as i64,
        key
    )
    .execute(&st.ctx.db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id.to_string(), "upload_url": upload_url, "url": public_url(&st, &key), "method": "PUT" }),
        ),
    ))
}

/// Profile images of the user: `avatar` or `banner`.
async fn user_asset(
    State(st): State<AppState>,
    a: Auth,
    Path(kind): Path<String>,
    Json(r): Json<UploadReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let prefix = match kind.as_str() {
        "avatar" => "avatars",
        "banner" => "banners",
        _ => return Err(ApiError::NotFound),
    };
    image_upload(&st, format!("{prefix}/{}/{}", a.id(), st.ctx.id()), &r)
}

fn image_upload(st: &AppState, key: String, r: &UploadReq) -> ApiResult<Json<serde_json::Value>> {
    if !ALLOWED_IMAGE.contains(&r.content_type.as_str()) || r.size == 0 || r.size > 8 * 1024 * 1024
    {
        return Err(bad("png/jpeg/webp/gif up to 8 MiB"));
    }
    Ok(Json(json!({
        "key": key,
        "upload_url": presign_put(st, &key, r.size, &r.content_type)?,
        "url": public_url(st, &key),
    })))
}

/// Object-key prefix a user may reference as their avatar/banner.
pub fn user_key_ok(key: &str, kind: &str, uid: u64) -> bool {
    key.is_empty() || key.starts_with(&format!("{kind}/{uid}/"))
}

/// Object-key prefix for a per-server profile image.
pub fn member_key_ok(key: &str, gid: u64, uid: u64) -> bool {
    key.is_empty() || key.starts_with(&format!("members/{gid}/{uid}/"))
}

async fn guild_asset(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, kind)): Path<(u64, String)>,
    Json(r): Json<UploadReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let ga = guild_access(&st, gid, a.id()).await?;
    // Per-server profile images need only membership.
    if kind == "member-avatar" || kind == "member-banner" {
        return image_upload(&st, format!("members/{gid}/{}/{}", a.id(), st.ctx.id()), &r);
    }
    let (need, prefix, max) = match kind.as_str() {
        "icon" => (P::MANAGE_GUILD, "icons", 8 * 1024 * 1024),
        "emoji" => (P::MANAGE_EMOJIS, "emojis", 256 * 1024),
        "sticker" => (P::MANAGE_EMOJIS, "stickers", 512 * 1024),
        _ => return Err(ApiError::NotFound),
    };
    ga.require(need)?;
    let ok_type = ALLOWED_IMAGE.contains(&r.content_type.as_str())
        || (kind == "sticker" && r.content_type == "application/json");
    if !ok_type || r.size == 0 || r.size > max {
        return Err(bad(format!("image up to {max} bytes")));
    }
    let key = format!("{prefix}/{gid}/{}", st.ctx.id());
    Ok(Json(
        json!({ "key": key, "upload_url": presign_put(&st, &key, r.size, &r.content_type)?, "url": public_url(&st, &key) }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\x\\отчёт 2026.pdf"), "отчёт_2026.pdf");
        assert_eq!(sanitize_filename(".htaccess"), "htaccess");
        assert_eq!(sanitize_filename(""), "file");
    }
}
