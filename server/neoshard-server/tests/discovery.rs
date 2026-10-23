//! Public servers (ADR-0022): found by name and joined without an invite,
//! only when the owner opts in; bans still apply.

mod common;

use common::spawn;
use serde_json::json;

#[sqlx::test(migrations = "../migrations")]
async fn public_guilds_are_found_by_name_and_joined(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, _) = s.register("alice").await;
    let (bob, bob_id) = s.register("bob").await;
    let (gid, _, _) = s.guild(&alice, "Мастерская 100%").await;

    // Private by default: not listed, cannot be joined, settings are the owner's.
    let (st, found) = s.get("/discovery/guilds?q=маст", &bob).await;
    assert_eq!(st, 200);
    assert_eq!(found.as_array().unwrap().len(), 0);
    let (st, _) = s
        .post(&format!("/discovery/guilds/{gid}/join"), &bob, json!({}))
        .await;
    assert_eq!(st, 404);
    let (st, _) = s
        .put(
            &format!("/guilds/{gid}/discovery"),
            &bob,
            json!({ "enabled": true }),
        )
        .await;
    assert!(st == 403 || st == 404, "{st}");

    // The owner makes it public.
    let (st, cfg) = s
        .put(
            &format!("/guilds/{gid}/discovery"),
            &alice,
            json!({ "enabled": true, "description": "  Чиним и мастерим  " }),
        )
        .await;
    assert_eq!(st, 200, "{cfg}");
    assert_eq!(cfg["description"], "Чиним и мастерим");
    let (_, cfg) = s.get(&format!("/guilds/{gid}/discovery"), &alice).await;
    assert_eq!(cfg["enabled"], true);

    // Case-insensitive search; % and _ are plain characters.
    let (_, found) = s.get("/discovery/guilds?q=МАСТ", &bob).await;
    let list = found.as_array().unwrap();
    assert_eq!(list.len(), 1, "{found}");
    assert_eq!(list[0]["id"], gid);
    assert_eq!(list[0]["member_count"], 1);
    assert_eq!(list[0]["joined"], false);
    let (_, found) = s.get("/discovery/guilds?q=100%25", &bob).await;
    assert_eq!(found.as_array().unwrap().len(), 1);
    let (_, found) = s.get("/discovery/guilds?q=%25%25", &bob).await;
    assert_eq!(found.as_array().unwrap().len(), 0);

    // Join without an invite.
    let (st, g) = s
        .post(&format!("/discovery/guilds/{gid}/join"), &bob, json!({}))
        .await;
    assert_eq!(st, 200, "{g}");
    assert_eq!(g["id"], gid);
    let (_, found) = s.get("/discovery/guilds?q=маст", &bob).await;
    assert_eq!(found[0]["joined"], true);
    assert_eq!(found[0]["member_count"], 2);

    // A ban keeps them out.
    let (st, _) = s
        .put(&format!("/guilds/{gid}/bans/{bob_id}"), &alice, json!({}))
        .await;
    assert!(st == 200 || st == 204, "{st}");
    let (st, _) = s
        .post(&format!("/discovery/guilds/{gid}/join"), &bob, json!({}))
        .await;
    assert_eq!(st, 403);
}
