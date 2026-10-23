//! Server configuration. Every option is a CLI flag and an environment
//! variable (`NEOSHARD_*`), so the same binary runs under docker compose,
//! Helm or systemd without config files.

use std::net::SocketAddr;

use base64::Engine;

#[derive(clap::Parser, Clone, Debug)]
#[command(
    name = "neoshard-server",
    version,
    about = "NeoShard server (API + Gateway + presence + media-auth)"
)]
pub struct Config {
    /// Address to listen on (HTTP API and WebSocket gateway share one port → works through 443 only).
    #[arg(long, env = "NEOSHARD_BIND", default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,

    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,

    #[arg(long, env = "NEOSHARD_DB_MAX_CONNECTIONS", default_value_t = 32)]
    pub db_max_connections: u32,

    /// Public base URL of this instance, e.g. https://neoshard.ru
    #[arg(
        long,
        env = "NEOSHARD_PUBLIC_URL",
        default_value = "http://localhost:8080"
    )]
    pub public_url: String,

    /// Public gateway URL; defaults to `{public_url}` with ws(s) scheme.
    #[arg(long, env = "NEOSHARD_GATEWAY_URL")]
    pub gateway_url: Option<String>,

    /// Federation domain of this instance.
    #[arg(long, env = "NEOSHARD_DOMAIN", default_value = "localhost")]
    pub domain: String,

    /// Snowflake worker id of this node (0..1023). Must be unique per node.
    #[arg(long, env = "NEOSHARD_WORKER_ID", default_value_t = 0)]
    pub worker_id: u16,

    /// 32-byte key, base64. Encrypts TOTP secrets at rest.
    #[arg(long, env = "NEOSHARD_SECRET_KEY")]
    pub secret_key: String,

    #[arg(long, env = "NEOSHARD_REGISTRATION_OPEN", default_value_t = true, action = clap::ArgAction::Set)]
    pub registration_open: bool,

    /// Run only some services: `all`, `api`, `gateway` (comma separated).
    #[arg(long, env = "NEOSHARD_ONLY", default_value = "all")]
    pub only: String,

    #[arg(long, env = "NEOSHARD_HEARTBEAT_MS", default_value_t = 30_000)]
    pub heartbeat_interval_ms: u32,

    /// Trust `X-Forwarded-For` / `X-Real-IP` (set when behind your own reverse proxy).
    #[arg(long, env = "NEOSHARD_TRUST_PROXY", default_value_t = false, action = clap::ArgAction::Set)]
    pub trust_proxy: bool,

    // ---- media (LiveKit + TURN), ADR-0012
    #[arg(long, env = "LIVEKIT_URL")]
    pub livekit_url: Option<String>,
    #[arg(long, env = "LIVEKIT_API_KEY")]
    pub livekit_api_key: Option<String>,
    #[arg(long, env = "LIVEKIT_API_SECRET")]
    pub livekit_api_secret: Option<String>,
    /// Comma separated TURN URLs, e.g. `turns:turn.neoshard.ru:443?transport=tcp`.
    #[arg(long, env = "NEOSHARD_TURN_URLS", value_delimiter = ',')]
    pub turn_urls: Vec<String>,
    /// coturn `static-auth-secret` for time-limited TURN credentials.
    #[arg(long, env = "NEOSHARD_TURN_SECRET")]
    pub turn_secret: Option<String>,

    // ---- object storage (S3-compatible: Yandex Object Storage, MinIO, Selectel)
    #[arg(long, env = "S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,
    #[arg(long, env = "S3_BUCKET", default_value = "neoshard")]
    pub s3_bucket: String,
    #[arg(long, env = "S3_REGION", default_value = "ru-central1")]
    pub s3_region: String,
    #[arg(long, env = "S3_ACCESS_KEY")]
    pub s3_access_key: Option<String>,
    #[arg(long, env = "S3_SECRET_KEY")]
    pub s3_secret_key: Option<String>,
    /// Public (CDN) base URL for objects; defaults to `{endpoint}/{bucket}`.
    #[arg(long, env = "S3_PUBLIC_URL")]
    pub s3_public_url: Option<String>,
    #[arg(long, env = "NEOSHARD_MAX_UPLOAD_BYTES", default_value_t = 25 * 1024 * 1024)]
    pub max_upload_bytes: u64,
    /// Local file storage for single-node / self-hosted setups without S3:
    /// uploads go to this directory and are served at `/files/*`.
    #[arg(long, env = "NEOSHARD_FILES_DIR")]
    pub files_dir: Option<std::path::PathBuf>,

    // ---- federation, ADR-0014
    #[arg(long, env = "NEOSHARD_FEDERATION", default_value_t = false, action = clap::ArgAction::Set)]
    pub federation_enabled: bool,
    /// Ed25519 signing key seed (32 bytes, base64).
    #[arg(long, env = "NEOSHARD_FEDERATION_KEY")]
    pub federation_key: Option<String>,

    // ---- cluster
    /// NATS URL; when unset events stay in-process (single node).
    #[arg(long, env = "NATS_URL")]
    pub nats_url: Option<String>,

    // ---- email (password recovery). Self-hosted / RU provider SMTP URL.
    #[arg(long, env = "NEOSHARD_SMTP_URL")]
    pub smtp_url: Option<String>,
    #[arg(
        long,
        env = "NEOSHARD_SMTP_FROM",
        default_value = "NeoShard <noreply@localhost>"
    )]
    pub smtp_from: String,

    /// Development mode: permissive CORS, recovery tokens are logged instead of mailed.
    #[arg(long, env = "NEOSHARD_DEV", default_value_t = false, action = clap::ArgAction::Set)]
    pub dev: bool,
}

impl Config {
    pub fn secret_key_bytes(&self) -> anyhow::Result<[u8; 32]> {
        let raw = base64::engine::general_purpose::STANDARD.decode(self.secret_key.trim())?;
        raw.try_into()
            .map_err(|_| anyhow::anyhow!("NEOSHARD_SECRET_KEY must be 32 bytes (base64)"))
    }

    pub fn gateway_public_url(&self) -> String {
        self.gateway_url.clone().unwrap_or_else(|| {
            let base = self.public_url.trim_end_matches('/');
            let ws = if let Some(rest) = base.strip_prefix("https://") {
                format!("wss://{rest}")
            } else if let Some(rest) = base.strip_prefix("http://") {
                format!("ws://{rest}")
            } else {
                base.to_string()
            };
            format!("{ws}/gateway")
        })
    }

    /// Public base URL for stored objects (avatars, banners, attachments).
    pub fn files_base(&self) -> String {
        match (&self.s3_public_url, &self.s3_endpoint) {
            (Some(p), _) => p.trim_end_matches('/').to_string(),
            (None, Some(ep)) if self.s3_access_key.is_some() => {
                format!("{}/{}", ep.trim_end_matches('/'), self.s3_bucket)
            }
            _ => format!("{}/files", self.public_url.trim_end_matches('/')),
        }
    }

    pub fn runs(&self, service: &str) -> bool {
        self.only
            .split(',')
            .any(|s| s.trim() == "all" || s.trim() == service)
    }

    /// Minimal config for tests and examples.
    pub fn for_tests(database_url: &str) -> Self {
        use clap::Parser;
        let mut c = Config::parse_from([
            "neoshard-server",
            "--database-url",
            database_url,
            "--secret-key",
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=",
        ]);
        c.dev = true;
        c.heartbeat_interval_ms = 1_000;
        c
    }
}
