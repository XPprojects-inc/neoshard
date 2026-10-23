//! # neoshard-core
//!
//! The NeoShard client core — the TDLib of NeoShard. All client logic lives
//! here; a UI (Solid web app, Tauri, Kotlin, Swift, a terminal) only renders
//! [`Update`](neoshard_proto::core_api::Update)s and sends
//! [`Command`](neoshard_proto::core_api::Command)s.
//!
//! The core is **sans-IO** ([`core::Core`]): it never touches sockets,
//! clocks or threads. Drivers execute its [`core::Effect`]s:
//! * [`native`] — tokio (CLI, desktop, mobile via uniffi);
//! * `wasm` — the browser (`@neoshard/core` on npm).
//!
//! API stability: semantic versioning of this crate and of `core.proto`;
//! see `docs/core-api.md`.

pub mod cache;
pub mod core;
pub mod gateway;
pub mod outbox;

#[cfg(feature = "native")]
pub mod native;

#[cfg(feature = "wasm")]
pub mod wasm;

#[cfg(feature = "audio")]
pub mod audio;

#[cfg(feature = "e2ee")]
pub mod e2ee;

#[cfg(feature = "ffi")]
pub mod ffi;

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

pub use crate::core::{Core, Effect, HttpRequest, HttpResponse};
pub use neoshard_proto as proto;
pub use neoshard_proto::core_api as api;
