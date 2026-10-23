//! neoshard-core against a live NeoShard node (real Postgres, HTTP, WebSocket).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use neoshard_core::api::{self, ConnectionState, command, update};
use neoshard_core::native::{MemoryStore, NativeClient};
use neoshard_core::proto::v1::event::Kind;
use neoshard_server_common::bus::LocalBus;
use neoshard_server_common::{Config, Ctx};
use serde_json::json;

async fn server(pool: sqlx::PgPool) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut cfg = Config::for_tests("postgres://unused");
    cfg.public_url = format!("http://{addr}");
    let ctx = Ctx::with_pool(cfg, pool, Arc::new(LocalBus::new())).unwrap();
    let node = neoshard_server::build(ctx);
    tokio::spawn(async move {
        axum::serve(
            listener,
            node.router
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

async fn register(addr: SocketAddr, name: &str) -> (String, serde_json::Value) {
    let v: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/v1/auth/register"))
        .json(&json!({ "username": name, "password": "correct horse battery" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (v["token"].as_str().unwrap().to_string(), v)
}

fn cmd(kind: command::Kind) -> api::Command {
    api::Command {
        request_id: 0,
        kind: Some(kind),
    }
}

async fn wait<T>(c: &mut NativeClient, mut f: impl FnMut(&update::Kind) -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let batch = tokio::time::timeout_at(deadline, c.updates.recv())
            .await
            .expect("timed out waiting for update")
            .expect("core stopped");
        for u in batch.updates {
            if let Some(k) = &u.kind
                && let Some(t) = f(k)
            {
                return t;
            }
        }
    }
}

#[sqlx::test(migrations = "../../server/migrations")]
async fn core_end_to_end(pool: sqlx::PgPool) {
    let addr = server(pool).await;
    let (token, _) = register(addr, "alice").await;
    let http = reqwest::Client::new();
    let g: serde_json::Value = http
        .post(format!("http://{addr}/api/v1/guilds"))
        .bearer_auth(&token)
        .json(&json!({ "name": "Ядро" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text: u64 = g["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "CHANNEL_TYPE_TEXT")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let store = Arc::new(MemoryStore::new());
    let mut c = NativeClient::spawn("neoshard-core-test", store.clone());
    // Discovery via /.well-known/neoshard (no gateway URL given).
    c.send(cmd(command::Kind::Connect(api::Connect {
        api_base: format!("http://{addr}"),
        gateway_url: String::new(),
        token: token.clone(),
    })));
    let ready = wait(&mut c, |k| {
        if let update::Kind::Ready(r) = k {
            Some(r.clone())
        } else {
            None
        }
    })
    .await;
    assert_eq!(ready.user.unwrap().username, "alice");
    assert_eq!(ready.guilds.len(), 1);

    // Optimistic send: pending first, then confirmed with a real id.
    c.send(cmd(command::Kind::SendMessage(api::SendMessage {
        channel_id: text,
        content: "из ядра".into(),
        reply_to_id: 0,
        attachment_ids: vec![],
        sticker_id: 0,
    })));
    let nonce = wait(&mut c, |k| {
        if let update::Kind::PendingMessage(p) = k {
            Some(p.nonce.clone())
        } else {
            None
        }
    })
    .await;
    let sent = wait(&mut c, |k| {
        if let update::Kind::PendingSent(s) = k {
            Some(s.clone())
        } else {
            None
        }
    })
    .await;
    assert_eq!(sent.nonce, nonce);
    let id = sent.message.unwrap().id;
    assert!(id > 0);

    // History comes back through the core API.
    c.send(cmd(command::Kind::LoadHistory(api::LoadHistory {
        channel_id: text,
        before_id: 0,
        limit: 50,
    })));
    let h = wait(&mut c, |k| {
        if let update::Kind::History(h) = k {
            Some(h.clone())
        } else {
            None
        }
    })
    .await;
    assert!(h.messages.iter().any(|m| m.id == id));

    // Permissions computed by the core for the selected channel.
    c.send(cmd(command::Kind::SelectChannel(api::SelectChannel {
        channel_id: text,
    })));
    let perms = wait(&mut c, |k| {
        if let update::Kind::Permissions(p) = k {
            Some(p.permissions)
        } else {
            None
        }
    })
    .await;
    assert!(
        neoshard_core::proto::permissions::Permissions::raw(perms)
            .contains(neoshard_core::proto::permissions::Permissions::ADMINISTRATOR),
        "owner has everything"
    );

    // Network drop: messages sent meanwhile by someone else arrive after resume.
    c.send(cmd(command::Kind::Network(api::SetNetworkOnline {
        online: false,
    })));
    tokio::time::sleep(Duration::from_millis(300)).await;
    http.post(format!("http://{addr}/api/v1/channels/{text}/messages"))
        .bearer_auth(&token)
        .json(&json!({ "content": "пока вы были офлайн" }))
        .send()
        .await
        .unwrap();
    // Queued while offline.
    c.send(cmd(command::Kind::SendMessage(api::SendMessage {
        channel_id: text,
        content: "отправлено офлайн".into(),
        reply_to_id: 0,
        attachment_ids: vec![],
        sticker_id: 0,
    })));
    c.send(cmd(command::Kind::Network(api::SetNetworkOnline {
        online: true,
    })));
    wait(&mut c, |k| match k {
        update::Kind::Connection(s) if s.state == ConnectionState::Resuming as i32 => Some(()),
        _ => None,
    })
    .await;
    let missed = wait(&mut c, |k| match k {
        update::Kind::Event(e) => match &e.kind {
            Some(Kind::MessageCreate(m)) if m.content == "пока вы были офлайн" => {
                Some(m.id)
            }
            _ => None,
        },
        _ => None,
    })
    .await;
    assert!(missed > id);
    let offline_sent = wait(&mut c, |k| {
        if let update::Kind::PendingSent(s) = k {
            Some(s.message.clone().unwrap().content)
        } else {
            None
        }
    })
    .await;
    assert_eq!(offline_sent, "отправлено офлайн");

    // Server-side rejection is reported (empty after trim is rejected locally, so use a too-long message).
    c.send(api::Command {
        request_id: 7,
        kind: Some(command::Kind::SendMessage(api::SendMessage {
            channel_id: 999,
            content: "в никуда".into(),
            reply_to_id: 0,
            attachment_ids: vec![],
            sticker_id: 0,
        })),
    });
    let failed = wait(&mut c, |k| {
        if let update::Kind::PendingFailed(f) = k {
            Some(f.clone())
        } else {
            None
        }
    })
    .await;
    assert!(!failed.retryable);
    assert_eq!(failed.channel_id, 999);
}

#[sqlx::test(migrations = "../../server/migrations")]
async fn core_reports_auth_failure(pool: sqlx::PgPool) {
    let addr = server(pool).await;
    let mut c = NativeClient::spawn("t", Arc::new(MemoryStore::new()));
    c.send(cmd(command::Kind::Connect(api::Connect {
        api_base: format!("http://{addr}"),
        gateway_url: format!("ws://{addr}/gateway"),
        token: "ns_invalid".into(),
    })));
    wait(&mut c, |k| match k {
        update::Kind::Connection(s) if s.state == ConnectionState::AuthFailed as i32 => Some(()),
        _ => None,
    })
    .await;
}
