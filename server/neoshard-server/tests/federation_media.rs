//! Two live instances federating DMs; voice token issuance with LiveKit configured.

mod common;

use common::spawn_with;
use neoshard_federation::InstanceKey;
use neoshard_proto::v1::event::Kind;
use serde_json::json;
use sqlx::Executor;
use sqlx::postgres::PgPoolOptions;

async fn second_db() -> (sqlx::PgPool, String) {
    let admin_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await
        .unwrap();
    let name = format!("neoshard_fed_{}", rand_suffix());
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let base = admin_url.rsplit_once('/').unwrap().0;
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&format!("{base}/{name}"))
        .await
        .unwrap();
    neoshard_server_common::MIGRATOR.run(&pool).await.unwrap();
    (pool, name)
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
        % 1_000_000_000
}

#[sqlx::test(migrations = "../migrations")]
async fn federated_dm_between_instances(pool: sqlx::PgPool) {
    let (pool_b, db_b) = second_db().await;
    let key_a = InstanceKey::generate().seed_b64();
    let key_b = InstanceKey::generate().seed_b64();
    let a = spawn_with(pool, |c| {
        c.federation_enabled = true;
        c.federation_key = Some(key_a.clone());
    })
    .await;
    let b = spawn_with(pool_b.clone(), |c| {
        c.federation_enabled = true;
        c.federation_key = Some(key_b.clone());
    })
    .await;

    let (alice, _) = a.register("alice").await;
    let (bob, _) = b.register("bob").await;
    let mut gb = b.gateway(&bob).await;

    // Alice resolves bob@<instance B> into a local shadow user and DMs him.
    let addr = format!("bob@{}", b.addr);
    let (st, remote) = a
        .post(&format!("/federation/resolve/{addr}"), &alice, json!({}))
        .await;
    assert_eq!(st, 200, "{remote}");
    assert_eq!(remote["instance"], b.addr.to_string());
    let (st, dm) = a
        .post(
            "/users/@me/channels",
            &alice,
            json!({ "recipient_id": remote["id"] }),
        )
        .await;
    assert_eq!(st, 200, "{dm}");
    let (st, _) = a
        .post(
            &format!("/channels/{}/messages", dm["id"].as_str().unwrap()),
            &alice,
            json!({ "content": "привет через федерацию" }),
        )
        .await;
    assert_eq!(st, 201);

    // Bob receives it on instance B, from the shadow user alice@A.
    let m = gb
        .wait(|k| match k {
            Kind::MessageCreate(m) => Some(m.clone()),
            _ => None,
        })
        .await;
    assert_eq!(m.content, "привет через федерацию");
    let author = m.author.unwrap();
    assert_eq!(author.username, "alice");
    assert_eq!(author.instance, a.addr.to_string());

    // Bob replies; it travels back to A.
    let mut ga = a.gateway(&alice).await;
    let (st, _) = b
        .post(
            &format!("/channels/{}/messages", m.channel_id),
            &bob,
            json!({ "content": "получил!" }),
        )
        .await;
    assert_eq!(st, 201);
    let reply = ga
        .wait(|k| match k {
            Kind::MessageCreate(m) if m.content == "получил!" => Some(m.clone()),
            _ => None,
        })
        .await;
    assert_eq!(reply.author.unwrap().instance, b.addr.to_string());

    // An unsigned / forged inbox request is rejected.
    let r = b.http.post(format!("http://{}/federation/v1/inbox", b.addr)).json(&json!({ "type": "dm.message", "id": "1", "from": "x", "to": "bob", "content": "spoof" })).send().await.unwrap();
    assert_eq!(r.status(), 400);
    let r = b
        .http
        .post(format!("http://{}/federation/v1/inbox", b.addr))
        .header("neoshard-origin", a.addr.to_string())
        .header("neoshard-destination", b.addr.to_string())
        .header("neoshard-date", (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).to_string())
        .header("neoshard-nonce", "forged")
        .header("neoshard-signature", "AAAA")
        .json(&json!({ "type": "dm.message", "id": "1", "from": "alice", "to": "bob", "content": "spoof" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    ga.close().await;
    gb.close().await;
    drop(pool_b);
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let _ = admin
        .execute(format!("DROP DATABASE IF EXISTS {db_b} WITH (FORCE)").as_str())
        .await;
}

#[sqlx::test(migrations = "../migrations")]
async fn voice_tokens_follow_permissions(pool: sqlx::PgPool) {
    let s = spawn_with(pool, |c| {
        c.livekit_url = Some("wss://media.example".into());
        c.livekit_api_key = Some("key".into());
        c.livekit_api_secret = Some("secret-secret-secret-secret-secret".into());
        c.turn_secret = Some("turn-secret".into());
        c.turn_urls = vec!["turns:turn.example:443?transport=tcp".into()];
    })
    .await;
    let (alice, _) = s.register("alice").await;
    let (bob, _) = s.register("bob").await;
    let (gid, text, voice) = s.guild(&alice, "g").await;
    s.join(&alice, &text, &bob).await;

    let (st, v) = s
        .post(&format!("/channels/{voice}/voice"), &bob, json!({}))
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["room"], format!("voice:{voice}"));
    assert!(
        v["grants"]["canPublishSources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "screen_share")
    );
    assert!(v["turn"]["username"].as_str().unwrap().ends_with(&format!(
        ":{}",
        s.get("/users/@me", &bob).await.1["id"].as_str().unwrap()
    )));

    // Deny STREAM for @everyone in the voice channel: no screen share grant.
    let deny = neoshard_proto::permissions::Permissions::STREAM
        .bits()
        .to_string();
    let (st, _) = s
        .put(
            &format!("/channels/{voice}/overwrites/{gid}"),
            &alice,
            json!({ "type": 0, "allow": "0", "deny": deny }),
        )
        .await;
    assert_eq!(st, 204);
    let (_, v) = s
        .post(&format!("/channels/{voice}/voice"), &bob, json!({}))
        .await;
    assert!(
        !v["grants"]["canPublishSources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "screen_share")
    );
    assert!(
        v["grants"]["canPublishSources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "microphone")
    );

    // Voice state over the gateway reaches others.
    let mut ga = s.gateway(&alice).await;
    let gb = s.gateway(&bob).await;
    gb.request(neoshard_proto::v1::client_request::Kind::VoiceState(
        neoshard_proto::v1::VoiceStateSet {
            guild_id: gid.parse().unwrap(),
            channel_id: voice.parse().unwrap(),
            self_mute: true,
            ..Default::default()
        },
    ))
    .await;
    let vs = ga
        .wait(|k| match k {
            Kind::VoiceStateUpdate(v) => Some(*v),
            _ => None,
        })
        .await;
    assert_eq!(vs.channel_id.to_string(), voice);
    assert!(vs.self_mute);
    ga.close().await;
    gb.close().await;
}
