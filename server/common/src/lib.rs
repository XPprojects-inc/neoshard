//! Shared server infrastructure for NeoShard services.
//! SPDX-License-Identifier: AGPL-3.0-or-later

pub mod auth;
pub mod bus;
pub mod config;
pub mod repo;

use std::sync::Arc;

pub use config::Config;
use neoshard_proto::snowflake::Generator;
use sqlx::postgres::PgPoolOptions;

/// Process-wide context shared by all services of a node.
pub struct Ctx {
    pub cfg: Config,
    pub db: sqlx::PgPool,
    pub ids: Generator,
    pub bus: bus::SharedBus,
    pub sessions: auth::SessionCache,
    pub secret_key: [u8; 32],
}

pub type SharedCtx = Arc<Ctx>;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../migrations");

impl Ctx {
    pub async fn connect(cfg: Config) -> anyhow::Result<SharedCtx> {
        let db = PgPoolOptions::new()
            .max_connections(cfg.db_max_connections)
            .connect(&cfg.database_url)
            .await?;
        MIGRATOR.run(&db).await?;
        let bus: bus::SharedBus = match &cfg.nats_url {
            #[cfg(feature = "nats")]
            Some(url) => bus::nats::NatsBus::connect(url).await?,
            #[cfg(not(feature = "nats"))]
            Some(_) => anyhow::bail!("NATS_URL set but server built without the `nats` feature"),
            None => Arc::new(bus::LocalBus::new()),
        };
        Self::with_pool(cfg, db, bus)
    }

    pub fn with_pool(
        cfg: Config,
        db: sqlx::PgPool,
        bus: bus::SharedBus,
    ) -> anyhow::Result<SharedCtx> {
        let secret_key = cfg.secret_key_bytes()?;
        Ok(Arc::new(Self {
            ids: Generator::new(cfg.worker_id),
            cfg,
            db,
            bus,
            sessions: Default::default(),
            secret_key,
        }))
    }

    pub fn id(&self) -> u64 {
        self.ids.next()
    }
}
