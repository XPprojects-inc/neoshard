//! E2EE message flow inside the core (feature `e2ee`, ADR-0013).
//!
//! The UI never sees ciphertext or keys: it sends plain `SendMessage` and
//! receives decrypted events. For a channel with `e2ee = true` the core:
//! 1. on the first login creates an MLS device and publishes KeyPackages;
//! 2. before the first send creates the MLS group: claims one KeyPackage per
//!    device of every recipient, adds them, relays the Welcome, and only then
//!    encrypts the queued messages (so the Welcome always arrives first);
//! 3. decrypts incoming `e2ee_payload`s and applies Welcome/Commit messages;
//! 4. keeps the plaintext of its own messages (MLS cannot decrypt one's own
//!    ciphertext) and of decrypted ones (each ciphertext decrypts only once);
//! 5. persists the MLS state in the offline snapshot.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use neoshard_proto::core_api::{self as api, update};
use neoshard_proto::v1::{self, event::Kind};

use super::{Core, Purpose, upd};
use crate::e2ee::{Device, Incoming};
use crate::outbox::Pending;

use crate::e2ee::KEY_PACKAGES;
const PLAIN_CACHE: usize = 5_000;

/// A message waiting for the MLS group: nonce, content, reply, attachment ids.
type Queued = (String, String, u64, Vec<u64>);

#[derive(Default)]
pub struct E2eeState {
    pub device: Option<Device>,
    /// Messages waiting for the group to be created: channel → (nonce, content, reply, files).
    queue: HashMap<u64, Vec<Queued>>,
    setup_in_flight: HashSet<u64>,
    /// Own plaintexts by nonce; decrypted plaintexts by message id.
    sent_plain: HashMap<String, String>,
    plain_by_id: HashMap<u64, String>,
}

impl E2eeState {
    fn remember(&mut self, id: u64, content: &str) {
        if self.plain_by_id.len() > PLAIN_CACHE {
            self.plain_by_id.clear();
        }
        self.plain_by_id.insert(id, content.to_string());
    }
}

impl Core {
    /// After Ready: make sure this installation has an MLS device and published keys.
    pub(super) fn e2ee_after_ready(&mut self) {
        if self.e2ee.device.is_some() {
            return;
        }
        let me = self.cache.me_id();
        if me == 0 {
            return;
        }
        let device_id = format!("{:016x}", self.seed ^ me.rotate_left(17));
        let Ok(device) = Device::new(&format!("{me}:{device_id}")) else {
            return;
        };
        let Ok(kps) = device.key_packages(KEY_PACKAGES) else {
            return;
        };
        self.e2ee.device = Some(device);
        let body = serde_json::json!({
            "device_id": device_id,
            "key_packages": kps.iter().map(|k| B64.encode(k)).collect::<Vec<_>>(),
        });
        self.http(
            Purpose::Fire,
            "POST",
            "/mls/key-packages",
            Some(body.to_string()),
        );
        self.dirty = true;
        self.last_persist = 0; // persist on the next tick: keys must not be lost
    }

    /// SendMessage into an E2EE channel.
    pub(super) fn e2ee_send(
        &mut self,
        channel_id: u64,
        nonce: String,
        content: String,
        reply_to_id: u64,
        attachment_ids: Vec<u64>,
        now: u64,
    ) {
        self.e2ee.sent_plain.insert(nonce.clone(), content.clone());
        let ready = self
            .e2ee
            .device
            .as_ref()
            .is_some_and(|d| d.has_group(channel_id));
        if ready {
            self.e2ee_enqueue(channel_id, nonce, content, reply_to_id, attachment_ids, now);
            return;
        }
        self.e2ee.queue.entry(channel_id).or_default().push((
            nonce,
            content,
            reply_to_id,
            attachment_ids,
        ));
        if self.e2ee.device.is_none() {
            // Keys are created after Ready; the queue is flushed by the setup below.
            self.e2ee_after_ready();
        }
        if self.e2ee.device.is_some() && self.e2ee.setup_in_flight.insert(channel_id) {
            let me = self.cache.me_id();
            let recipients: Vec<u64> = self
                .cache
                .channel(channel_id)
                .map(|c| {
                    c.recipient_ids
                        .iter()
                        .copied()
                        .filter(|u| *u != me)
                        .collect()
                })
                .unwrap_or_default();
            self.e2ee_claim_next(channel_id, recipients, Vec::new());
        }
    }

    fn e2ee_enqueue(
        &mut self,
        channel_id: u64,
        nonce: String,
        content: String,
        reply_to_id: u64,
        attachment_ids: Vec<u64>,
        now: u64,
    ) {
        let payload = match self
            .e2ee
            .device
            .as_mut()
            .map(|d| d.encrypt(channel_id, content.as_bytes()))
        {
            Some(Ok(ct)) => B64.encode(ct),
            _ => {
                self.e2ee_fail(channel_id, &nonce, "не удалось зашифровать сообщение");
                return;
            }
        };
        self.outbox.enqueue(Pending {
            nonce,
            channel_id,
            content,
            reply_to_id,
            e2ee_payload: Some(payload),
            attachment_ids,
            sticker_id: 0,
            attempts: 0,
            not_before: 0,
            in_flight: false,
            failed: false,
        });
        self.dirty = true;
        self.pump_outbox(now);
    }

    fn e2ee_fail(&mut self, channel_id: u64, nonce: &str, reason: &str) {
        self.updates.push(upd(update::Kind::PendingFailed(
            api::PendingMessageFailed {
                nonce: nonce.to_string(),
                channel_id,
                reason: reason.to_string(),
                retryable: false,
            },
        )));
    }

    fn e2ee_fail_queue(&mut self, channel_id: u64, reason: &str) {
        self.e2ee.setup_in_flight.remove(&channel_id);
        for (nonce, _, _, _) in self.e2ee.queue.remove(&channel_id).unwrap_or_default() {
            self.e2ee_fail(channel_id, &nonce, reason);
        }
    }

    fn e2ee_claim_next(&mut self, channel_id: u64, mut remaining: Vec<u64>, kps: Vec<Vec<u8>>) {
        match remaining.pop() {
            Some(uid) => self.http(
                Purpose::E2eeClaim {
                    channel_id,
                    remaining,
                    kps,
                },
                "POST",
                &format!("/users/{uid}/mls/key-package"),
                None,
            ),
            None => self.e2ee_create_group(channel_id, kps),
        }
    }

    fn e2ee_create_group(&mut self, channel_id: u64, kps: Vec<Vec<u8>>) {
        let me = self.cache.me_id();
        let Some(device) = self.e2ee.device.as_mut() else {
            return;
        };
        if kps.is_empty() {
            return self.e2ee_fail_queue(channel_id, "у собеседника нет ключей шифрования");
        }
        let result = device
            .create_group(channel_id)
            .and_then(|_| device.add_members(channel_id, &kps));
        let Ok((_commit, welcome)) = result else {
            return self.e2ee_fail_queue(channel_id, "не удалось создать зашифрованную группу");
        };
        let to: Vec<String> = self
            .cache
            .channel(channel_id)
            .map(|c| {
                c.recipient_ids
                    .iter()
                    .filter(|u| **u != me)
                    .map(|u| u.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let body =
            serde_json::json!({ "kind": "welcome", "payload": B64.encode(welcome), "to": to });
        self.http(
            Purpose::E2eeWelcome { channel_id },
            "POST",
            &format!("/channels/{channel_id}/mls"),
            Some(body.to_string()),
        );
        self.dirty = true;
    }

    /// HTTP responses of the setup steps.
    pub(super) fn e2ee_on_http(&mut self, purpose: Purpose, status: u16, body: &str, now: u64) {
        match purpose {
            Purpose::E2eeClaim {
                channel_id,
                remaining,
                mut kps,
            } => {
                if status != 200 {
                    return self.e2ee_fail_queue(
                        channel_id,
                        "у собеседника нет ключей шифрования (он ещё не входил в NeoShard)",
                    );
                }
                let list: Vec<serde_json::Value> = serde_json::from_str(body).unwrap_or_default();
                for item in list {
                    if let Some(kp) = item["key_package"]
                        .as_str()
                        .and_then(|s| B64.decode(s).ok())
                    {
                        kps.push(kp);
                    }
                }
                self.e2ee_claim_next(channel_id, remaining, kps);
            }
            Purpose::E2eeWelcome { channel_id } => {
                self.e2ee.setup_in_flight.remove(&channel_id);
                if !(200..300).contains(&status) {
                    return self.e2ee_fail_queue(channel_id, "не удалось отправить ключи группы");
                }
                // Welcome delivered first: now the queued messages may go out.
                for (nonce, content, reply, files) in
                    self.e2ee.queue.remove(&channel_id).unwrap_or_default()
                {
                    self.e2ee_enqueue(channel_id, nonce, content, reply, files, now);
                }
                self.last_persist = 0;
            }
            _ => {}
        }
    }

    /// Decrypt a message in place (content ← plaintext). Own messages use the
    /// remembered plaintext; anything already decrypted comes from the cache.
    pub(super) fn e2ee_decrypt(&mut self, m: &mut v1::Message) {
        if m.e2ee_payload.is_empty() {
            return;
        }
        if let Some(p) = self.e2ee.plain_by_id.get(&m.id) {
            m.content = p.clone();
            return;
        }
        let mine = m.author.as_ref().map(|a| a.id) == Some(self.cache.me_id());
        if mine {
            if let Some(p) = self.e2ee.sent_plain.get(&m.nonce).cloned() {
                m.content = p;
                self.e2ee.remember(m.id, &m.content);
            }
            return;
        }
        let Some(device) = self.e2ee.device.as_mut() else {
            return;
        };
        if let Ok(Incoming::Application(pt)) = device.process(m.channel_id, &m.e2ee_payload) {
            m.content = String::from_utf8_lossy(&pt).into_owned();
            let (id, content) = (m.id, m.content.clone());
            self.e2ee.remember(id, &content);
            self.dirty = true;
        }
    }

    /// Transform an incoming Gateway event before it reaches the cache and the UI.
    pub(super) fn e2ee_event(&mut self, e: &mut v1::Event) {
        match e.kind.as_mut() {
            Some(Kind::MessageCreate(m) | Kind::MessageUpdate(m)) => self.e2ee_decrypt(m),
            Some(Kind::Mls(ev)) => {
                let Some(device) = self.e2ee.device.as_mut() else {
                    return;
                };
                let ok = match ev.kind.as_str() {
                    "welcome" => device.join(&ev.payload).is_ok(),
                    "commit" | "proposal" => device.process(ev.channel_id, &ev.payload).is_ok(),
                    _ => false,
                };
                if ok {
                    self.dirty = true;
                    self.last_persist = 0;
                }
            }
            _ => {}
        }
    }

    pub(super) fn e2ee_export(&self) -> Vec<u8> {
        self.e2ee
            .device
            .as_ref()
            .map(Device::export)
            .unwrap_or_default()
    }

    pub(super) fn e2ee_import(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.e2ee.device = Device::import(bytes).ok();
        }
    }
}
