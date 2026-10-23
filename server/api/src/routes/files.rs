//! Local object storage for single-node and self-hosted instances without S3
//! (`NEOSHARD_FILES_DIR`). Same flow as S3: the API hands out a short-lived
//! signed PUT URL, the client uploads the bytes there, and `/files/{key}` serves
//! them. The signature binds the key, expiry, maximum size and content type.
//!
//! Serving is locked down: only images are rendered inline, everything else is
//! a download, and a sandbox CSP stops an uploaded HTML/SVG from running script.

use std::path::{Component, Path as FsPath, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tokio::io::AsyncWriteExt;

use crate::AppState;
use crate::error::{ApiError, ApiResult, bad};

/// Inline-safe image types; anything else is served as a download.
const INLINE: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];
const URL_TTL_SECS: u64 = 900;

/// `PUT /api/v1/files/{key}` (mounted under /api/v1).
pub fn upload_router() -> Router<AppState> {
    Router::new().route(
        "/files/{*key}",
        put(upload).layer(DefaultBodyLimit::disable()),
    )
}

/// `GET /files/{key}` (mounted at the root).
pub fn serve_router() -> Router<AppState> {
    Router::new().route("/files/{*key}", get(serve))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sign(st: &AppState, key: &str, exp: u64, max: u64, ct: &str) -> ApiResult<String> {
    let secret = st.ctx.cfg.secret_key_bytes()?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret).map_err(anyhow::Error::from)?;
    mac.update(format!("files\n{key}\n{exp}\n{max}\n{ct}").as_bytes());
    Ok(B64.encode(mac.finalize().into_bytes()))
}

/// Signed upload URL for `key`, or `None` when local storage is not configured.
pub fn signed_put_url(st: &AppState, key: &str, max: u64, ct: &str) -> ApiResult<Option<String>> {
    if st.ctx.cfg.files_dir.is_none() {
        return Ok(None);
    }
    let exp = now_secs() + URL_TTL_SECS;
    let sig = sign(st, key, exp, max, ct)?;
    let base = st.ctx.cfg.public_url.trim_end_matches('/');
    let ct_q = url::form_urlencoded::byte_serialize(ct.as_bytes()).collect::<String>();
    Ok(Some(format!(
        "{base}/api/v1/files/{key}?exp={exp}&max={max}&ct={ct_q}&sig={sig}"
    )))
}

/// Resolve a key inside the storage directory; rejects anything but plain segments.
fn resolve(dir: &FsPath, key: &str) -> Option<PathBuf> {
    if key.is_empty() || key.len() > 512 || key.contains('\\') || key.contains('\0') {
        return None;
    }
    let rel = FsPath::new(key);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(dir.join(rel))
}

fn sidecar(p: &FsPath) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".ct");
    PathBuf::from(s)
}

#[derive(Deserialize)]
struct PutQuery {
    exp: u64,
    max: u64,
    ct: String,
    sig: String,
}

async fn upload(
    State(st): State<AppState>,
    Path(key): Path<String>,
    Query(q): Query<PutQuery>,
    body: Body,
) -> ApiResult<StatusCode> {
    let dir = st.ctx.cfg.files_dir.clone().ok_or(ApiError::NotFound)?;
    let expected = sign(&st, &key, q.exp, q.max, &q.ct)?;
    if !neoshard_server_common::auth::ct_eq(expected.as_bytes(), q.sig.as_bytes())
        || q.exp < now_secs()
    {
        return Err(ApiError::Forbidden("upload URL is invalid or expired"));
    }
    let path = resolve(&dir, &key).ok_or_else(|| bad("invalid key"))?;
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(ApiError::Conflict("object already exists"));
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(anyhow::Error::from)?;
    }
    let tmp = path.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(anyhow::Error::from)?;
    let mut stream = body.into_data_stream();
    let mut written: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| bad(format!("upload interrupted: {e}")))?;
        written += chunk.len() as u64;
        if written > q.max {
            drop(file);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(bad("file is larger than declared"));
        }
        file.write_all(&chunk).await.map_err(anyhow::Error::from)?;
    }
    file.flush().await.map_err(anyhow::Error::from)?;
    drop(file);
    if written == 0 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(bad("empty upload"));
    }
    tokio::fs::write(sidecar(&path), q.ct.as_bytes())
        .await
        .map_err(anyhow::Error::from)?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(anyhow::Error::from)?;
    Ok(StatusCode::OK)
}

async fn serve(State(st): State<AppState>, Path(key): Path<String>) -> Response {
    let Some(dir) = st.ctx.cfg.files_dir.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(path) = resolve(&dir, &key) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let ct = tokio::fs::read_to_string(sidecar(&path))
        .await
        .unwrap_or_default();
    let inline = INLINE.contains(&ct.as_str());
    let mut h = HeaderMap::new();
    let ct_value = if inline {
        ct.as_str()
    } else {
        "application/octet-stream"
    };
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(ct_value)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if !inline {
        h.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        "Cross-Origin-Resource-Policy",
        HeaderValue::from_static("cross-origin"),
    );
    (h, bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_stay_inside_the_directory() {
        let d = FsPath::new("/data");
        assert!(resolve(d, "avatars/1/2").is_some());
        assert!(resolve(d, "../etc/passwd").is_none());
        assert!(resolve(d, "avatars/../../x").is_none());
        assert!(resolve(d, "/abs").is_none());
        assert!(resolve(d, "a\\b").is_none());
        assert!(resolve(d, "").is_none());
    }
}
