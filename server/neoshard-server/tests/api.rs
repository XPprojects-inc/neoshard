//! End-to-end tests against a live node: HTTP API + Gateway over real Postgres.

mod common;

use std::time::Duration;

use common::{Gw, id, spawn};
use neoshard_proto::permissions::Permissions as P;
use neoshard_proto::v1::{self, event::Kind, gateway_frame::Kind as F};
use reqwest::Method;
use serde_json::json;

#[sqlx::test(migrations = "../migrations")]
async fn register_login_and_sessions(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (tok, uid) = s.register("alice").await;
    let (st, me) = s.get("/users/@me", &tok).await;
    assert_eq!(st, 200);
    assert_eq!(me["id"], uid);
    assert_eq!(me["username"], "alice");

    // Duplicate username / email.
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/register",
            None,
            Some(json!({ "username": "alice", "password": "another password" })),
        )
        .await;
    assert_eq!(st, 409);
    // Invalid username and short password.
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/register",
            None,
            Some(json!({ "username": "Alice!", "password": "another password" })),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/register",
            None,
            Some(json!({ "username": "bob", "password": "short" })),
        )
        .await;
    assert_eq!(st, 400);

    // Login by username and by email; wrong password.
    let (st, v) = s
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "login": "alice", "password": "correct horse battery" })),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let tok2 = v["token"].as_str().unwrap().to_string();
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "login": "ALICE@example.ru", "password": "correct horse battery" })),
        )
        .await;
    assert_eq!(st, 200);
    let (st, v) = s
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "login": "alice", "password": "wrong password" })),
        )
        .await;
    assert_eq!(st, 401);
    assert_eq!(v["code"], "invalid_credentials");
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "login": "nobody", "password": "wrong password" })),
        )
        .await;
    assert_eq!(st, 401);

    // Sessions: list, revoke another one, it stops working.
    let (_, list) = s.get("/sessions", &tok).await;
    assert!(list.as_array().unwrap().len() >= 3);
    let other = list
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["current"] == false)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, _) = s.delete(&format!("/sessions/{other}"), &tok).await;
    assert_eq!(st, 204);
    // Logout current.
    let (st, _) = s.post("/auth/logout", &tok2, json!({})).await;
    assert!(st == 204 || st == 401, "{st}");
    let (st, _) = s.get("/users/@me", &tok2).await;
    assert_eq!(st, 401);
    // Unauthenticated.
    let (st, _) = s.call(Method::GET, "/users/@me", None, None).await;
    assert_eq!(st, 401);
}

#[sqlx::test(migrations = "../migrations")]
async fn totp_two_factor(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (tok, _) = s.register("carol").await;
    let (st, setup) = s.post("/auth/mfa/totp/setup", &tok, json!({})).await;
    assert_eq!(st, 200);
    let secret = base32_decode(setup["secret"].as_str().unwrap());
    let code = neoshard_server_common::auth::totp::current_code(&secret);
    let (st, v) = s
        .post(
            "/auth/mfa/totp/enable",
            &tok,
            json!({ "code": "000000", "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(st, 401, "wrong code must fail: {v}");
    let (st, v) = s
        .post(
            "/auth/mfa/totp/enable",
            &tok,
            json!({ "code": code, "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let backup = v["backup_codes"][0].as_str().unwrap().to_string();
    assert_eq!(v["backup_codes"].as_array().unwrap().len(), 10);

    // Login now yields a ticket, not a token.
    let (st, v) = s
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "login": "carol", "password": "correct horse battery" })),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(v["mfa"], true);
    assert!(v.get("token").is_none());
    let ticket = v["ticket"].as_str().unwrap().to_string();
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/mfa/totp",
            None,
            Some(json!({ "ticket": ticket, "code": "123456" })),
        )
        .await;
    assert_eq!(st, 401);
    // Next step's code is accepted (±1 window); the same code cannot be replayed.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let code = neoshard_server_common::auth::totp::code_at(&secret, now + 30);
    let (st, v) = s
        .call(
            Method::POST,
            "/auth/mfa/totp",
            None,
            Some(json!({ "ticket": ticket, "code": code })),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert!(v["user"]["mfa_enabled"].as_bool().unwrap());
    let (_, v2) = s
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "login": "carol", "password": "correct horse battery" })),
        )
        .await;
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/mfa/totp",
            None,
            Some(json!({ "ticket": v2["ticket"], "code": code })),
        )
        .await;
    assert_eq!(st, 401, "a TOTP code is accepted only once");

    // Backup code works exactly once.
    let login = || async {
        let (_, v) = s
            .call(
                Method::POST,
                "/auth/login",
                None,
                Some(json!({ "login": "carol", "password": "correct horse battery" })),
            )
            .await;
        v["ticket"].as_str().unwrap().to_string()
    };
    let t = login().await;
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/mfa/totp",
            None,
            Some(json!({ "ticket": t, "code": backup })),
        )
        .await;
    assert_eq!(st, 200);
    let t = login().await;
    let (st, _) = s
        .call(
            Method::POST,
            "/auth/mfa/totp",
            None,
            Some(json!({ "ticket": t, "code": backup })),
        )
        .await;
    assert_eq!(st, 401);
}

fn base32_decode(s: &str) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits = 0u64;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes().filter(|c| *c != b'=') {
        bits = (bits << 5) | A.iter().position(|x| *x == c).unwrap() as u64;
        n += 5;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    out
}

#[sqlx::test(migrations = "../migrations")]
async fn guild_messages_and_gateway(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, alice_id) = s.register("alice").await;
    let (bob, _) = s.register("bob").await;
    let (gid, text, _voice) = s.guild(&alice, "Тестовый сервер").await;

    let mut ga = s.gateway(&alice).await;
    assert_eq!(ga.ready.guilds.len(), 1);
    assert_eq!(ga.ready.user.as_ref().unwrap().username, "alice");
    assert_eq!(ga.ready.guilds[0].channels.len(), 3);

    s.join(&alice, &text, &bob).await;
    // Alice sees Bob join.
    let joined = ga
        .wait(|k| match k {
            Kind::MemberAdd(m) => m.user.clone(),
            _ => None,
        })
        .await;
    assert_eq!(joined.username, "bob");

    let mut gb = s.gateway(&bob).await;
    assert_eq!(gb.ready.guilds.len(), 1);

    // Send with a nonce; retry is idempotent.
    let (st, m1) = s
        .post(
            &format!("/channels/{text}/messages"),
            &bob,
            json!({ "content": format!("привет, <@{alice_id}>!"), "nonce": "n-1" }),
        )
        .await;
    assert_eq!(st, 201, "{m1}");
    let (st, m1b) = s
        .post(
            &format!("/channels/{text}/messages"),
            &bob,
            json!({ "content": format!("привет, <@{alice_id}>!"), "nonce": "n-1" }),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(m1["id"], m1b["id"]);

    let got = ga
        .wait(|k| match k {
            Kind::MessageCreate(m) => Some(m.clone()),
            _ => None,
        })
        .await;
    assert_eq!(got.content, format!("привет, <@{alice_id}>!"));
    assert_eq!(got.mention_user_ids, vec![alice_id.parse::<u64>().unwrap()]);
    assert_eq!(got.nonce, "n-1");
    // Mention bumps Alice's unread mention counter.
    let rs = ga
        .wait(|k| match k {
            Kind::ReadStateUpdate(r) => Some(*r),
            _ => None,
        })
        .await;
    assert_eq!(rs.mention_count, 1);

    // History, edit, react, pin, delete.
    let mid = m1["id"].as_str().unwrap().to_string();
    let (_, hist) = s
        .get(&format!("/channels/{text}/messages?limit=10"), &alice)
        .await;
    assert_eq!(hist.as_array().unwrap().len(), 1);
    let (st, _) = s
        .patch(
            &format!("/channels/{text}/messages/{mid}"),
            &alice,
            json!({ "content": "hack" }),
        )
        .await;
    assert_eq!(st, 403, "only the author edits");
    let (st, e) = s
        .patch(
            &format!("/channels/{text}/messages/{mid}"),
            &bob,
            json!({ "content": "исправлено" }),
        )
        .await;
    assert_eq!(st, 200);
    assert!(e["edited_at"].as_str().is_some() || e["edited_at"].as_i64().is_some());
    let up = ga
        .wait(|k| match k {
            Kind::MessageUpdate(m) => Some(m.content.clone()),
            _ => None,
        })
        .await;
    assert_eq!(up, "исправлено");

    let (st, _) = s
        .put(
            &format!("/channels/{text}/messages/{mid}/reactions/%F0%9F%91%8D/@me"),
            &alice,
            json!({}),
        )
        .await;
    assert_eq!(st, 204);
    let r = gb
        .wait(|k| match k {
            Kind::ReactionAdd(r) => Some(r.emoji.clone()),
            _ => None,
        })
        .await;
    assert_eq!(r, "👍");
    let (_, one) = s
        .get(&format!("/channels/{text}/messages/{mid}"), &bob)
        .await;
    assert_eq!(one["reactions"][0]["count"], 1);
    assert!(one["reactions"][0].get("me").is_none(), "bob did not react");

    let (st, _) = s
        .put(&format!("/channels/{text}/pins/{mid}"), &bob, json!({}))
        .await;
    assert_eq!(st, 403, "bob cannot manage messages");
    let (st, _) = s
        .put(&format!("/channels/{text}/pins/{mid}"), &alice, json!({}))
        .await;
    assert_eq!(st, 204);
    let (_, pins) = s.get(&format!("/channels/{text}/pins"), &bob).await;
    assert_eq!(pins.as_array().unwrap().len(), 1);

    // Search.
    let (st, res) = s
        .get(&format!("/guilds/{gid}/messages/search?q=исправлено"), &bob)
        .await;
    assert_eq!(st, 200);
    assert_eq!(res["messages"].as_array().unwrap().len(), 1);

    // Typing via gateway reaches the other user.
    gb.request(v1::client_request::Kind::Typing(v1::TypingSend {
        channel_id: text.parse().unwrap(),
    }))
    .await;
    let t = ga
        .wait(|k| match k {
            Kind::TypingStart(t) => Some(*t),
            _ => None,
        })
        .await;
    assert_eq!(t.channel_id, text.parse::<u64>().unwrap());

    let (st, _) = s
        .delete(&format!("/channels/{text}/messages/{mid}"), &alice)
        .await;
    assert_eq!(st, 204, "owner can delete");
    let del = gb
        .wait(|k| match k {
            Kind::MessageDelete(d) => Some(d.id),
            _ => None,
        })
        .await;
    assert_eq!(del.to_string(), mid);

    // Presence: Bob goes DND, Alice sees it.
    gb.request(v1::client_request::Kind::PresenceSet(v1::PresenceSet {
        status: v1::Status::Dnd as i32,
        custom_status: "занят".into(),
        ..Default::default()
    }))
    .await;
    let p = ga
        .wait(|k| match k {
            Kind::PresenceUpdate(p) if p.status == v1::Status::Dnd as i32 => Some(p.clone()),
            _ => None,
        })
        .await;
    assert_eq!(p.custom_status, "занят");

    // Lazy member list.
    ga.request(v1::client_request::Kind::MemberList(
        v1::MemberListRequest {
            guild_id: gid.parse().unwrap(),
            start: 0,
            end: 100,
        },
    ))
    .await;
    let chunk = ga
        .wait(|k| match k {
            Kind::MemberListChunk(c) => Some(c.clone()),
            _ => None,
        })
        .await;
    assert_eq!(chunk.total, 2);
    assert_eq!(chunk.members.len(), 2);

    ga.close().await;
    gb.close().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn channel_overwrites_hide_channels_and_events(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (bob, _) = s.register("bob").await;
    let (gid, text, _) = s.guild(&alice, "g").await;
    s.join(&alice, &text, &bob).await;
    let (st, secret) = s
        .post(
            &format!("/guilds/{gid}/channels"),
            &alice,
            json!({ "name": "Секрет", "type": 1 }),
        )
        .await;
    assert_eq!(st, 201);
    assert_eq!(secret["name"], "секрет");
    let sid = secret["id"].as_str().unwrap().to_string();

    let mut gb = s.gateway(&bob).await;
    assert!(
        gb.ready.guilds[0]
            .channels
            .iter()
            .any(|c| c.id.to_string() == sid)
    );

    // Deny VIEW_CHANNEL for @everyone.
    let (st, v) = s
        .put(
            &format!("/channels/{sid}/overwrites/{gid}"),
            &alice,
            json!({ "type": 0, "allow": "0", "deny": P::VIEW_CHANNEL.bits().to_string() }),
        )
        .await;
    assert_eq!(st, 204, "{v}");
    let gone = gb
        .wait(|k| match k {
            Kind::ChannelDelete(d) => Some(d.id),
            _ => None,
        })
        .await;
    assert_eq!(gone.to_string(), sid);

    let (st, _) = s.get(&format!("/channels/{sid}/messages"), &bob).await;
    assert_eq!(st, 404, "hidden channels do not exist for bob");
    let (st, _) = s
        .post(
            &format!("/channels/{sid}/messages"),
            &alice,
            json!({ "content": "только для меня" }),
        )
        .await;
    assert_eq!(st, 201);
    let leaked = gb
        .try_wait(Duration::from_millis(700), &mut |k| match k {
            Kind::MessageCreate(m) => Some(m.content.clone()),
            _ => None,
        })
        .await;
    assert!(
        leaked.is_none(),
        "bob must not receive messages from hidden channels"
    );

    // A role with an allow overwrite brings it back.
    let (st, role) = s
        .post(
            &format!("/guilds/{gid}/roles"),
            &alice,
            json!({ "name": "агент" }),
        )
        .await;
    assert_eq!(st, 201);
    let rid = role["id"].as_str().unwrap().to_string();
    let (st, _) = s
        .put(
            &format!("/channels/{sid}/overwrites/{rid}"),
            &alice,
            json!({ "type": 0, "allow": P::VIEW_CHANNEL.bits().to_string(), "deny": "0" }),
        )
        .await;
    assert_eq!(st, 204);
    let bob_id = gb.ready.user.as_ref().unwrap().id;
    let (st, v) = s
        .put(
            &format!("/guilds/{gid}/members/{bob_id}/roles/{rid}"),
            &alice,
            json!({}),
        )
        .await;
    assert_eq!(st, 204, "{v}");
    let back = gb
        .wait(|k| match k {
            Kind::ChannelCreate(c) => Some(c.id),
            _ => None,
        })
        .await;
    assert_eq!(back.to_string(), sid);
    let (st, _) = s.get(&format!("/channels/{sid}/messages"), &bob).await;
    assert_eq!(st, 200);
    gb.close().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn roles_hierarchy_and_moderation(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (owner, owner_id) = s.register("owner").await;
    let (moder, mod_id) = s.register("moder").await;
    let (user, user_id) = s.register("user1").await;
    let (gid, text, _) = s.guild(&owner, "g").await;
    s.join(&owner, &text, &moder).await;
    s.join(&owner, &text, &user).await;

    let mod_perms = (P::KICK_MEMBERS
        | P::BAN_MEMBERS
        | P::MODERATE_MEMBERS
        | P::MANAGE_ROLES
        | P::VIEW_AUDIT_LOG)
        .bits();
    let (_, role) = s
        .post(
            &format!("/guilds/{gid}/roles"),
            &owner,
            json!({ "name": "мод", "permissions": mod_perms.to_string() }),
        )
        .await;
    let rid = role["id"].as_str().unwrap().to_string();
    let (st, _) = s
        .put(
            &format!("/guilds/{gid}/members/{mod_id}/roles/{rid}"),
            &owner,
            json!({}),
        )
        .await;
    assert_eq!(st, 204);

    // A regular member cannot kick.
    let (st, v) = s
        .delete(&format!("/guilds/{gid}/members/{mod_id}"), &user)
        .await;
    assert_eq!(st, 403);
    assert_eq!(v["code"], "missing_permissions");
    // The moderator cannot touch the owner, nor grant ADMINISTRATOR, nor edit their own role.
    let (st, _) = s
        .put(&format!("/guilds/{gid}/bans/{owner_id}"), &moder, json!({}))
        .await;
    assert_eq!(st, 403);
    let (st, _) = s
        .post(
            &format!("/guilds/{gid}/roles"),
            &moder,
            json!({ "name": "x", "permissions": P::ADMINISTRATOR.bits().to_string() }),
        )
        .await;
    assert_eq!(st, 403);
    let (st, _) = s
        .patch(
            &format!("/guilds/{gid}/roles/{rid}"),
            &moder,
            json!({ "name": "супермод" }),
        )
        .await;
    assert_eq!(st, 403, "cannot edit own top role");

    // Timeout blocks sending.
    let until = chrono_ms() + 60_000;
    let (st, v) = s
        .patch(
            &format!("/guilds/{gid}/members/{user_id}"),
            &moder,
            json!({ "timeout_until": until, "reason": "флуд" }),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let (st, _) = s
        .post(
            &format!("/channels/{text}/messages"),
            &user,
            json!({ "content": "можно?" }),
        )
        .await;
    assert_eq!(st, 403);
    let (st, _) = s
        .patch(
            &format!("/guilds/{gid}/members/{user_id}"),
            &moder,
            json!({ "timeout_until": 0 }),
        )
        .await;
    assert_eq!(st, 200);
    let (st, _) = s
        .post(
            &format!("/channels/{text}/messages"),
            &user,
            json!({ "content": "теперь можно" }),
        )
        .await;
    assert_eq!(st, 201);

    // Ban removes and blocks re-joining via invite.
    let (st, _) = s
        .put(
            &format!("/guilds/{gid}/bans/{user_id}"),
            &moder,
            json!({ "reason": "спам", "delete_message_seconds": 3600 }),
        )
        .await;
    assert_eq!(st, 204);
    let (_, msgs) = s.get(&format!("/channels/{text}/messages"), &owner).await;
    assert!(
        msgs.as_array().unwrap().is_empty(),
        "recent messages of the banned user are deleted"
    );
    let (_, inv) = s
        .post(&format!("/channels/{text}/invites"), &owner, json!({}))
        .await;
    let (st, _) = s
        .post(
            &format!("/invites/{}", inv["code"].as_str().unwrap()),
            &user,
            json!({}),
        )
        .await;
    assert_eq!(st, 403);

    // Audit log has the moderation trail.
    let (st, log) = s.get(&format!("/guilds/{gid}/audit-log"), &moder).await;
    assert_eq!(st, 200);
    let actions: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for a in [
        "member.ban",
        "member.timeout",
        "member.role_add",
        "role.create",
    ] {
        assert!(actions.contains(&a), "missing {a} in {actions:?}");
    }
    let (st, _) = s.get(&format!("/guilds/{gid}/audit-log"), &user).await;
    assert_eq!(st, 404, "banned user is no longer a member");
}

fn chrono_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[sqlx::test(migrations = "../migrations")]
async fn gateway_resume_replays_missed_events(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (_gid, text, _) = s.guild(&alice, "g").await;
    let mut gw = s.gateway(&alice).await;
    let session_id = gw.ready.session_id.clone();
    s.post(
        &format!("/channels/{text}/messages"),
        &alice,
        json!({ "content": "один" }),
    )
    .await;
    gw.wait(|k| match k {
        Kind::MessageCreate(_) => Some(()),
        _ => None,
    })
    .await;
    let seq = *gw.last_seq.lock().await;
    assert!(seq >= 1);
    gw.close().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Missed while disconnected.
    s.post(
        &format!("/channels/{text}/messages"),
        &alice,
        json!({ "content": "два" }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (gw2, first) = Gw::open(
        s.addr,
        F::Resume(v1::Resume {
            token: alice.clone(),
            session_id: session_id.clone(),
            last_seq: seq,
        }),
    )
    .await;
    let mut gw2 = gw2.unwrap_or_else(|| panic!("resume failed: {first:?}"));
    let replayed = gw2
        .wait(|k| match k {
            Kind::MessageCreate(m) => Some(m.content.clone()),
            _ => None,
        })
        .await;
    assert_eq!(replayed, "два");
    let resumed = loop {
        match gw2.frame(Duration::from_secs(3)).await.map(|f| f.kind) {
            Some(Some(F::Resumed(r))) => break r,
            Some(_) => continue,
            None => panic!("no Resumed"),
        }
    };
    assert_eq!(resumed.replayed, 1);

    // Bad session → InvalidSession.
    let (none, f) = Gw::open(
        s.addr,
        F::Resume(v1::Resume {
            token: alice.clone(),
            session_id: "nope".into(),
            last_seq: 0,
        }),
    )
    .await;
    assert!(none.is_none());
    assert!(matches!(
        f.kind,
        Some(F::InvalidSession(v1::InvalidSession { resumable: false }))
    ));
    // Bad token → error frame.
    let (none, f) = Gw::open(
        s.addr,
        F::Identify(v1::Identify {
            token: "ns_bad".into(),
            ..Default::default()
        }),
    )
    .await;
    assert!(none.is_none());
    match f.kind {
        Some(F::Error(e)) => assert_eq!(e.code, v1::GatewayErrorCode::AuthFailed as i32),
        other => panic!("expected auth error, got {other:?}"),
    }
    gw2.close().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn rate_limits_and_slowmode(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (bob, _) = s.register("bob").await;
    let (_gid, text, _) = s.guild(&alice, "g").await;
    s.join(&alice, &text, &bob).await;
    let mut codes = vec![];
    for i in 0..7 {
        let (st, _) = s
            .post(
                &format!("/channels/{text}/messages"),
                &bob,
                json!({ "content": format!("m{i}") }),
            )
            .await;
        codes.push(st);
    }
    assert_eq!(&codes[..5], &[201; 5]);
    assert!(codes[5..].iter().all(|c| *c == 429), "{codes:?}");
    let resp = s
        .http
        .post(s.url(&format!("/channels/{text}/messages")))
        .bearer_auth(&bob)
        .json(&json!({ "content": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 429);
    assert!(resp.headers().get("retry-after").is_some());

    // Slowmode applies to bob, not to the owner.
    let (st, _) = s
        .patch(
            &format!("/channels/{text}"),
            &alice,
            json!({ "slowmode_seconds": 60 }),
        )
        .await;
    assert_eq!(st, 200);
    let (_, c2) = s
        .post(
            &format!("/guilds/{}/channels", _gid),
            &alice,
            json!({ "name": "медленно", "slowmode_seconds": 60 }),
        )
        .await;
    let c2 = c2["id"].as_str().unwrap().to_string();
    let (st, _) = s
        .post(
            &format!("/channels/{c2}/messages"),
            &bob,
            json!({ "content": "1" }),
        )
        .await;
    assert_eq!(st, 201);
    let (st, _) = s
        .post(
            &format!("/channels/{c2}/messages"),
            &bob,
            json!({ "content": "2" }),
        )
        .await;
    assert_eq!(st, 429);
    for _ in 0..2 {
        let (st, _) = s
            .post(
                &format!("/channels/{c2}/messages"),
                &alice,
                json!({ "content": "owner" }),
            )
            .await;
        assert_eq!(st, 201);
    }
}

#[sqlx::test(migrations = "../migrations")]
async fn webhooks_bots_and_slash_commands(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (gid, text, _) = s.guild(&alice, "g").await;

    // Webhook.
    let (st, wh) = s
        .post(
            &format!("/channels/{text}/webhooks"),
            &alice,
            json!({ "name": "CI" }),
        )
        .await;
    assert_eq!(st, 201);
    let (wid, wtok) = (wh["id"].as_str().unwrap(), wh["token"].as_str().unwrap());
    let r = s
        .http
        .post(s.url(&format!("/webhooks/{wid}/{wtok}")))
        .json(&json!({ "content": "сборка зелёная @everyone" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let m: serde_json::Value = r.json().await.unwrap();
    assert_eq!(m["author"]["username"], "CI");
    assert!(m["content"].as_str().unwrap().contains("@\u{200B}everyone"));
    let r = s
        .http
        .post(s.url(&format!("/webhooks/{wid}/wrong")))
        .json(&json!({ "content": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // Bot application.
    let (st, app) = s
        .post("/applications", &alice, json!({ "name": "Кубик" }))
        .await;
    assert_eq!(st, 201, "{app}");
    let aid = app["id"].as_str().unwrap().to_string();
    let bot_token = app["token"].as_str().unwrap().to_string();
    assert!(bot_token.starts_with("nsb_"));
    let (st, _) = s.post(&format!("/guilds/{gid}/bots"), &alice, json!({ "application_id": aid, "permissions": (P::SEND_MESSAGES | P::VIEW_CHANNEL).bits().to_string() })).await;
    assert_eq!(st, 204);
    let (st, _) = s
        .put(
            &format!("/applications/{aid}/commands"),
            &bot_token,
            json!([{ "name": "roll", "description": "Бросить кубик" }]),
        )
        .await;
    assert_eq!(st, 200);
    let (_, cmds) = s.get(&format!("/guilds/{gid}/commands"), &alice).await;
    assert_eq!(cmds[0]["name"], "roll");

    // Bot connects with intents: no MESSAGE_CONTENT.
    let mut bot = Gw::connect(
        s.addr,
        &bot_token,
        (v1::Intent::Guilds as u32) | (v1::Intent::GuildMessages as u32),
    )
    .await;
    assert_eq!(bot.ready.guilds.len(), 1);
    s.post(
        &format!("/channels/{text}/messages"),
        &alice,
        json!({ "content": "секретный текст" }),
    )
    .await;
    let content = bot
        .wait(|k| match k {
            Kind::MessageCreate(m) => Some(m.content.clone()),
            _ => None,
        })
        .await;
    assert_eq!(
        content, "",
        "content is hidden without MESSAGE_CONTENT intent"
    );

    let (st, inv) = s.post("/interactions", &alice, json!({ "application_id": aid, "channel_id": text, "command": "roll", "options": { "sides": 6 } })).await;
    assert_eq!(st, 202, "{inv}");
    let it = bot
        .wait(|k| match k {
            Kind::InteractionCreate(i) => Some(i.clone()),
            _ => None,
        })
        .await;
    assert_eq!(it.command, "roll");
    assert!(it.options_json.contains("sides"));
    let r = s
        .http
        .post(s.url(&format!("/interactions/{}/{}/callback", it.id, it.token)))
        .json(&json!({ "content": "🎲 4" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let r = s
        .http
        .post(s.url(&format!("/interactions/{}/{}/callback", it.id, it.token)))
        .json(&json!({ "content": "again" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "an interaction is answered once");
    let (_, hist) = s.get(&format!("/channels/{text}/messages"), &alice).await;
    assert!(
        hist.as_array()
            .unwrap()
            .iter()
            .any(|m| m["content"] == "🎲 4")
    );
    bot.close().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn dms_threads_automod(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (bob, bob_id) = s.register("bob").await;
    let (eve, _) = s.register("eve").await;
    let (gid, text, _) = s.guild(&alice, "g").await;
    s.join(&alice, &text, &bob).await;

    // DMs need a shared guild.
    let (st, _) = s
        .post(
            "/users/@me/channels",
            &eve,
            json!({ "recipient_id": bob_id }),
        )
        .await;
    assert_eq!(st, 403);
    let (st, dm) = s
        .post(
            "/users/@me/channels",
            &alice,
            json!({ "recipient_id": bob_id }),
        )
        .await;
    assert_eq!(st, 200, "{dm}");
    let (_, dm2) = s
        .post(
            "/users/@me/channels",
            &alice,
            json!({ "recipient_id": bob_id }),
        )
        .await;
    assert_eq!(dm["id"], dm2["id"], "one DM per pair");
    let dm_id = dm["id"].as_str().unwrap().to_string();
    let mut gb = s.gateway(&bob).await;
    s.post(
        &format!("/channels/{dm_id}/messages"),
        &alice,
        json!({ "content": "лично" }),
    )
    .await;
    let m = gb
        .wait(|k| match k {
            Kind::MessageCreate(m) if m.guild_id == 0 => Some(m.content.clone()),
            _ => None,
        })
        .await;
    assert_eq!(m, "лично");
    let (st, _) = s.get(&format!("/channels/{dm_id}/messages"), &eve).await;
    assert_eq!(st, 404);

    // Threads.
    let (_, msg) = s
        .post(
            &format!("/channels/{text}/messages"),
            &bob,
            json!({ "content": "обсудим?" }),
        )
        .await;
    let (st, th) = s
        .post(
            &format!("/channels/{text}/threads"),
            &bob,
            json!({ "name": "обсуждение", "message_id": msg["id"] }),
        )
        .await;
    assert_eq!(st, 201, "{th}");
    let tid = th["id"].as_str().unwrap().to_string();
    let (st, _) = s
        .post(
            &format!("/channels/{tid}/messages"),
            &alice,
            json!({ "content": "в треде" }),
        )
        .await;
    assert_eq!(st, 201);
    let (_, parent_msg) = s
        .get(
            &format!("/channels/{text}/messages/{}", msg["id"].as_str().unwrap()),
            &bob,
        )
        .await;
    assert_eq!(parent_msg["thread_id"], tid);

    // Forum requires posts as threads.
    let (_, forum) = s
        .post(
            &format!("/guilds/{gid}/channels"),
            &alice,
            json!({ "name": "форум", "type": 5 }),
        )
        .await;
    let fid = forum["id"].as_str().unwrap().to_string();
    let (st, _) = s
        .post(
            &format!("/channels/{fid}/messages"),
            &bob,
            json!({ "content": "x" }),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = s
        .post(
            &format!("/channels/{fid}/threads"),
            &bob,
            json!({ "name": "Вопрос", "content": "Как настроить?" }),
        )
        .await;
    assert_eq!(st, 201);

    // Automod keyword rule blocks bob but not the owner.
    let (st, v) = s
        .post(
            &format!("/guilds/{gid}/automod/rules"),
            &alice,
            json!({ "name": "мат", "kind": "keyword", "keywords": ["казино*"], "action": "block" }),
        )
        .await;
    assert_eq!(st, 201, "{v}");
    let (st, v) = s
        .post(
            &format!("/channels/{text}/messages"),
            &bob,
            json!({ "content": "лучшее КАЗИНОрояль" }),
        )
        .await;
    assert_eq!(st, 403, "{v}");
    let (st, _) = s
        .post(
            &format!("/channels/{text}/messages"),
            &alice,
            json!({ "content": "казино" }),
        )
        .await;
    assert_eq!(st, 201);
    gb.close().await;
}

#[sqlx::test(migrations = "../migrations")]
async fn discovery_and_voice_unconfigured(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let wk: serde_json::Value = s
        .http
        .get(format!("http://{}/.well-known/neoshard", s.addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(wk["protocol_version"], 1);
    assert!(wk["gateway"].as_str().unwrap().ends_with("/gateway"));
    let (alice, _) = s.register("alice").await;
    let (_gid, text, voice) = s.guild(&alice, "g").await;
    let (st, _) = s
        .post(&format!("/channels/{voice}/voice"), &alice, json!({}))
        .await;
    assert_eq!(st, 503, "LiveKit not configured");
    let (st, _) = s
        .post(&format!("/channels/{text}/voice"), &alice, json!({}))
        .await;
    assert_eq!(st, 400, "not a voice channel");
    let _ = id(&json!("1"));
}

#[sqlx::test(migrations = "../migrations")]
async fn revoked_sessions_are_disconnected(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (_, v) = s
        .call(
            Method::POST,
            "/auth/login",
            None,
            Some(json!({ "login": "alice", "password": "correct horse battery" })),
        )
        .await;
    let second = v["token"].as_str().unwrap().to_string();
    let mut gw = s.gateway(&second).await;
    // Revoke the second session from the first one.
    let (_, list) = s.get("/sessions", &alice).await;
    let other = list
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["current"] == false)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, _) = s.delete(&format!("/sessions/{other}"), &alice).await;
    assert_eq!(st, 204);
    let err = loop {
        match gw.frame(Duration::from_secs(5)).await {
            Some(v1::GatewayFrame {
                kind: Some(F::Error(e)),
            }) => break e,
            Some(_) => continue,
            None => panic!("gateway was not told about the revocation"),
        }
    };
    assert_eq!(err.code, v1::GatewayErrorCode::AuthFailed as i32);
    assert!(
        gw.frame(Duration::from_secs(3)).await.is_none(),
        "connection is closed"
    );
    // And the token is dead for REST and for a new Identify.
    let (st, _) = s.get("/users/@me", &second).await;
    assert_eq!(st, 401);
    let (none, _) = Gw::open(
        s.addr,
        F::Identify(v1::Identify {
            token: second,
            ..Default::default()
        }),
    )
    .await;
    assert!(none.is_none());
}

#[sqlx::test(migrations = "../migrations")]
async fn reactions_are_scoped_to_their_channel(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (mallory, _) = s.register("mallory").await;
    let (_g1, private, _) = s.guild(&alice, "private").await;
    let (_g2, mine, _) = s.guild(&mallory, "mine").await;
    let (_, m) = s
        .post(
            &format!("/channels/{private}/messages"),
            &alice,
            json!({ "content": "secret" }),
        )
        .await;
    let mid = m["id"].as_str().unwrap();
    s.put(
        &format!("/channels/{private}/messages/{mid}/reactions/%E2%9D%A4/@me"),
        &alice,
        json!({}),
    )
    .await;
    // Mallory addresses the foreign message through her own channel.
    let (st, users) = s
        .get(
            &format!("/channels/{mine}/messages/{mid}/reactions/%E2%9D%A4"),
            &mallory,
        )
        .await;
    assert_eq!(st, 200);
    assert!(
        users.as_array().unwrap().is_empty(),
        "no leak of who reacted: {users}"
    );
    s.delete(
        &format!("/channels/{mine}/messages/{mid}/reactions/%E2%9D%A4"),
        &mallory,
    )
    .await;
    let (_, after) = s
        .get(&format!("/channels/{private}/messages/{mid}"), &alice)
        .await;
    assert_eq!(
        after["reactions"][0]["count"], 1,
        "foreign reactions untouched"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn overwrites_respect_role_hierarchy(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (owner, _) = s.register("owner").await;
    let (moder, mod_id) = s.register("moder").await;
    let (gid, text, _) = s.guild(&owner, "g").await;
    s.join(&owner, &text, &moder).await;
    let (_, high) = s
        .post(
            &format!("/guilds/{gid}/roles"),
            &owner,
            json!({ "name": "high" }),
        )
        .await;
    let mod_perms =
        (P::MANAGE_ROLES | P::MANAGE_CHANNELS | P::SEND_MESSAGES | P::VIEW_CHANNEL).bits();
    let (_, low) = s
        .post(
            &format!("/guilds/{gid}/roles"),
            &owner,
            json!({ "name": "mod", "permissions": mod_perms.to_string() }),
        )
        .await;
    // "mod" was created last → position 1; move "high" above it explicitly.
    let (st, _) = s
        .patch(
            &format!("/guilds/{gid}/roles"),
            &owner,
            json!([{ "id": high["id"], "position": 3 }, { "id": low["id"], "position": 2 }]),
        )
        .await;
    assert_eq!(st, 200);
    s.put(
        &format!(
            "/guilds/{gid}/members/{mod_id}/roles/{}",
            low["id"].as_str().unwrap()
        ),
        &owner,
        json!({}),
    )
    .await;
    let deny = P::SEND_MESSAGES.bits().to_string();
    let (st, _) = s
        .put(
            &format!(
                "/channels/{text}/overwrites/{}",
                high["id"].as_str().unwrap()
            ),
            &moder,
            json!({ "type": 0, "allow": "0", "deny": deny }),
        )
        .await;
    assert_eq!(st, 403, "cannot set overwrites for a higher role");
    let (st, _) = s
        .put(
            &format!("/channels/{text}/overwrites/{gid}"),
            &moder,
            json!({ "type": 0, "allow": "0", "deny": deny }),
        )
        .await;
    assert_eq!(st, 204, "@everyone is below everyone");
    // The owner adds a deny of a bit the moderator lacks; the moderator cannot remove it.
    let ban = P::BAN_MEMBERS.bits().to_string();
    s.put(
        &format!("/channels/{text}/overwrites/{gid}"),
        &owner,
        json!({ "type": 0, "allow": "0", "deny": ban }),
    )
    .await;
    let (st, _) = s
        .delete(&format!("/channels/{text}/overwrites/{gid}"), &moder)
        .await;
    assert_eq!(st, 403);
}
