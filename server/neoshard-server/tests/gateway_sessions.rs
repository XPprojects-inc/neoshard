//! Reloading a client many times must not lock the user out: sessions waiting
//! for Resume are superseded by a new Identify, live ones are never evicted.

mod common;

use common::{Gw, spawn};
use neoshard_proto::v1::{self, gateway_frame::Kind as F};

#[sqlx::test(migrations = "../migrations")]
async fn reloads_do_not_exhaust_the_session_cap(pool: sqlx::PgPool) {
    let s = spawn(pool).await;
    let (tok, _) = s.register("alice").await;
    // 25 "page reloads": each connects, then drops the socket without resuming.
    for i in 0..25 {
        let gw = s.gateway(&tok).await;
        assert!(!gw.ready.session_id.is_empty(), "reload {i} got Ready");
        gw.close().await;
    }
    // A live tab plus another reload still work.
    let live = s.gateway(&tok).await;
    let again = s.gateway(&tok).await;
    assert!(!again.ready.session_id.is_empty());
    // The live tab was not evicted: it can still ask for the member list.
    live.request(v1::client_request::Kind::PresenceSet(v1::PresenceSet {
        status: v1::Status::Idle as i32,
        ..Default::default()
    }))
    .await;
    let (gw, first) = Gw::open(
        s.addr,
        F::Identify(v1::Identify {
            token: tok.clone(),
            intents: 0,
            properties: None,
            initial_status: 1,
        }),
    )
    .await;
    assert!(gw.is_some(), "identify after many reloads: {first:?}");
}
