//! # neoshard-proto
//!
//! Single source of truth for the NeoShard protocol, shared by the server and
//! by `neoshard-core`:
//!
//! * [`v1`] — generated protobuf types for the wire protocol (`spec/proto`),
//!   with canonical proto3 JSON (serde) for REST;
//! * [`core_api`] — the client core's command/update API;
//! * [`permissions`] — permission bits and the one permission algorithm;
//! * [`snowflake`] — ID generation and decoding;
//! * [`framing`] — Gateway frame encoding with optional per-frame zstd.

#![forbid(unsafe_code)]

pub mod framing;
pub mod permissions;
pub mod profile;
pub mod snowflake;

/// Generated code; module nesting mirrors the proto packages.
#[allow(clippy::all, missing_docs)]
pub mod generated {
    pub mod neoshard {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/neoshard.v1.rs"));
            include!(concat!(env!("OUT_DIR"), "/neoshard.v1.serde.rs"));
        }
        pub mod core {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/neoshard.core.v1.rs"));
                include!(concat!(env!("OUT_DIR"), "/neoshard.core.v1.serde.rs"));
            }
        }
    }
}

/// Client core API types (`package neoshard.core.v1`).
pub use generated::neoshard::core::v1 as core_api;
/// Wire protocol types (`package neoshard.v1`).
pub use generated::neoshard::v1;

/// Encoded `FileDescriptorSet` of the whole protocol, for reflection-based tooling.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/descriptor.bin"));

/// Wire protocol version spoken by this crate.
pub const PROTOCOL_VERSION: u32 = 1;

pub use prost::Message as ProstMessage;
