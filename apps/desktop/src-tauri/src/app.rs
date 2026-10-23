//! NeoShard app (desktop and Android, see apps/android): the web UI in a system WebView, with neoshard-core
//! running natively (no WASM). The UI talks to the core with two commands:
//!   core_dispatch(bytes)  — protobuf Command, synchronous so order is kept
//!   core_drain() -> bytes — all pending UpdateBatches, concatenated
//!                           (concatenated protobuf messages merge into one)
//! and the `core-updates` event, which says "something is waiting".

use std::sync::{Arc, Mutex};

use neoshard_core::api::Command;
use neoshard_core::native::{NativeClient, SqliteStore, Store};
use prost::Message as _;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{Emitter, Manager};

struct CoreState {
    client: Mutex<NativeClient>,
    pending: Arc<Mutex<Vec<u8>>>,
}

/// The command bytes. Desktop sends them raw over the `ipc:` protocol; Android
/// cannot read request bodies there, so Tauri uses postMessage, which turns a
/// Uint8Array into a JSON array of numbers.
fn command_bytes(body: &InvokeBody) -> Result<Vec<u8>, String> {
    match body {
        InvokeBody::Raw(bytes) => Ok(bytes.clone()),
        InvokeBody::Json(serde_json::Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| "expected bytes".to_string())
            })
            .collect(),
        _ => Err("expected bytes".into()),
    }
}

#[tauri::command]
fn core_dispatch(request: Request<'_>, state: tauri::State<'_, CoreState>) -> Result<(), String> {
    let bytes = command_bytes(request.body())?;
    let cmd = Command::decode(bytes.as_slice()).map_err(|e| e.to_string())?;
    state
        .client
        .lock()
        .map_err(|_| "core lock poisoned")?
        .send(cmd);
    Ok(())
}

#[tauri::command]
fn core_drain(state: tauri::State<'_, CoreState>) -> Response {
    let bytes = std::mem::take(&mut *state.pending.lock().expect("pending lock"));
    Response::new(bytes)
}

/// Entry point for desktop (`main.rs`) and mobile (Android/iOS).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store: Arc<dyn Store> = Arc::new(SqliteStore::open(dir.join("neoshard.db"))?);
            let mut client = tauri::async_runtime::block_on(async {
                NativeClient::spawn(
                    if cfg!(mobile) {
                        "neoshard-android"
                    } else {
                        "neoshard-desktop"
                    },
                    store,
                )
            });
            // Move the update stream out; the client handle stays for dispatching.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            std::mem::swap(&mut client.updates, &mut rx);
            drop(tx);
            let pending = Arc::new(Mutex::new(Vec::new()));
            let sink = pending.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(batch) = rx.recv().await {
                    batch
                        .encode(&mut *sink.lock().expect("pending lock"))
                        .expect("vec grows");
                    let _ = handle.emit("core-updates", ());
                }
            });
            app.manage(CoreState {
                client: Mutex::new(client),
                pending,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![core_dispatch, core_drain])
        .run(tauri::generate_context!())
        .expect("error while running NeoShard");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_bytes_from_both_ipc_paths() {
        assert_eq!(
            command_bytes(&InvokeBody::Raw(vec![1, 2, 255])).unwrap(),
            vec![1, 2, 255]
        );
        let json = InvokeBody::Json(serde_json::json!([1, 2, 255]));
        assert_eq!(command_bytes(&json).unwrap(), vec![1, 2, 255]);
        assert!(command_bytes(&InvokeBody::Json(serde_json::json!([256]))).is_err());
        assert!(command_bytes(&InvokeBody::Json(serde_json::json!({"0": 1}))).is_err());
    }
}
