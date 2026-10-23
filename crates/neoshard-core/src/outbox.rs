//! Optimistic sends. A message appears in the UI immediately as pending,
//! is sent with an idempotency `nonce`, retried on network errors and rate
//! limits, and resolved by the HTTP response or the matching Gateway event
//! (whichever comes first). One in-flight send per channel keeps order.

use std::collections::{HashMap, VecDeque};

use neoshard_proto::v1;

pub const MAX_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub nonce: String,
    pub channel_id: u64,
    pub content: String,
    pub reply_to_id: u64,
    /// Base64 MLS ciphertext for E2EE channels (content stays local).
    pub e2ee_payload: Option<String>,
    /// Uploaded attachments (ids from POST /channels/{id}/attachments).
    pub attachment_ids: Vec<u64>,
    /// Guild sticker to send (0 = none).
    pub sticker_id: u64,
    pub attempts: u32,
    /// Earliest time (ms) the next attempt may start.
    pub not_before: u64,
    pub in_flight: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutboxEvent {
    /// Start an HTTP send for this nonce now.
    Send(Pending),
    Sent {
        nonce: String,
        message: Box<v1::Message>,
    },
    Failed {
        nonce: String,
        channel_id: u64,
        reason: String,
        retryable: bool,
    },
}

#[derive(Default)]
pub struct Outbox {
    queues: HashMap<u64, VecDeque<Pending>>,
    counter: u64,
}

impl Outbox {
    pub fn new_nonce(&mut self, now: u64) -> String {
        self.counter += 1;
        format!("{now:x}-{:x}", self.counter)
    }

    pub fn enqueue(&mut self, p: Pending) {
        self.queues.entry(p.channel_id).or_default().push_back(p);
    }

    pub fn len(&self) -> usize {
        self.queues.values().map(VecDeque::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn pending(&self) -> impl Iterator<Item = &Pending> {
        self.queues.values().flatten()
    }

    /// Sends that may start now (head of each channel queue).
    pub fn ready(&mut self, now: u64, online: bool) -> Vec<OutboxEvent> {
        if !online {
            return vec![];
        }
        let mut out = vec![];
        for q in self.queues.values_mut() {
            if let Some(head) = q.front_mut()
                && !head.in_flight
                && !head.failed
                && head.not_before <= now
            {
                head.in_flight = true;
                head.attempts += 1;
                out.push(OutboxEvent::Send(head.clone()));
            }
        }
        out
    }

    fn take(&mut self, nonce: &str) -> Option<Pending> {
        for q in self.queues.values_mut() {
            if let Some(i) = q.iter().position(|p| p.nonce == nonce) {
                return q.remove(i);
            }
        }
        None
    }

    fn get_mut(&mut self, nonce: &str) -> Option<&mut Pending> {
        self.queues
            .values_mut()
            .flatten()
            .find(|p| p.nonce == nonce)
    }

    /// HTTP response for a send. `status == 0` means a network error.
    pub fn on_response(
        &mut self,
        nonce: &str,
        status: u16,
        body: &str,
        retry_after_ms: u64,
        now: u64,
    ) -> Vec<OutboxEvent> {
        match status {
            200 | 201 => {
                let message: v1::Message = serde_json::from_str(body).unwrap_or_default();
                match self.take(nonce) {
                    Some(_) => vec![OutboxEvent::Sent {
                        nonce: nonce.into(),
                        message: Box::new(message),
                    }],
                    None => vec![], // already resolved by the gateway event
                }
            }
            0 | 429 | 500..=599 => {
                let Some(p) = self.get_mut(nonce) else {
                    return vec![];
                };
                p.in_flight = false;
                if p.attempts >= MAX_ATTEMPTS {
                    p.failed = true;
                    return vec![OutboxEvent::Failed {
                        nonce: nonce.into(),
                        channel_id: p.channel_id,
                        reason: "network".into(),
                        retryable: true,
                    }];
                }
                let backoff = if status == 429 {
                    retry_after_ms.max(500)
                } else {
                    500u64 << p.attempts.min(5)
                };
                p.not_before = now + backoff;
                vec![]
            }
            _ => {
                let reason = serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|v| v["message"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| format!("http {status}"));
                match self.take(nonce) {
                    Some(p) => vec![OutboxEvent::Failed {
                        nonce: nonce.into(),
                        channel_id: p.channel_id,
                        reason,
                        retryable: false,
                    }],
                    None => vec![],
                }
            }
        }
    }

    /// Gateway `message_create` with our nonce: resolves the pending send.
    pub fn on_gateway_message(&mut self, m: &v1::Message, me: u64) -> Option<OutboxEvent> {
        if m.nonce.is_empty() || m.author.as_ref().map(|a| a.id) != Some(me) {
            return None;
        }
        self.take(&m.nonce).map(|_| OutboxEvent::Sent {
            nonce: m.nonce.clone(),
            message: Box::new(m.clone()),
        })
    }

    /// Manual retry of a failed message.
    pub fn retry(&mut self, nonce: &str) -> bool {
        match self.get_mut(nonce) {
            Some(p) if p.failed => {
                p.failed = false;
                p.attempts = 0;
                p.not_before = 0;
                true
            }
            _ => false,
        }
    }

    /// Drop a failed message (user discarded it).
    pub fn discard(&mut self, nonce: &str) -> bool {
        self.take(nonce).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(nonce: &str, ch: u64) -> Pending {
        Pending {
            nonce: nonce.into(),
            channel_id: ch,
            content: "x".into(),
            reply_to_id: 0,
            e2ee_payload: None,
            attachment_ids: vec![],
            sticker_id: 0,
            attempts: 0,
            not_before: 0,
            in_flight: false,
            failed: false,
        }
    }

    #[test]
    fn one_in_flight_per_channel_in_order() {
        let mut o = Outbox::default();
        o.enqueue(p("a", 1));
        o.enqueue(p("b", 1));
        o.enqueue(p("c", 2));
        let r = o.ready(0, true);
        assert_eq!(r.len(), 2);
        assert!(o.ready(0, true).is_empty(), "heads are in flight");
        let ev = o.on_response("a", 201, r#"{"id":"5","nonce":"a"}"#, 0, 1);
        assert!(
            matches!(&ev[0], OutboxEvent::Sent { nonce, message } if nonce == "a" && message.id == 5)
        );
        match &o.ready(1, true)[0] {
            OutboxEvent::Send(x) => assert_eq!(x.nonce, "b"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn retries_with_backoff_then_fails() {
        let mut o = Outbox::default();
        o.enqueue(p("a", 1));
        let mut now = 0;
        for attempt in 1..=MAX_ATTEMPTS {
            let r = o.ready(now, true);
            assert_eq!(r.len(), 1, "attempt {attempt}");
            let ev = o.on_response("a", 0, "", 0, now);
            if attempt == MAX_ATTEMPTS {
                assert!(matches!(
                    &ev[0],
                    OutboxEvent::Failed {
                        retryable: true,
                        ..
                    }
                ));
            } else {
                assert!(ev.is_empty());
                assert!(o.ready(now, true).is_empty(), "backoff");
                now += 60_000;
            }
        }
        assert!(o.retry("a"));
        assert_eq!(o.ready(now, true).len(), 1);
    }

    #[test]
    fn rate_limit_respects_retry_after() {
        let mut o = Outbox::default();
        o.enqueue(p("a", 1));
        o.ready(0, true);
        o.on_response("a", 429, "", 3_000, 0);
        assert!(o.ready(2_999, true).is_empty());
        assert_eq!(o.ready(3_000, true).len(), 1);
    }

    #[test]
    fn client_errors_fail_without_retry() {
        let mut o = Outbox::default();
        o.enqueue(p("a", 1));
        o.ready(0, true);
        let ev = o.on_response(
            "a",
            403,
            r#"{"code":"missing_permissions","message":"missing permissions"}"#,
            0,
            0,
        );
        assert_eq!(
            ev,
            vec![OutboxEvent::Failed {
                nonce: "a".into(),
                channel_id: 1,
                reason: "missing permissions".into(),
                retryable: false
            }]
        );
        assert!(o.is_empty());
    }

    #[test]
    fn gateway_event_resolves_first() {
        let mut o = Outbox::default();
        o.enqueue(p("a", 1));
        o.ready(0, true);
        let m = v1::Message {
            id: 9,
            nonce: "a".into(),
            author: Some(v1::User {
                id: 3,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(o.on_gateway_message(&m, 4).is_none(), "not our message");
        assert!(o.on_gateway_message(&m, 3).is_some());
        assert!(
            o.on_response("a", 201, "{}", 0, 0).is_empty(),
            "late HTTP response is ignored"
        );
    }

    #[test]
    fn offline_holds_sends() {
        let mut o = Outbox::default();
        o.enqueue(p("a", 1));
        assert!(o.ready(0, false).is_empty());
        assert_eq!(o.ready(0, true).len(), 1);
    }
}
