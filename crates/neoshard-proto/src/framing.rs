//! Gateway framing.
//!
//! One WebSocket binary message = one protobuf `GatewayFrame`, optionally
//! wrapped in an independent zstd frame. Compression is self-describing: a
//! zstd frame starts with the magic `28 B5 2F FD`, while a `GatewayFrame`
//! can never start with byte `0x28` (that would be field 5 with the varint
//! wire type, and every `GatewayFrame` field is length-delimited). Decoders
//! therefore accept both, and encoders choose per connection.

use prost::Message;

use crate::v1::GatewayFrame;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// Upper bound for a decompressed frame (protects against zstd bombs).
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// Frames smaller than this are sent uncompressed even when compression is on:
/// zstd gains nothing on them and costs CPU per recipient.
pub const COMPRESS_MIN_BYTES: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("protobuf decode: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("zstd: {0}")]
    Zstd(String),
    #[error("frame too large")]
    TooLarge,
    #[error("compressed frame received but zstd support is not compiled in")]
    NoZstd,
}

/// Whether `bytes` is a zstd-compressed frame.
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == ZSTD_MAGIC
}

/// Whether this build can produce compressed frames.
pub const fn can_compress() -> bool {
    cfg!(feature = "zstd-native")
}

/// Encode a frame; compresses with zstd when `compress` and an encoder is compiled in.
pub fn encode(frame: &GatewayFrame, compress: bool) -> Vec<u8> {
    let raw = frame.encode_to_vec();
    #[cfg(feature = "zstd-native")]
    if compress
        && raw.len() >= COMPRESS_MIN_BYTES
        && let Ok(c) = zstd::bulk::compress(&raw, 3)
    {
        return c;
    }
    let _ = compress;
    raw
}

/// Decode a frame, transparently decompressing zstd.
pub fn decode(bytes: &[u8]) -> Result<GatewayFrame, FrameError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    if is_compressed(bytes) {
        let raw = decompress(bytes)?;
        return Ok(GatewayFrame::decode(raw.as_slice())?);
    }
    Ok(GatewayFrame::decode(bytes)?)
}

#[cfg(feature = "zstd-native")]
fn decompress(bytes: &[u8]) -> Result<Vec<u8>, FrameError> {
    zstd::bulk::decompress(bytes, MAX_FRAME_BYTES).map_err(|e| FrameError::Zstd(e.to_string()))
}

#[cfg(all(not(feature = "zstd-native"), feature = "zstd-pure"))]
fn decompress(bytes: &[u8]) -> Result<Vec<u8>, FrameError> {
    use std::io::Read;
    let dec = ruzstd::decoding::StreamingDecoder::new(bytes)
        .map_err(|e| FrameError::Zstd(e.to_string()))?;
    let mut out = Vec::new();
    dec.take(MAX_FRAME_BYTES as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| FrameError::Zstd(e.to_string()))?;
    if out.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    Ok(out)
}

#[cfg(all(not(feature = "zstd-native"), not(feature = "zstd-pure")))]
fn decompress(_bytes: &[u8]) -> Result<Vec<u8>, FrameError> {
    Err(FrameError::NoZstd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::{Heartbeat, Hello, gateway_frame::Kind};

    fn hello() -> GatewayFrame {
        GatewayFrame {
            kind: Some(Kind::Hello(Hello {
                heartbeat_interval_ms: 30_000,
                node: "n1".into(),
            })),
        }
    }

    #[test]
    fn raw_roundtrip() {
        let bytes = encode(&hello(), false);
        assert!(!is_compressed(&bytes));
        assert_eq!(decode(&bytes).unwrap(), hello());
    }

    #[test]
    fn small_frames_stay_raw() {
        assert!(!is_compressed(&encode(&hello(), true)));
    }

    #[test]
    fn compressed_roundtrip() {
        let big = GatewayFrame {
            kind: Some(Kind::Hello(Hello {
                heartbeat_interval_ms: 1,
                node: "x".repeat(2048),
            })),
        };
        let bytes = encode(&big, true);
        assert!(is_compressed(&bytes));
        assert!(bytes.len() < 200);
        assert_eq!(decode(&bytes).unwrap(), big);
    }

    #[test]
    fn compressed_roundtrip_small_still_decodes() {
        let bytes = encode(&hello(), true);
        assert_eq!(decode(&bytes).unwrap(), hello());
    }

    #[test]
    fn frames_never_start_with_zstd_magic_byte() {
        for n in 1..=12u32 {
            let tag = (n << 3) | 2;
            assert_ne!(tag as u8, 0x28);
        }
        let hb = GatewayFrame {
            kind: Some(Kind::Heartbeat(Heartbeat { last_seq: 0 })),
        };
        assert_ne!(encode(&hb, false).first().copied(), Some(0x28));
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(&[0x28, 0xB5, 0x2F, 0xFD, 1, 2, 3]).is_err());
    }
}
