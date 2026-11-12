//! Port of `packages/business/__tests__/realtime-stream-publisher.test.ts`
//! (P2.8 spec 4.6, R223-R232). Only the two behaviours that survive the
//! Redis removal are ported (adapted): the raise-only revoke marker (now
//! `DurableKv::raise_number` through `Realtime::revoke_member`) and the
//! buffer retention (512 entries, 5 min). R223-R230 (Redis connection,
//! XADD retry/timeout, XTRIM failure, fire-and-forget XADD) are not
//! applicable (user decision 2026-11-12, DECISIONS P2.8).

mod support;

use std::sync::Arc;

use chatbotx_kv::DurableKv;
use chatbotx_rt::protocol::{Envelope, RevokeReason, member_revoked_key};
use chatbotx_rt::revoke::RevokeArgs;
use chatbotx_rt::{Realtime, RealtimeDeps, TestClock};
use serde_json::json;
use support::{Fakes, migrated_db, test_config, test_realtime};

const NOW_MS: i64 = 1_790_000_000_000;

/// Port of: packages/business/__tests__/realtime-stream-publisher.test.ts:311 "realtime stream publisher Redis connection > advances a revoke marker through an atomic compare-and-set"
#[tokio::test(start_paused = true)]
async fn realtime_stream_publisher_redis_connection_advances_a_revoke_marker_through_an_atomic_compare_and_set()
 {
    let clock = Arc::new(TestClock::new(NOW_MS));
    let durable = DurableKv::new(migrated_db().await, clock.clone());
    let fakes = Fakes::new();
    let rt = Realtime::with_deps(
        test_config("gateway-test-secretgateway-test-secret"),
        RealtimeDeps {
            revoke_store: Arc::new(durable.clone()),
            ..fakes.deps(clock.clone())
        },
    );
    let revoke = |revoked_at_ms| RevokeArgs {
        workspace_id: "ws-1".to_owned(),
        user_id: "user-1".to_owned(),
        reason: RevokeReason::Reauth,
        revoked_at_ms: Some(revoked_at_ms),
    };

    rt.revoke_member(revoke(2000)).await.unwrap();
    rt.revoke_member(revoke(1000)).await.unwrap();

    let key = member_revoked_key("ws-1", "user-1");
    assert_eq!(key, "realtime:revoked:ws-1:user-1");
    assert_eq!(durable.get::<i64>(&key).await.unwrap(), Some(2000));
    assert_eq!(durable.pttl_ms(&key).await.unwrap(), Some(65_000));
}

/// Port of: packages/business/__tests__/realtime-stream-publisher.test.ts:341 "realtime stream publisher Redis connection > appends with the Redis stream retention arguments"
#[tokio::test(start_paused = true)]
async fn realtime_stream_publisher_redis_connection_appends_with_the_redis_stream_retention_arguments()
 {
    // `MAXLEN ~ 10 000` per shard and `MINID ~ now - 300 000` become the
    // workspace buffer bounds: 512 entries and 5 minutes.
    let clock = Arc::new(TestClock::new(NOW_MS));
    let (rt, _fakes) = test_realtime(clock.clone());
    let typing: Envelope =
        serde_json::from_value(json!({ "data": {}, "eventType": "typing" })).unwrap();

    let mut seqs = Vec::new();
    for _ in 0..513 {
        rt.publish_member_event("ws-1", "user-1", typing.clone())
            .unwrap();
        seqs.push(rt.hub().buffered("ws-1").last().unwrap().seq);
    }
    let buffered = rt.hub().buffered("ws-1");
    assert_eq!(buffered.len(), 512);
    assert_eq!(buffered[0].seq, seqs[1]);

    clock.advance(300_001);
    rt.publish_member_event("ws-1", "user-1", typing).unwrap();
    let buffered = rt.hub().buffered("ws-1");
    assert_eq!(buffered.len(), 1);
    assert!(buffered[0].seq > seqs[512]);
}
