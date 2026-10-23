//! Federation primitives (ADR-0014, spec/federation.md).
//!
//! Every server-to-server request carries:
//! * `NeoShard-Origin: <domain>` — the sending instance;
//! * `NeoShard-Date: <unix seconds>` — rejected if skew > [`MAX_SKEW_SECS`];
//! * `NeoShard-Nonce: <random>` — rejected if seen before (replay protection);
//! * `NeoShard-Signature: <base64 Ed25519 signature>` over [`canonical`].
//!
//! The origin's public key is fetched once from
//! `https://<domain>/.well-known/neoshard` (`federation.public_key`) and pinned
//! (trust on first use, operator can block/replace it).
//! SPDX-License-Identifier: AGPL-3.0-or-later

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

pub const MAX_SKEW_SECS: u64 = 300;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FedError {
    #[error("invalid key")]
    BadKey,
    #[error("invalid signature")]
    BadSignature,
    #[error("request date outside allowed skew")]
    Skew,
    #[error("missing header {0}")]
    Missing(&'static str),
}

pub struct InstanceKey {
    key: SigningKey,
}

impl InstanceKey {
    pub fn generate() -> Self {
        let seed: [u8; 32] = rand::random();
        Self {
            key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn from_seed_b64(seed: &str) -> Result<Self, FedError> {
        let raw = B64.decode(seed.trim()).map_err(|_| FedError::BadKey)?;
        let seed: [u8; 32] = raw.try_into().map_err(|_| FedError::BadKey)?;
        Ok(Self {
            key: SigningKey::from_bytes(&seed),
        })
    }

    pub fn seed_b64(&self) -> String {
        B64.encode(self.key.to_bytes())
    }

    pub fn public_b64(&self) -> String {
        B64.encode(self.key.verifying_key().to_bytes())
    }

    pub fn sign(&self, req: &SignedParts<'_>) -> String {
        B64.encode(self.key.sign(canonical(req).as_bytes()).to_bytes())
    }
}

/// The parts of a request covered by the signature.
#[derive(Debug, Clone, Copy)]
pub struct SignedParts<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub origin: &'a str,
    pub destination: &'a str,
    pub date: u64,
    pub nonce: &'a str,
    pub body: &'a [u8],
}

pub fn canonical(p: &SignedParts<'_>) -> String {
    let digest = B64.encode(Sha256::digest(p.body));
    format!(
        "neoshard-federation-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        p.method.to_ascii_uppercase(),
        p.path,
        p.origin,
        p.destination,
        p.date,
        p.nonce,
        digest
    )
}

pub fn verify(
    public_b64: &str,
    req: &SignedParts<'_>,
    signature_b64: &str,
    now: u64,
) -> Result<(), FedError> {
    if req.date.abs_diff(now) > MAX_SKEW_SECS {
        return Err(FedError::Skew);
    }
    let pk: [u8; 32] = B64
        .decode(public_b64)
        .map_err(|_| FedError::BadKey)?
        .try_into()
        .map_err(|_| FedError::BadKey)?;
    let pk = VerifyingKey::from_bytes(&pk).map_err(|_| FedError::BadKey)?;
    let sig: [u8; 64] = B64
        .decode(signature_b64)
        .map_err(|_| FedError::BadSignature)?
        .try_into()
        .map_err(|_| FedError::BadSignature)?;
    pk.verify(canonical(req).as_bytes(), &Signature::from_bytes(&sig))
        .map_err(|_| FedError::BadSignature)
}

pub fn new_nonce() -> String {
    let b: [u8; 16] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Federated user address `name@domain`.
pub fn parse_address(addr: &str) -> Option<(&str, &str)> {
    let (u, d) = addr.split_once('@')?;
    let ok_user = (2..=32).contains(&u.len())
        && u.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.');
    let ok_domain = !d.is_empty()
        && d.len() <= 253
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':');
    (ok_user && ok_domain).then_some((u, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts<'a>(body: &'a [u8], date: u64) -> SignedParts<'a> {
        SignedParts {
            method: "post",
            path: "/federation/v1/events",
            origin: "a.example",
            destination: "b.example",
            date,
            nonce: "n1",
            body,
        }
    }

    #[test]
    fn sign_and_verify() {
        let k = InstanceKey::generate();
        let p = parts(b"{}", 1000);
        let sig = k.sign(&p);
        assert_eq!(verify(&k.public_b64(), &p, &sig, 1000), Ok(()));
    }

    #[test]
    fn tampering_detected() {
        let k = InstanceKey::generate();
        let sig = k.sign(&parts(b"{}", 1000));
        assert_eq!(
            verify(&k.public_b64(), &parts(b"{\"x\":1}", 1000), &sig, 1000),
            Err(FedError::BadSignature)
        );
        let other = InstanceKey::generate();
        assert_eq!(
            verify(&other.public_b64(), &parts(b"{}", 1000), &sig, 1000),
            Err(FedError::BadSignature)
        );
    }

    #[test]
    fn skew_rejected() {
        let k = InstanceKey::generate();
        let p = parts(b"", 1000);
        let sig = k.sign(&p);
        assert_eq!(
            verify(&k.public_b64(), &p, &sig, 1000 + MAX_SKEW_SECS + 1),
            Err(FedError::Skew)
        );
    }

    #[test]
    fn seed_roundtrip() {
        let k = InstanceKey::generate();
        let k2 = InstanceKey::from_seed_b64(&k.seed_b64()).unwrap();
        assert_eq!(k.public_b64(), k2.public_b64());
        assert!(InstanceKey::from_seed_b64("short").is_err());
    }

    #[test]
    fn addresses() {
        assert_eq!(
            parse_address("alice@neoshard.ru"),
            Some(("alice", "neoshard.ru"))
        );
        assert!(parse_address("Alice@x.ru").is_none());
        assert!(parse_address("alice").is_none());
        assert!(parse_address("alice@exa mple").is_none());
    }
}
