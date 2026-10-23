//! Calls in DMs: joining a DM's voice room rings the other participants
//! (VoiceStateUpdate on their user topic); a block stops 1:1 calls.

mod common;

use std::time::Duration;

use common::{id, spawn};
use neoshard_proto::v1::{self, client_request::Kind as R, event::Kind};
use serde_json::json;

#[sqlx::test(migrations = "../migrations")]
async fn dm_call_rings_the_peer_and_blocks_stop_it(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (alice, alice_id) = s.register("alice").await;
    let (bob, bob_id) = s.register("bob").await;
    // Friends, so they may DM without a shared server.
    s.post(
        "/users/@me/relationships",
        &alice,
        json!({ "username": "bob" }),
    )
    .await;
    let (st, _) = s
        .put(
            &format!("/users/@me/relationships/{alice_id}"),
            &bob,
            json!({ "type": 1 }),
        )
        .await;
    assert_eq!(st, 200);
    let (st, dm) = s
        .post(
            "/users/@me/channels",
            &alice,
            json!({ "recipient_id": bob_id }),
        )
        .await;
    assert_eq!(st, 200, "{dm}");
    let dm_id = id(&dm["id"]);

    let mut ga = s.gateway(&alice).await;
    let gb = s.gateway(&bob).await;

    // Bob calls: Alice hears about it although the DM has no guild.
    gb.request(R::VoiceState(v1::VoiceStateSet {
        guild_id: 0,
        channel_id: dm_id,
        ..Default::default()
    }))
    .await;
    let vs = ga
        .wait(|k| match k {
            Kind::VoiceStateUpdate(v) => Some(*v),
            _ => None,
        })
        .await;
    assert_eq!(
        (vs.guild_id, vs.channel_id, vs.user_id.to_string()),
        (0, dm_id, bob_id.clone())
    );

    // Hanging up is announced too.
    gb.request(R::VoiceState(v1::VoiceStateSet::default()))
        .await;
    let left = ga
        .wait(|k| match k {
            Kind::VoiceStateUpdate(v) => Some(*v),
            _ => None,
        })
        .await;
    assert_eq!(left.channel_id, 0);

    // After a block neither the token nor the ring goes through.
    let (st, _) = s
        .put(
            &format!("/users/@me/relationships/{bob_id}"),
            &alice,
            json!({ "type": 2 }),
        )
        .await;
    assert_eq!(st, 200);
    let (st, _) = s
        .post(&format!("/channels/{dm_id}/voice"), &bob, json!({}))
        .await;
    assert_eq!(st, 403);
    gb.request(R::VoiceState(v1::VoiceStateSet {
        guild_id: 0,
        channel_id: dm_id,
        ..Default::default()
    }))
    .await;
    let rang = ga
        .try_wait(Duration::from_secs(2), &mut |k| match k {
            Kind::VoiceStateUpdate(v) if v.channel_id != 0 => Some(*v),
            _ => None,
        })
        .await;
    assert!(rang.is_none(), "a blocked user must not ring");
    ga.close().await;
    gb.close().await;
}
