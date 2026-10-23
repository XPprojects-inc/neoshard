//! End-to-end encrypted DMs through a live server: the core does MLS, the
//! server only ever stores ciphertext.
#![cfg(feature = "e2ee")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use neoshard_core::api::{self, command, update};
use neoshard_core::native::{MemoryStore, NativeClient, Store};
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

async fn post(
    addr: SocketAddr,
    path: &str,
    token: Option<&str>,
    body: serde_json::Value,
) -> serde_json::Value {
    let mut r = reqwest::Client::new()
        .post(format!("http://{addr}/api/v1{path}"))
        .json(&body);
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    r.send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap_or(serde_json::Value::Null)
}

fn cmd(kind: command::Kind) -> api::Command {
    api::Command {
        request_id: 0,
        kind: Some(kind),
    }
}

async fn wait<T>(c: &mut NativeClient, mut f: impl FnMut(&update::Kind) -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let batch = tokio::time::timeout_at(deadline, c.updates.recv())
            .await
            .expect("timed out")
            .expect("core stopped");
        for u in batch.updates {
            if let Some(t) = u.kind.as_ref().and_then(&mut f) {
                return t;
            }
        }
    }
}

async fn connect(addr: SocketAddr, token: &str, store: Arc<dyn Store>) -> NativeClient {
    let mut c = NativeClient::spawn("e2ee-test", store);
    c.send(cmd(command::Kind::Connect(api::Connect {
        api_base: format!("http://{addr}"),
        gateway_url: format!("ws://{addr}/gateway"),
        token: token.into(),
    })));
    wait(&mut c, |k| {
        matches!(k, update::Kind::Ready(_)).then_some(())
    })
    .await;
    c
}

fn new_message(k: &update::Kind) -> Option<neoshard_core::proto::v1::Message> {
    match k {
        update::Kind::Event(e) => match &e.kind {
            Some(Kind::MessageCreate(m)) => Some(m.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[sqlx::test(migrations = "../../server/migrations")]
async fn encrypted_dm_round_trip(pool: sqlx::PgPool) {
    let addr = server(pool.clone()).await;
    let a = post(
        addr,
        "/auth/register",
        None,
        json!({ "username": "alice", "password": "correct horse battery" }),
    )
    .await;
    let b = post(
        addr,
        "/auth/register",
        None,
        json!({ "username": "bob", "password": "correct horse battery" }),
    )
    .await;
    let (alice_tok, bob_tok) = (
        a["token"].as_str().unwrap().to_string(),
        b["token"].as_str().unwrap().to_string(),
    );
    let bob_id = b["user"]["id"].as_str().unwrap().to_string();
    // DMs require a shared guild.
    let g = post(addr, "/guilds", Some(&alice_tok), json!({ "name": "g" })).await;
    let text = g["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "CHANNEL_TYPE_TEXT")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let inv = post(
        addr,
        &format!("/channels/{text}/invites"),
        Some(&alice_tok),
        json!({}),
    )
    .await;
    post(
        addr,
        &format!("/invites/{}", inv["code"].as_str().unwrap()),
        Some(&bob_tok),
        json!({}),
    )
    .await;

    let bob_store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let mut alice = connect(addr, &alice_tok, Arc::new(MemoryStore::new())).await;
    let mut bob = connect(addr, &bob_tok, bob_store.clone()).await;

    // Both devices publish KeyPackages right after Ready.
    for _ in 0..50 {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM mls_key_packages")
            .fetch_one(&pool)
            .await
            .unwrap();
        if n >= 2 * neoshard_core::e2ee::KEY_PACKAGES as i64 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Alice opens an E2EE DM through the core.
    alice.send(api::Command {
        request_id: 1,
        kind: Some(command::Kind::Api(api::ApiRequest {
            method: "POST".into(),
            path: "/users/@me/channels".into(),
            body_json: json!({ "recipient_id": bob_id, "e2ee": true }).to_string(),
        })),
    });
    // The Api response and the ChannelCreate event may arrive in either order.
    let (mut dm, mut created) = (0u64, Vec::<(u64, bool)>::new());
    wait(&mut alice, |k| {
        match k {
            update::Kind::Api(r) if r.request_id == 1 => {
                dm = serde_json::from_str::<serde_json::Value>(&r.body_json)
                    .ok()
                    .and_then(|v| v["id"].as_str().and_then(|s| s.parse().ok()))
                    .expect("dm id");
            }
            update::Kind::Event(e) => {
                if let Some(Kind::ChannelCreate(c)) = &e.kind {
                    created.push((c.id, c.e2ee));
                }
            }
            _ => {}
        }
        (dm != 0 && created.contains(&(dm, true))).then_some(())
    })
    .await;
    wait(&mut bob, |k| match k {
        update::Kind::Event(e) => {
            matches!(&e.kind, Some(Kind::ChannelCreate(c)) if c.id == dm).then_some(())
        }
        _ => None,
    })
    .await;

    // The plain DM of the same pair is a different channel; repeating the request returns the same ones.
    let plain = post(
        addr,
        "/users/@me/channels",
        Some(&alice_tok),
        json!({ "recipient_id": bob_id }),
    )
    .await;
    assert_ne!(plain["id"].as_str().unwrap(), dm.to_string());
    assert_ne!(plain["e2ee"], json!(true));
    let again = post(
        addr,
        "/users/@me/channels",
        Some(&alice_tok),
        json!({ "recipient_id": bob_id, "e2ee": true }),
    )
    .await;
    assert_eq!(again["id"].as_str().unwrap(), dm.to_string());

    // Alice → Bob.
    alice.send(cmd(command::Kind::SendMessage(api::SendMessage {
        channel_id: dm,
        content: "секретное сообщение".into(),
        reply_to_id: 0,
        attachment_ids: vec![],
        sticker_id: 0,
    })));
    let sent = wait(&mut alice, |k| {
        if let update::Kind::PendingSent(s) = k {
            s.message.clone()
        } else {
            None
        }
    })
    .await;
    assert_eq!(
        sent.content, "секретное сообщение",
        "own message shown in plaintext"
    );
    let got = wait(&mut bob, new_message).await;
    assert_eq!(got.content, "секретное сообщение");

    // The server stored only ciphertext.
    let rows: Vec<(String, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT content, e2ee_payload FROM messages WHERE channel_id = $1")
            .bind(dm as i64)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "");
    let ct = rows[0].1.clone().unwrap();
    assert!(ct.len() > 32);
    assert!(!ct.windows("секрет".len()).any(|w| w == "секрет".as_bytes()));

    // Bob → Alice.
    bob.send(cmd(command::Kind::SendMessage(api::SendMessage {
        channel_id: dm,
        content: "получено".into(),
        reply_to_id: 0,
        attachment_ids: vec![],
        sticker_id: 0,
    })));
    let reply = wait(&mut alice, |k| {
        new_message(k).filter(|m| m.content == "получено")
    })
    .await;
    assert_eq!(reply.content, "получено");

    // Bob restarts from his persisted snapshot (keys + group state) and keeps reading.
    tokio::time::sleep(Duration::from_millis(11_000)).await; // let the periodic persist run
    drop(bob);
    let mut bob2 = NativeClient::spawn("e2ee-test", bob_store.clone());
    wait(&mut bob2, |k| {
        matches!(k, update::Kind::Connection(s) if s.state == api::ConnectionState::Ready as i32)
            .then_some(())
    })
    .await;
    alice.send(cmd(command::Kind::SendMessage(api::SendMessage {
        channel_id: dm,
        content: "после перезапуска".into(),
        reply_to_id: 0,
        attachment_ids: vec![],
        sticker_id: 0,
    })));
    let after = wait(&mut bob2, |k| {
        new_message(k).filter(|m| {
            m.channel_id == dm && !m.content.is_empty() && m.content != "секретное сообщение"
        })
    })
    .await;
    assert_eq!(after.content, "после перезапуска");
}

#[sqlx::test(migrations = "../../server/migrations")]
async fn recipient_without_keys_fails_cleanly(pool: sqlx::PgPool) {
    let addr = server(pool.clone()).await;
    let a = post(
        addr,
        "/auth/register",
        None,
        json!({ "username": "alice", "password": "correct horse battery" }),
    )
    .await;
    let b = post(
        addr,
        "/auth/register",
        None,
        json!({ "username": "bob", "password": "correct horse battery" }),
    )
    .await;
    let alice_tok = a["token"].as_str().unwrap().to_string();
    let bob_tok = b["token"].as_str().unwrap().to_string();
    let g = post(addr, "/guilds", Some(&alice_tok), json!({ "name": "g" })).await;
    let text = g["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "CHANNEL_TYPE_TEXT")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let inv = post(
        addr,
        &format!("/channels/{text}/invites"),
        Some(&alice_tok),
        json!({}),
    )
    .await;
    post(
        addr,
        &format!("/invites/{}", inv["code"].as_str().unwrap()),
        Some(&bob_tok),
        json!({}),
    )
    .await;
    // Bob never logged in with a NeoShard client: no KeyPackages.
    let mut alice = connect(addr, &alice_tok, Arc::new(MemoryStore::new())).await;
    let dm = post(
        addr,
        "/users/@me/channels",
        Some(&alice_tok),
        json!({ "recipient_id": b["user"]["id"], "e2ee": true }),
    )
    .await;
    let dm: u64 = dm["id"].as_str().unwrap().parse().unwrap();
    wait(&mut alice, |k| match k {
        update::Kind::Event(e) => {
            matches!(&e.kind, Some(Kind::ChannelCreate(c)) if c.id == dm).then_some(())
        }
        _ => None,
    })
    .await;
    alice.send(cmd(command::Kind::SendMessage(api::SendMessage {
        channel_id: dm,
        content: "x".into(),
        reply_to_id: 0,
        attachment_ids: vec![],
        sticker_id: 0,
    })));
    let failed = wait(&mut alice, |k| {
        if let update::Kind::PendingFailed(f) = k {
            Some(f.clone())
        } else {
            None
        }
    })
    .await;
    assert!(failed.reason.contains("ключей"), "{}", failed.reason);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE channel_id = $1")
        .bind(dm as i64)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "nothing is sent in plaintext");
}
