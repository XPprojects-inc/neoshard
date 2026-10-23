use std::net::SocketAddr;

use clap::Parser;
use neoshard_server_common::{Config, Ctx};
use tracing_subscriber::EnvFilter;

// mimalloc: lower fragmentation after Identify bursts, faster small allocations.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> anyhow::Result<()> {
    // Before the runtime starts any thread (set_var is only sound single-threaded).
    let _ = dotenv_lite();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let cfg = Config::parse();
    let json = std::env::var("NEOSHARD_LOG_FORMAT").is_ok_and(|v| v == "json");
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    if json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    let bind = cfg.bind;
    let ctx = Ctx::connect(cfg).await?;
    let node = neoshard_server::build(ctx.clone());
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, services = %ctx.cfg.only, "NeoShard server listening");
    axum::serve(
        listener,
        node.router
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
    })
    .await?;
    Ok(())
}

/// Load `KEY=VALUE` lines from `./.env` into the environment if present (dev convenience).
fn dotenv_lite() -> std::io::Result<()> {
    let text = std::fs::read_to_string(".env")?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && std::env::var_os(k.trim()).is_none()
        {
            // SAFETY: called from main() before the tokio runtime spawns threads.
            unsafe { std::env::set_var(k.trim(), v.trim()) };
        }
    }
    Ok(())
}
