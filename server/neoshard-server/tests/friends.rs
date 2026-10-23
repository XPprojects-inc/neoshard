//! Friends by username (ADR-0021): requests, accept, DMs and presence without
//! a shared server, blocks.

mod common;

use common::spawn;
use neoshard_proto::v1::{self, RelationshipKind as K, event::Kind};
use serde_json::json;

#[sqlx::test(migrations = "../migrations")]
async fn friends_without_a_shared_server(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, alice_id) = s.register("alice").await;
    let (bob, bob_id) = s.register("bob").await;
    let (carol, _) = s.register("carol").await;
    let mut gb = s.gateway(&bob).await;

    // Unknown users and yourself.
    let (st, _) = s
        .post(
            "/users/@me/relationships",
            &alice,
            json!({ "username": "nobody" }),
        )
        .await;
    assert_eq!(st, 404);
    let (st, _) = s
        .post(
            "/users/@me/relationships",
            &alice,
            json!({ "username": "alice" }),
        )
        .await;
    assert_eq!(st, 400);

    // No shared server: a DM is refused before they are friends.
    let (st, _) = s
        .post(
            "/users/@me/channels",
            &alice,
            json!({ "recipient_id": bob_id }),
        )
        .await;
    assert_eq!(st, 403);

    // Request by username (with @ and capitals).
    let (st, rel) = s
        .post(
            "/users/@me/relationships",
            &alice,
            json!({ "username": "@Bob" }),
        )
        .await;
    assert_eq!(st, 201, "{rel}");
    assert_eq!(rel["kind"], "RELATIONSHIP_KIND_OUTGOING");
    let incoming = gb
        .wait(|k| match k {
            Kind::RelationshipAdd(r) => Some(r.clone()),
            _ => None,
        })
        .await;
    assert_eq!(incoming.kind, K::Incoming as i32);
    assert_eq!(incoming.user.unwrap().username, "alice");
    let (st, _) = s
        .post(
            "/users/@me/relationships",
            &alice,
            json!({ "username": "bob" }),
        )
        .await;
    assert_eq!(st, 409, "a second request is a conflict");

    // Bob accepts.
    let (st, rel) = s
        .put(
            &format!("/users/@me/relationships/{alice_id}"),
            &bob,
            json!({ "type": 1 }),
        )
        .await;
    assert_eq!(st, 200, "{rel}");
    assert_eq!(rel["kind"], "RELATIONSHIP_KIND_FRIEND");
    let (_, list) = s.get("/users/@me/relationships", &alice).await;
    assert_eq!(list[0]["kind"], "RELATIONSHIP_KIND_FRIEND");

    // Friends can DM and see each other's presence and profile without a shared server.
    let (st, dm) = s
        .post(
            "/users/@me/channels",
            &alice,
            json!({ "recipient_id": bob_id }),
        )
        .await;
    assert_eq!(st, 200, "{dm}");
    let (st, p) = s.get(&format!("/users/{bob_id}/profile"), &alice).await;
    assert_eq!(st, 200);
    assert_eq!(p["relationship"], "RELATIONSHIP_KIND_FRIEND");
    let _ga = s.gateway(&alice).await;
    let online = gb
        .wait(|k| match k {
            Kind::PresenceUpdate(p)
                if p.user_id.to_string() == alice_id && p.status == v1::Status::Online as i32 =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert_eq!(online, ());
    let gb2 = s.gateway(&bob).await;
    assert_eq!(gb2.ready.relationships.len(), 1);
    assert!(
        gb2.ready
            .presences
            .iter()
            .any(|p| p.user_id.to_string() == alice_id),
        "friend presence in Ready"
    );

    // Mutual friends: carol befriends both.
    s.post(
        "/users/@me/relationships",
        &carol,
        json!({ "username": "alice" }),
    )
    .await;
    s.post(
        "/users/@me/relationships",
        &alice,
        json!({ "username": "carol" }),
    )
    .await; // accepts
    s.post(
        "/users/@me/relationships",
        &carol,
        json!({ "username": "bob" }),
    )
    .await;
    s.post(
        "/users/@me/relationships",
        &bob,
        json!({ "username": "carol" }),
    )
    .await;
    let (_, p) = s.get(&format!("/users/{bob_id}/profile"), &alice).await;
    assert_eq!(p["mutual_friend_ids"].as_array().unwrap().len(), 1);

    // Bob blocks Alice: friendship ends for her, DMs stop, requests bounce.
    let (st, _) = s
        .put(
            &format!("/users/@me/relationships/{alice_id}"),
            &bob,
            json!({ "type": 2 }),
        )
        .await;
    assert_eq!(st, 200);
    let (_, list) = s.get("/users/@me/relationships", &alice).await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|r| r["user"]["username"] != "bob")
    );
    let dm_id = dm["id"].as_str().unwrap();
    let (st, _) = s
        .post(
            &format!("/channels/{dm_id}/messages"),
            &alice,
            json!({ "content": "привет?" }),
        )
        .await;
    assert_eq!(st, 403);
    let (st, _) = s
        .post(
            "/users/@me/relationships",
            &alice,
            json!({ "username": "bob" }),
        )
        .await;
    assert_eq!(st, 400, "a block is not revealed, the request just fails");

    // Unblock, then removing a friend clears both sides.
    let (st, _) = s
        .delete(&format!("/users/@me/relationships/{alice_id}"), &bob)
        .await;
    assert_eq!(st, 204);
    let (st, _) = s
        .delete(&format!("/users/@me/relationships/{alice_id}"), &carol)
        .await;
    assert_eq!(st, 204);
    let (_, list) = s.get("/users/@me/relationships", &alice).await;
    assert!(list.as_array().unwrap().is_empty(), "{list}");
}
