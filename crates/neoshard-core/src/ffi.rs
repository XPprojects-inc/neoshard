//! Kotlin / Swift bindings via uniffi (feature `ffi`).
//!
//! Same byte-level API as WASM and Tauri: protobuf `Command` in, protobuf
//! `UpdateBatch` out. Generate bindings with:
//!   cargo build -p neoshard-core --features ffi --release
//!   cargo run -p uniffi-bindgen -- generate --library target/release/libneoshard_core.so --language kotlin --out-dir out/
//! (see docs/mobile.md)

use std::sync::{Arc, Mutex};
use std::time::Duration;

use neoshard_proto::core_api::Command;
use prost::Message as _;

use crate::native::{NativeClient, SqliteStore, Store};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("invalid command: {msg}")]
    InvalidCommand { msg: String },
    #[error("storage: {msg}")]
    Storage { msg: String },
}

#[derive(uniffi::Object)]
pub struct NeoShardClient {
    rt: tokio::runtime::Runtime,
    client: Mutex<NativeClient>,
}

#[uniffi::export]
impl NeoShardClient {
    /// `db_path`: SQLite file for the offline cache (app-private storage).
    #[uniffi::constructor]
    pub fn new(client_name: String, db_path: String) -> Result<Arc<Self>, FfiError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| FfiError::Storage { msg: e.to_string() })?;
        let store: Arc<dyn Store> = Arc::new(
            SqliteStore::open(&db_path).map_err(|e| FfiError::Storage { msg: e.to_string() })?,
        );
        let client = rt.block_on(async { NativeClient::spawn(&client_name, store) });
        Ok(Arc::new(Self {
            rt,
            client: Mutex::new(client),
        }))
    }

    /// Send a protobuf-encoded `neoshard.core.v1.Command`.
    pub fn dispatch(&self, command: Vec<u8>) -> Result<(), FfiError> {
        let cmd = Command::decode(command.as_slice())
            .map_err(|e| FfiError::InvalidCommand { msg: e.to_string() })?;
        self.client.lock().expect("client lock").send(cmd);
        Ok(())
    }

    /// Wait up to `timeout_ms` for the next `UpdateBatch` (protobuf). Empty = none.
    /// Call from a background thread / coroutine in a loop.
    pub fn poll(&self, timeout_ms: u64) -> Vec<u8> {
        let mut client = self.client.lock().expect("client lock");
        self.rt.block_on(async {
            match tokio::time::timeout(Duration::from_millis(timeout_ms), client.updates.recv())
                .await
            {
                Ok(Some(batch)) => batch.encode_to_vec(),
                _ => Vec::new(),
            }
        })
    }
}
