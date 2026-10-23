//! Profile customization end to end (ADR-0020): profile fields and validation,
//! visibility, per-server profiles, server tags, notes, links, local file
//! storage and the persistent custom status.

mod common;

use common::{spawn, spawn_with};
use neoshard_proto::v1::{self, event::Kind};
use serde_json::json;

#[sqlx::test(migrations = "../migrations")]
async fn profile_fields_visibility_and_server_profiles(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, alice_id) = s.register("alice").await;
    let (bob, _) = s.register("bob").await;
    let (carol, _) = s.register("carol").await;
    let (gid, text, _) = s.guild(&alice, "Осколки").await;
    s.join(&alice, &text, &bob).await;
    let mut gb = s.gateway(&bob).await;

    // Validation.
    for bad in [
        json!({ "bio": "я".repeat(191) }),
        json!({ "pronouns": "x".repeat(41) }),
        json!({ "avatar_decoration": "not-in-catalog" }),
        json!({ "theme_colors": [1] }),
        json!({ "theme_colors": [1, 16777216] }),
        json!({ "banner_color": 16777216 }),
        json!({ "name_style": { "font": "comic-sans" } }),
        json!({ "avatar": "avatars/1/2" }),
        json!({ "primary_guild_id": gid }),
    ] {
        let (st, v) = s.patch("/users/@me", &alice, bad.clone()).await;
        assert_eq!(st, 400, "{bad} → {v}");
    }

    // A full profile.
    let (st, me) = s
        .patch(
            "/users/@me",
            &alice,
            json!({
                "display_name": "Алиса",
                "pronouns": "она",
                "bio": "Пишу **NeoShard**",
                "banner_color": 0x2ec4a6,
                "theme_colors": [0x1b2a4a, 0x7b5ea7],
                "avatar_decoration": "orbit",
                "profile_effect": "aurora",
                "nameplate": "lagoon",
                "name_style": { "font": "serif", "effect": "gradient", "colors": [0xff8800, 0x2ec4a6] },
            }),
        )
        .await;
    assert_eq!(st, 200, "{me}");
    assert_eq!(me["avatar_decoration"], "orbit");
    assert_eq!(me["name_style"]["font"], "serif");
    // Others see the compact cosmetics right away.
    let upd = gb
        .wait(|k| match k {
            Kind::UserUpdate(u) if u.avatar_decoration == "orbit" => Some(u.clone()),
            _ => None,
        })
        .await;
    assert_eq!(upd.nameplate, "lagoon");
    assert!(!upd.mfa_enabled);

    // Server tag: needs a tag on the guild and membership.
    let (st, _) = s
        .patch(
            &format!("/guilds/{gid}"),
            &alice,
            json!({ "tag": "TOOLONG" }),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = s
        .patch(
            &format!("/guilds/{gid}"),
            &alice,
            json!({ "tag": "ОСК", "tag_badge": "star" }),
        )
        .await;
    assert_eq!(st, 200);
    let (st, me) = s
        .patch("/users/@me", &alice, json!({ "primary_guild_id": gid }))
        .await;
    assert_eq!(st, 200, "{me}");
    assert_eq!(me["guild_tag"]["tag"], "ОСК");
    assert_eq!(me["guild_tag"]["badge"], "star");
    let (st, _) = s
        .patch("/users/@me", &carol, json!({ "primary_guild_id": gid }))
        .await;
    assert_eq!(st, 400, "carol is not in the guild");

    // Per-server profile: only for yourself.
    let (st, m) = s
        .patch(
            &format!("/guilds/{gid}/members/@me"),
            &alice,
            json!({ "bio": "На этом сервере — модератор", "pronouns": "она/её" }),
        )
        .await;
    assert_eq!(st, 200, "{m}");
    let (st, _) = s
        .patch(
            &format!("/guilds/{gid}/members/{alice_id}"),
            &bob,
            json!({ "bio": "взлом" }),
        )
        .await;
    assert_eq!(st, 403);

    // Bob sees Alice's profile with the server profile and mutual servers.
    let (st, p) = s
        .get(&format!("/users/{alice_id}/profile?guild_id={gid}"), &bob)
        .await;
    assert_eq!(st, 200, "{p}");
    assert_eq!(p["pronouns"], "она");
    assert_eq!(p["bio"], "Пишу **NeoShard**");
    assert_eq!(p["banner_color"], 0x2ec4a6);
    assert_eq!(p["theme_colors"], json!([0x1b2a4a, 0x7b5ea7]));
    assert_eq!(p["profile_effect"], "aurora");
    assert_eq!(p["member"]["bio"], "На этом сервере — модератор");
    assert_eq!(p["mutual_guild_ids"], json!([gid]));
    assert!(p["created_at"].as_str().unwrap().parse::<i64>().unwrap() > 0);

    // Notes are private to their author.
    let (st, _) = s
        .put(
            &format!("/users/{alice_id}/note"),
            &bob,
            json!({ "note": "знакомы по работе" }),
        )
        .await;
    assert_eq!(st, 204);
    let (_, p) = s.get(&format!("/users/{alice_id}/profile"), &bob).await;
    assert_eq!(p["note"], "знакомы по работе");
    let (_, own) = s.get("/users/@me/profile", &alice).await;
    assert!(own["note"].is_null() || own["note"] == "");

    // A stranger gets 404, as for an unknown id.
    let (st, _) = s.get(&format!("/users/{alice_id}/profile"), &carol).await;
    assert_eq!(st, 404);
    let (st, _) = s
        .put(
            &format!("/users/{alice_id}/note"),
            &carol,
            json!({ "note": "x" }),
        )
        .await;
    assert_eq!(st, 404);

    // Links: validated, unverified until a rel="me" backlink is found.
    let (st, _) = s
        .put(
            "/users/@me/links",
            &alice,
            json!([{ "label": "сайт", "url": "javascript:alert(1)" }]),
        )
        .await;
    assert_eq!(st, 400);
    let (st, links) = s
        .put(
            "/users/@me/links",
            &alice,
            json!([{ "label": "сайт", "url": "https://example.org/alice" }]),
        )
        .await;
    assert_eq!(st, 200, "{links}");
    assert!(links[0]["verified"].is_null() || links[0]["verified"] == false);
}

#[sqlx::test(migrations = "../migrations")]
async fn local_file_storage_and_avatars(pool: sqlx::PgPool) {
    let dir = std::env::temp_dir().join(format!("neoshard-files-{}", std::process::id()));
    let s = spawn_with(pool, |c| c.files_dir = Some(dir.clone())).await;
    let (alice, _) = s.register("alice").await;
    let (bob, _) = s.register("bob").await;
    let png: Vec<u8> = [&b"\x89PNG\r\n\x1a\n"[..], &[0u8; 64]].concat();

    let (st, up) = s
        .post(
            "/uploads/avatar",
            &alice,
            json!({ "filename": "a.png", "content_type": "image/png", "size": png.len() }),
        )
        .await;
    assert_eq!(st, 200, "{up}");
    let upload_url = up["upload_url"].as_str().unwrap();
    // A tampered signature or an oversized body is refused.
    let r = s
        .http
        .put(upload_url.replace("sig=", "sig=x"))
        .body(png.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = s
        .http
        .put(upload_url)
        .body([png.clone(), vec![0; 10]].concat())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let r = s
        .http
        .put(upload_url)
        .body(png.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = s
        .http
        .put(upload_url)
        .body(png.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "objects are immutable");

    let file = s
        .http
        .get(up["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(file.status(), 200);
    assert_eq!(file.headers()["content-type"], "image/png");
    assert_eq!(file.headers()["x-content-type-options"], "nosniff");
    assert!(
        file.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
    assert_eq!(file.bytes().await.unwrap().to_vec(), png);
    let r = s
        .http
        .get(format!("http://{}/files/..%2F..%2Fetc%2Fpasswd", s.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // Only your own keys can become your avatar.
    let key = up["key"].as_str().unwrap();
    let (st, _) = s.patch("/users/@me", &bob, json!({ "avatar": key })).await;
    assert_eq!(st, 400);
    let (st, me) = s
        .patch("/users/@me", &alice, json!({ "avatar": key }))
        .await;
    assert_eq!(st, 200);
    assert_eq!(me["avatar"], key);
    // The Ready payload tells clients where keys live.
    let gw = s.gateway(&alice).await;
    assert_eq!(gw.ready.files_base, format!("http://{}/files", s.addr));
    let _ = std::fs::remove_dir_all(dir);
}

#[sqlx::test(migrations = "../migrations")]
async fn custom_status_with_emoji_survives_reconnect(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, alice_id) = s.register("alice").await;
    let (bob, _) = s.register("bob").await;
    let (_, text, _) = s.guild(&alice, "g").await;
    s.join(&alice, &text, &bob).await;
    let mut gb = s.gateway(&bob).await;
    let ga = s.gateway(&alice).await;
    let in_an_hour = chrono::Utc::now().timestamp_millis() + 3_600_000;
    ga.request(v1::client_request::Kind::PresenceSet(v1::PresenceSet {
        status: v1::Status::Online as i32,
        custom_status: "в отпуске".into(),
        custom_emoji: "🏖".into(),
        custom_expires_at: in_an_hour,
    }))
    .await;
    let p = gb
        .wait(|k| match k {
            Kind::PresenceUpdate(p) if p.custom_emoji == "🏖" => Some(p.clone()),
            _ => None,
        })
        .await;
    assert_eq!(p.custom_status, "в отпуске");
    assert_eq!(p.custom_expires_at, in_an_hour);
    // Persisted: a fresh node (empty presence registry) restores it on connect.
    for _ in 0..50 {
        let e: String = sqlx::query_scalar("SELECT custom_status_emoji FROM users WHERE id = $1")
            .bind(alice_id.parse::<i64>().unwrap())
            .fetch_one(&s.ctx.db)
            .await
            .unwrap();
        if e == "🏖" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let s2 = spawn(s.ctx.db.clone()).await;
    let ga2 = s2.gateway(&alice).await;
    // A new device learns its own stored status from Ready (so it does not wipe it).
    let own = ga2
        .ready
        .presences
        .iter()
        .find(|p| p.user_id.to_string() == alice_id)
        .expect("own presence in Ready");
    assert_eq!(own.custom_emoji, "🏖");
    let gb2 = s2.gateway(&bob).await;
    let p = gb2
        .ready
        .presences
        .iter()
        .find(|p| p.user_id.to_string() == alice_id)
        .expect("alice is online on the new node")
        .clone();
    assert_eq!(p.custom_status, "в отпуске");
    assert_eq!(p.custom_emoji, "🏖");
    assert_eq!(p.custom_expires_at, in_an_hour);
}
