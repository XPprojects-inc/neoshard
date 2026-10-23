//! End-to-end encryption for DMs and group DMs: MLS (RFC 9420) via openmls,
//! feature `e2ee` (ADR-0013).
//!
//! One MLS group per E2EE channel (group id = channel id). The server only
//! stores KeyPackages and relays opaque bytes:
//! * `key_packages()` → upload with `POST /mls/key-packages`;
//! * `add_members()` → send the commit to the channel (`POST /channels/{id}/mls`,
//!   kind "commit") and the welcome to the new members (kind "welcome");
//! * `encrypt()` → `e2ee_payload` of a message; `process()` on receive.
//!
//! Limitation (documented in PROGRESS.md): group state lives in the provider's
//! in-memory storage; persisting it requires a storage provider backed by
//! IndexedDB / SQLite (openmls `StorageProvider`).

use std::collections::HashMap;

use openmls::prelude::tls_codec::{Deserialize as _, Serialize as _};
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;

pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;
/// KeyPackages published per device (one is consumed per group a peer adds us to).
pub const KEY_PACKAGES: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum E2eeError {
    #[error("mls: {0}")]
    Mls(String),
    #[error("unknown group for channel {0}")]
    UnknownGroup(u64),
    #[error("malformed message")]
    Malformed,
}

fn mls<E: std::fmt::Debug>(e: E) -> E2eeError {
    E2eeError::Mls(format!("{e:?}"))
}

/// What an incoming MLS message turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    Application(Vec<u8>),
    /// A commit was applied (membership or keys changed).
    Commit,
    Other,
}

pub struct Device {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    identity: String,
    groups: HashMap<u64, MlsGroup>,
}

const EXPORT_VERSION: u8 = 1;

fn put(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

fn take<'a>(r: &mut &'a [u8]) -> Result<&'a [u8], E2eeError> {
    if r.len() < 4 {
        return Err(E2eeError::Malformed);
    }
    let n = u32::from_le_bytes(r[..4].try_into().expect("4 bytes")) as usize;
    if r.len() < 4 + n {
        return Err(E2eeError::Malformed);
    }
    let v = &r[4..4 + n];
    *r = &r[4 + n..];
    Ok(v)
}

fn group_id(channel_id: u64) -> GroupId {
    GroupId::from_slice(&channel_id.to_be_bytes())
}

fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .build()
}

impl Device {
    /// `identity` is what other members see as the credential, e.g. "user_id:device_id".
    pub fn new(identity: &str) -> Result<Self, E2eeError> {
        let provider = OpenMlsRustCrypto::default();
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).map_err(mls)?;
        signer.store(provider.storage()).map_err(mls)?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(identity.as_bytes().to_vec()).into(),
            signature_key: signer.to_public_vec().into(),
        };
        Ok(Self {
            provider,
            signer,
            credential,
            identity: identity.to_string(),
            groups: HashMap::new(),
        })
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Serialize the whole device (signing key, KeyPackage private keys, group
    /// states) for the local store. Keep it as private as the session token.
    pub fn export(&self) -> Vec<u8> {
        let mut out = vec![EXPORT_VERSION];
        put(&mut out, self.identity.as_bytes());
        put(&mut out, self.signer.public());
        let ids: Vec<u8> = self.groups.keys().flat_map(|c| c.to_le_bytes()).collect();
        put(&mut out, &ids);
        let values = self.provider.storage().values.read().expect("storage lock");
        out.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for (k, v) in values.iter() {
            put(&mut out, k);
            put(&mut out, v);
        }
        out
    }

    /// Restore a device produced by [`Device::export`].
    pub fn import(bytes: &[u8]) -> Result<Self, E2eeError> {
        let (&version, mut r) = bytes.split_first().ok_or(E2eeError::Malformed)?;
        if version != EXPORT_VERSION {
            return Err(E2eeError::Malformed);
        }
        let identity =
            String::from_utf8(take(&mut r)?.to_vec()).map_err(|_| E2eeError::Malformed)?;
        let public = take(&mut r)?.to_vec();
        let ids = take(&mut r)?;
        if r.len() < 4 {
            return Err(E2eeError::Malformed);
        }
        let n = u32::from_le_bytes(r[..4].try_into().expect("4 bytes")) as usize;
        r = &r[4..];
        let provider = OpenMlsRustCrypto::default();
        {
            let mut values = provider.storage().values.write().expect("storage lock");
            for _ in 0..n {
                let k = take(&mut r)?.to_vec();
                let v = take(&mut r)?.to_vec();
                values.insert(k, v);
            }
        }
        let signer = SignatureKeyPair::read(
            provider.storage(),
            &public,
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(E2eeError::Malformed)?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(identity.as_bytes().to_vec()).into(),
            signature_key: public.into(),
        };
        let mut groups = HashMap::new();
        for chunk in ids.as_chunks::<8>().0 {
            let cid = u64::from_le_bytes(*chunk);
            if let Some(g) = MlsGroup::load(provider.storage(), &group_id(cid)).map_err(mls)? {
                groups.insert(cid, g);
            }
        }
        Ok(Self {
            provider,
            signer,
            credential,
            identity,
            groups,
        })
    }

    /// Fresh single-use KeyPackages (TLS-serialized) for the server's directory.
    pub fn key_packages(&self, n: usize) -> Result<Vec<Vec<u8>>, E2eeError> {
        (0..n)
            .map(|_| {
                let bundle = KeyPackage::builder()
                    .build(
                        CIPHERSUITE,
                        &self.provider,
                        &self.signer,
                        self.credential.clone(),
                    )
                    .map_err(mls)?;
                bundle.key_package().tls_serialize_detached().map_err(mls)
            })
            .collect()
    }

    pub fn create_group(&mut self, channel_id: u64) -> Result<(), E2eeError> {
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .use_ratchet_tree_extension(true)
            .build();
        let group = MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &config,
            group_id(channel_id),
            self.credential.clone(),
        )
        .map_err(mls)?;
        self.groups.insert(channel_id, group);
        Ok(())
    }

    pub fn has_group(&self, channel_id: u64) -> bool {
        self.groups.contains_key(&channel_id)
    }

    /// Add members from their KeyPackages. Returns (commit for existing members, welcome for new ones).
    pub fn add_members(
        &mut self,
        channel_id: u64,
        key_packages: &[Vec<u8>],
    ) -> Result<(Vec<u8>, Vec<u8>), E2eeError> {
        let crypto = self.provider.crypto();
        let kps = key_packages
            .iter()
            .map(|b| {
                let kp = KeyPackageIn::tls_deserialize_exact(b.as_slice())
                    .map_err(|_| E2eeError::Malformed)?;
                kp.validate(crypto, ProtocolVersion::Mls10).map_err(mls)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let group = self
            .groups
            .get_mut(&channel_id)
            .ok_or(E2eeError::UnknownGroup(channel_id))?;
        let (commit, welcome, _info) = group
            .add_members(&self.provider, &self.signer, &kps)
            .map_err(mls)?;
        group.merge_pending_commit(&self.provider).map_err(mls)?;
        Ok((
            commit.to_bytes().map_err(mls)?,
            welcome.to_bytes().map_err(mls)?,
        ))
    }

    /// Join a group from a Welcome. Returns the channel id.
    pub fn join(&mut self, welcome: &[u8]) -> Result<u64, E2eeError> {
        let msg = MlsMessageIn::tls_deserialize_exact(welcome).map_err(|_| E2eeError::Malformed)?;
        let MlsMessageBodyIn::Welcome(welcome) = msg.extract() else {
            return Err(E2eeError::Malformed);
        };
        let staged = StagedWelcome::new_from_welcome(&self.provider, &join_config(), welcome, None)
            .map_err(mls)?;
        let group = staged.into_group(&self.provider).map_err(mls)?;
        let id: [u8; 8] = group
            .group_id()
            .as_slice()
            .try_into()
            .map_err(|_| E2eeError::Malformed)?;
        let channel_id = u64::from_be_bytes(id);
        self.groups.insert(channel_id, group);
        Ok(channel_id)
    }

    pub fn encrypt(&mut self, channel_id: u64, plaintext: &[u8]) -> Result<Vec<u8>, E2eeError> {
        let group = self
            .groups
            .get_mut(&channel_id)
            .ok_or(E2eeError::UnknownGroup(channel_id))?;
        group
            .create_message(&self.provider, &self.signer, plaintext)
            .map_err(mls)?
            .to_bytes()
            .map_err(mls)
    }

    /// Process an application message or a commit for a channel.
    pub fn process(&mut self, channel_id: u64, bytes: &[u8]) -> Result<Incoming, E2eeError> {
        let group = self
            .groups
            .get_mut(&channel_id)
            .ok_or(E2eeError::UnknownGroup(channel_id))?;
        let msg = MlsMessageIn::tls_deserialize_exact(bytes).map_err(|_| E2eeError::Malformed)?;
        let protocol = msg
            .try_into_protocol_message()
            .map_err(|_| E2eeError::Malformed)?;
        let processed = group
            .process_message(&self.provider, protocol)
            .map_err(mls)?;
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(m) => {
                Ok(Incoming::Application(m.into_bytes()))
            }
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                group
                    .merge_staged_commit(&self.provider, *staged)
                    .map_err(mls)?;
                Ok(Incoming::Commit)
            }
            _ => Ok(Incoming::Other),
        }
    }

    /// Identities of the group members (for the UI's "who can read this" list).
    pub fn members(&self, channel_id: u64) -> Vec<String> {
        self.groups
            .get(&channel_id)
            .map(|g| {
                g.members()
                    .map(|m| {
                        String::from_utf8_lossy(m.credential.serialized_content()).into_owned()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_party_then_group_e2ee() {
        let ch = 4242u64;
        let mut alice = Device::new("alice:phone").unwrap();
        let mut bob = Device::new("bob:laptop").unwrap();
        let mut carol = Device::new("carol:web").unwrap();
        let eve = Device::new("eve:evil").unwrap();

        // Bob publishes key packages; Alice claims one and adds him.
        let bob_kps = bob.key_packages(3).unwrap();
        assert_eq!(bob_kps.len(), 3);
        alice.create_group(ch).unwrap();
        let (_commit, welcome) = alice.add_members(ch, &bob_kps[..1]).unwrap();
        assert_eq!(bob.join(&welcome).unwrap(), ch);
        assert_eq!(alice.members(ch), vec!["alice:phone", "bob:laptop"]);

        // Messages both ways; ciphertext is not plaintext.
        let ct = alice.encrypt(ch, "секрет".as_bytes()).unwrap();
        assert!(!ct.windows(6).any(|w| w == "секрет".as_bytes()));
        assert_eq!(
            bob.process(ch, &ct).unwrap(),
            Incoming::Application("секрет".as_bytes().to_vec())
        );
        let ct2 = bob.encrypt(ch, b"ok").unwrap();
        assert_eq!(
            alice.process(ch, &ct2).unwrap(),
            Incoming::Application(b"ok".to_vec())
        );

        // Carol is added later: Bob processes the commit, Carol joins from the welcome.
        let carol_kp = carol.key_packages(1).unwrap();
        let (commit, welcome) = alice.add_members(ch, &carol_kp).unwrap();
        assert_eq!(bob.process(ch, &commit).unwrap(), Incoming::Commit);
        carol.join(&welcome).unwrap();
        let ct3 = alice.encrypt(ch, b"all three").unwrap();
        assert_eq!(
            bob.process(ch, &ct3).unwrap(),
            Incoming::Application(b"all three".to_vec())
        );
        assert_eq!(
            carol.process(ch, &ct3).unwrap(),
            Incoming::Application(b"all three".to_vec())
        );

        // Eve (not a member) has no group and cannot read anything.
        let mut eve = eve;
        assert!(matches!(
            eve.process(ch, &ct3),
            Err(E2eeError::UnknownGroup(_))
        ));
        assert!(
            eve.join(&welcome).is_err(),
            "a welcome for Carol is useless to Eve"
        );
    }

    #[test]
    fn export_import_keeps_keys_and_groups() {
        let ch = 7u64;
        let mut alice = Device::new("alice:a").unwrap();
        let bob = Device::new("bob:b").unwrap();
        // Bob's KeyPackage private keys must survive his restart.
        let kp = bob.key_packages(1).unwrap();
        let mut bob = Device::import(&bob.export()).unwrap();
        alice.create_group(ch).unwrap();
        let (_c, welcome) = alice.add_members(ch, &kp).unwrap();
        bob.join(&welcome).unwrap();
        let ct = alice.encrypt(ch, b"one").unwrap();
        assert_eq!(
            bob.process(ch, &ct).unwrap(),
            Incoming::Application(b"one".to_vec())
        );
        // Both restart: groups and ratchets continue.
        let mut alice = Device::import(&alice.export()).unwrap();
        let mut bob = Device::import(&bob.export()).unwrap();
        assert_eq!(bob.identity(), "bob:b");
        let ct = alice.encrypt(ch, b"two").unwrap();
        assert_eq!(
            bob.process(ch, &ct).unwrap(),
            Incoming::Application(b"two".to_vec())
        );
        let ct = bob.encrypt(ch, b"three").unwrap();
        assert_eq!(
            alice.process(ch, &ct).unwrap(),
            Incoming::Application(b"three".to_vec())
        );
        assert!(Device::import(&[9, 9, 9]).is_err());
    }

    #[test]
    fn rejects_garbage() {
        let mut a = Device::new("a").unwrap();
        a.create_group(1).unwrap();
        assert!(matches!(
            a.add_members(1, &[vec![1, 2, 3]]),
            Err(E2eeError::Malformed)
        ));
        assert!(a.process(1, b"nope").is_err());
        assert!(a.join(b"nope").is_err());
    }
}
