//! Port of `apps/realtime/__tests__/gateway.test.ts` (P2.8 spec 4.6,
//! R127-R166). The uWS server on `127.0.0.1:0` with a fake Redis is the
//! axum router on `127.0.0.1:0` with fake collaborators: the Redis `get`
//! of the revoke marker is `FakeRevokeStore`, `reportWorkspacePresence` is
//! `RecordingPresence`. Real time, as the TS cases.
//! Not applicable (user decision 2026-11-12, DECISIONS P2.8): R136-R141
//! (stream reader readiness, Redis activation/replay failures, Redis
//! close) and R162-R164 (malformed records, tainted shards, XREAD
//! recovery).

mod support;

use std::time::Duration;

use support::gateway::{
    CLOSE_ON_OPEN_MAX, SECRET, TestGateway, close_before, close_client, connect,
    expired_guest_token, expired_member_token, gateway_config, guest_token, member_token,
    member_token_with_iat_ms, now_ms, request_http, request_upgrade, start_upgrade, stays_open,
    wait_for_close, wait_for_message,
};
use support::{Deferred, Fakes, IO_WAIT, ReadStep, wait_for};

use chatbotx_rt::protocol::Envelope;
use serde_json::json;

async fn returns_401_for(request_path: String) {
    let gateway = TestGateway::start(gateway_config(), Fakes::new()).await;

    let status = request_upgrade(gateway.port, &request_path).await;

    assert_eq!(status, 401);
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for a member token forged with the wrong secret"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_a_member_token_forged_with_the_wrong_secret() {
    returns_401_for(format!(
        "/rt/workspaces/ws-1?token={}",
        member_token("ws-1", "user-1", "wrong-secret")
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for an expired member token"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_an_expired_member_token() {
    returns_401_for(format!(
        "/rt/workspaces/ws-1?token={}",
        expired_member_token("ws-1", SECRET)
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for a member token minted for a different workspace"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_a_member_token_minted_for_a_different_workspace()
 {
    returns_401_for(format!(
        "/rt/workspaces/ws-1?token={}",
        member_token("ws-other", "user-1", SECRET)
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for a forged guest token"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_a_forged_guest_token() {
    returns_401_for(format!(
        "/rt/guests/guest-1?token={}",
        guest_token("guest-1", "ws-1", "wrong-secret")
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for an expired guest token"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_an_expired_guest_token() {
    returns_401_for(format!(
        "/rt/guests/guest-1?token={}",
        expired_guest_token("guest-1", "ws-1", SECRET)
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for a guest token minted for a different guest conversation"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_a_guest_token_minted_for_a_different_guest_conversation()
 {
    returns_401_for(format!(
        "/rt/guests/ws-1:guest-a?token={}",
        guest_token("ws-other:guest-a", "ws-other", SECRET)
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for a member-connect token used against the guest route"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_a_member_connect_token_used_against_the_guest_route()
 {
    returns_401_for(format!(
        "/rt/guests/ws-1?token={}",
        member_token("ws-1", "user-1", SECRET)
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:281 "createRealtimeGateway (live) > returns 401 for a guest-connect token used against the member route"
#[tokio::test]
async fn createrealtimegateway_live_returns_401_for_a_guest_connect_token_used_against_the_member_route()
 {
    returns_401_for(format!(
        "/rt/workspaces/ws-1?token={}",
        guest_token("ws-1:guest-a", "ws-1", SECRET)
    ))
    .await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:365 "createRealtimeGateway (live) > keeps health live and becomes ready after the initial Redis probe"
#[tokio::test]
async fn createrealtimegateway_live_keeps_health_live_and_becomes_ready_after_the_initial_redis_probe()
 {
    // Adapted: no Redis probe; ready while serving, not ready once the
    // gateway shuts down; health stays live throughout.
    let gateway = TestGateway::start(gateway_config(), Fakes::new()).await;

    assert_eq!(request_http(gateway.port, "/health").await, 200);
    assert_eq!(request_http(gateway.port, "/ready").await, 200);

    gateway.rt.shutdown().await;

    assert_eq!(request_http(gateway.port, "/ready").await, 503);
    assert_eq!(request_http(gateway.port, "/health").await, 200);
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:596 "createRealtimeGateway (live) > caps a workspace's total guest connections across distinct guest conversation ids"
#[tokio::test]
async fn createrealtimegateway_live_caps_a_workspace_s_total_guest_connections_across_distinct_guest_conversation_ids()
 {
    let mut cfg = gateway_config();
    cfg.max_guest_connections_per_workspace = 1;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let token_a = guest_token("guest-a", "ws-1", SECRET);
    let token_b = guest_token("guest-b", "ws-1", SECRET);

    let first = connect(&gateway.url(&format!("/rt/guests/guest-a?token={token_a}"))).await;
    let mut second = connect(&gateway.url(&format!("/rt/guests/guest-b?token={token_b}"))).await;

    assert_eq!(wait_for_close(&mut second).await.0, 4003);

    close_client(first).await;
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:639 "createRealtimeGateway (live) > N+k parallel upgrades against the cap: exactly N succeed"
#[tokio::test]
async fn createrealtimegateway_live_n_k_parallel_upgrades_against_the_cap_exactly_n_succeed() {
    const CAP: usize = 3;
    const EXTRA: usize = 2;
    let mut cfg = gateway_config();
    cfg.max_connections_per_workspace = CAP;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let token = member_token("ws-1", "user-1", SECRET);
    let url = gateway.url(&format!("/rt/workspaces/ws-1?token={token}"));

    // Every connect starts at once, so every upgrade races its admission.
    let mut sockets = futures_util::future::join_all((0..CAP + EXTRA).map(|_| connect(&url))).await;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    let mut open = Vec::new();
    let mut not_open = 0;
    for mut socket in sockets.drain(..) {
        match close_before(&mut socket, deadline).await {
            None => open.push(socket),
            Some((code, _)) => {
                assert_eq!(code, 4003);
                not_open += 1;
            }
        }
    }

    assert_eq!(open.len(), CAP);
    assert_eq!(not_open, EXTRA);
    for socket in open {
        close_client(socket).await;
    }
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:702 "createRealtimeGateway (live) > releases a workspace's connection slot once its socket closes"
#[tokio::test]
async fn createrealtimegateway_live_releases_a_workspace_s_connection_slot_once_its_socket_closes()
{
    let mut cfg = gateway_config();
    cfg.max_connections_per_workspace = 1;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let token = member_token("ws-1", "user-1", SECRET);
    let url = gateway.url(&format!("/rt/workspaces/ws-1?token={token}"));

    let first = connect(&url).await;
    close_client(first).await;

    let mut second = connect(&url).await;
    assert!(stays_open(&mut second, 50).await);

    close_client(second).await;
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:738 "createRealtimeGateway (live) > releases a guest connection slot once its socket closes"
#[tokio::test]
async fn createrealtimegateway_live_releases_a_guest_connection_slot_once_its_socket_closes() {
    let mut cfg = gateway_config();
    cfg.max_guest_connections_per_workspace = 1;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let first_token = guest_token("guest-1", "ws-1", SECRET);
    let first = connect(&gateway.url(&format!("/rt/guests/guest-1?token={first_token}"))).await;
    close_client(first).await;

    let second_token = guest_token("guest-2", "ws-1", SECRET);
    let mut second =
        connect(&gateway.url(&format!("/rt/guests/guest-2?token={second_token}"))).await;

    assert!(stays_open(&mut second, 50).await);
    close_client(second).await;
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:774 "createRealtimeGateway (live) > accepts a percent-encoded guest conversation id"
#[tokio::test]
async fn createrealtimegateway_live_accepts_a_percent_encoded_guest_conversation_id() {
    let gateway = TestGateway::start(gateway_config(), Fakes::new()).await;
    let guest_conversation_id = "ws-1:guest-1";
    let token = guest_token(guest_conversation_id, "ws-1", SECRET);

    let socket = connect(&gateway.url(&format!("/rt/guests/ws-1%3Aguest-1?token={token}"))).await;

    close_client(socket).await;
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:797 "createRealtimeGateway (live) > releases a member pending-upgrade slot and stream reference when the request aborts during revoke recheck"
#[tokio::test]
async fn createrealtimegateway_live_releases_a_member_pending_upgrade_slot_and_stream_reference_when_the_request_aborts_during_revoke_recheck()
 {
    let fakes = Fakes::new();
    let recheck_started = Deferred::new();
    let recheck = Deferred::new();
    fakes.revoke_store.push_read(ReadStep::Value(None));
    fakes.revoke_store.push_read(ReadStep::Block {
        started: recheck_started.clone(),
        release: recheck.clone(),
    });
    let mut cfg = gateway_config();
    cfg.max_connections_per_workspace = 1;
    let gateway = TestGateway::start(cfg, fakes).await;
    let token = member_token("ws-1", "user-1", SECRET);

    let aborted_request =
        start_upgrade(gateway.port, &format!("/rt/workspaces/ws-1?token={token}")).await;
    tokio::time::timeout(IO_WAIT, recheck_started.wait())
        .await
        .unwrap();
    drop(aborted_request);
    recheck.resolve();
    wait_for(IO_WAIT, || {
        gateway.rt.pending_upgrades() == 0 && gateway.rt.hub().workspace_refs("ws-1") == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut socket = connect(&gateway.url(&format!("/rt/workspaces/ws-1?token={token}"))).await;

    assert!(stays_open(&mut socket, 50).await);
    close_client(socket).await;
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:846 "createRealtimeGateway (live) > releases a guest pending-upgrade slot and stream reference when the request aborts"
#[tokio::test]
async fn createrealtimegateway_live_releases_a_guest_pending_upgrade_slot_and_stream_reference_when_the_request_aborts()
 {
    // Adapted: a guest upgrade awaits nothing in Rust, so the client drops
    // the connection right after sending the request.
    let mut cfg = gateway_config();
    cfg.max_guest_connections_per_workspace = 1;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let token = guest_token("guest-1", "ws-1", SECRET);

    let aborted_request =
        start_upgrade(gateway.port, &format!("/rt/guests/guest-1?token={token}")).await;
    drop(aborted_request);
    tokio::time::sleep(Duration::from_millis(50)).await;
    wait_for(IO_WAIT, || {
        gateway.rt.pending_upgrades() == 0
            && gateway.rt.pending_guest_upgrades() == 0
            && gateway.rt.hub().workspace_refs("ws-1") == 0
    })
    .await;

    let mut socket = connect(&gateway.url(&format!("/rt/guests/guest-1?token={token}"))).await;

    assert!(stays_open(&mut socket, 50).await);
    close_client(socket).await;
    gateway.close().await;
}

async fn forces_reauth_once_its_connection_lifetime_elapses(member: bool) {
    let mut cfg = gateway_config();
    cfg.connection_lifetime_ms = 50;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let path = if member {
        format!(
            "/rt/workspaces/ws-1?token={}",
            member_token("ws-1", "user-1", SECRET)
        )
    } else {
        format!(
            "/rt/guests/guest-1?token={}",
            guest_token("guest-1", "ws-1", SECRET)
        )
    };
    let mut socket = connect(&gateway.url(&path)).await;

    assert_eq!(
        wait_for_close(&mut socket).await,
        (4004, "connection-lifetime-exceeded".to_owned())
    );
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:889 "createRealtimeGateway (live) > forces a member socket to reauth once its connection lifetime elapses"
#[tokio::test]
async fn createrealtimegateway_live_forces_a_member_socket_to_reauth_once_its_connection_lifetime_elapses()
 {
    forces_reauth_once_its_connection_lifetime_elapses(true).await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:889 "createRealtimeGateway (live) > forces a guest socket to reauth once its connection lifetime elapses"
#[tokio::test]
async fn createrealtimegateway_live_forces_a_guest_socket_to_reauth_once_its_connection_lifetime_elapses()
 {
    forces_reauth_once_its_connection_lifetime_elapses(false).await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:925 "createRealtimeGateway (live) > sends an open-time cursor frame even when nothing was replayed"
#[tokio::test]
async fn createrealtimegateway_live_sends_an_open_time_cursor_frame_even_when_nothing_was_replayed()
{
    let gateway = TestGateway::start(gateway_config(), Fakes::new()).await;
    let token = member_token("ws-1", "user-1", SECRET);
    let mut socket = connect(&gateway.url(&format!("/rt/workspaces/ws-1?token={token}"))).await;

    let frame = wait_for_message(&mut socket).await;

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&frame).unwrap(),
        json!({ "batch": [], "seq": "0-0" })
    );
    close_client(socket).await;
    gateway.close().await;
}

async fn closes_with_resync_for(last_seq: &str, reason: &str) {
    let gateway = TestGateway::start(gateway_config(), Fakes::new()).await;
    let token = member_token("ws-1", "user-1", SECRET);
    let mut socket = connect(&gateway.url(&format!(
        "/rt/workspaces/ws-1?token={token}&lastSeq={last_seq}"
    )))
    .await;
    let opened_at = tokio::time::Instant::now();

    assert_eq!(wait_for_close(&mut socket).await, (4002, reason.to_owned()));
    assert!(opened_at.elapsed() < CLOSE_ON_OPEN_MAX);
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:958 "createRealtimeGateway (live) > closes with resync for a malformed lastSeq"
#[tokio::test]
async fn createrealtimegateway_live_closes_with_resync_for_a_malformed_lastseq() {
    closes_with_resync_for("not-a-stream-id", "invalid-last-seq").await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:958 "createRealtimeGateway (live) > closes with resync for a stale lastSeq"
#[tokio::test]
async fn createrealtimegateway_live_closes_with_resync_for_a_stale_lastseq() {
    closes_with_resync_for("1-0", "replay-cursor-ahead").await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:994 "createRealtimeGateway (live) > closes a workspace socket with resync when its recent-entry buffer has evicted unseen records"
#[tokio::test]
async fn createrealtimegateway_live_closes_a_workspace_socket_with_resync_when_its_recent_entry_buffer_has_evicted_unseen_records()
 {
    // Adapted: one buffer (512 entries) replaces the replay stream and the
    // live-reader ring; the evicted part answers `replay-window-expired`.
    let workspace_id = "ws-recent-gap";
    let gateway = TestGateway::start(gateway_config(), Fakes::new()).await;
    for _ in 0..513 {
        let typing: Envelope =
            serde_json::from_value(json!({ "data": {}, "eventType": "typing" })).unwrap();
        gateway
            .rt
            .hub()
            .append_workspace_events(workspace_id, vec![typing]);
    }
    let token = member_token(workspace_id, "user-1", SECRET);

    let mut socket = connect(&gateway.url(&format!(
        "/rt/workspaces/{workspace_id}?token={token}&lastSeq=0-0"
    )))
    .await;

    assert_eq!(
        wait_for_close(&mut socket).await,
        (4002, "replay-window-expired".to_owned())
    );
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1067 "createRealtimeGateway (live) > rejects a connect whose token was minted before a revoke was recorded, even with no lastSeq to replay through"
#[tokio::test]
async fn createrealtimegateway_live_rejects_a_connect_whose_token_was_minted_before_a_revoke_was_recorded_even_with_no_lastseq_to_replay_through()
 {
    let fakes = Fakes::new();
    fakes
        .revoke_store
        .set_default(Some((now_ms() + 60_000).to_string()));
    let gateway = TestGateway::start(gateway_config(), fakes).await;
    let token = member_token("ws-1", "user-1", SECRET);

    let status = request_upgrade(gateway.port, &format!("/rt/workspaces/ws-1?token={token}")).await;

    assert_eq!(status, 401);
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1097 "createRealtimeGateway (live) > fails closed when the initial revoke check cannot read Redis"
#[tokio::test]
async fn createrealtimegateway_live_fails_closed_when_the_initial_revoke_check_cannot_read_redis() {
    // Adapted: the marker store (SQLite in production) fails the read; no
    // replay snapshot is taken (no recheck, no reference left).
    let fakes = Fakes::new();
    fakes.revoke_store.push_read(ReadStep::Fail);
    let gateway = TestGateway::start(gateway_config(), fakes).await;
    let token = member_token("ws-1", "user-1", SECRET);

    let status = request_upgrade(gateway.port, &format!("/rt/workspaces/ws-1?token={token}")).await;

    assert_eq!(status, 503);
    assert_eq!(gateway.fakes.revoke_store.reads(), 1);
    wait_for(IO_WAIT, || {
        gateway.rt.pending_upgrades() == 0 && gateway.rt.hub().workspace_refs("ws-1") == 0
    })
    .await;
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1122 "createRealtimeGateway (live) > fails closed and releases admission state when the revoke recheck cannot read Redis"
#[tokio::test]
async fn createrealtimegateway_live_fails_closed_and_releases_admission_state_when_the_revoke_recheck_cannot_read_redis()
 {
    let fakes = Fakes::new();
    fakes.revoke_store.push_read(ReadStep::Value(None));
    fakes.revoke_store.push_read(ReadStep::Fail);
    let mut cfg = gateway_config();
    cfg.max_connections_per_workspace = 1;
    let gateway = TestGateway::start(cfg, fakes).await;
    let token = member_token("ws-1", "user-1", SECRET);

    let status = request_upgrade(gateway.port, &format!("/rt/workspaces/ws-1?token={token}")).await;
    assert_eq!(status, 503);

    let mut socket = connect(&gateway.url(&format!("/rt/workspaces/ws-1?token={token}"))).await;
    assert!(stays_open(&mut socket, 50).await);
    close_client(socket).await;
    gateway.close().await;
}

async fn handles_revoke_marker_boundary(
    iat_offset: i64,
    marker: bool,
    expected_status: Option<u16>,
) {
    let revoked_at = now_ms();
    let fakes = Fakes::new();
    if marker {
        fakes.revoke_store.set_default(Some(revoked_at.to_string()));
    }
    let gateway = TestGateway::start(gateway_config(), fakes).await;
    let token = member_token_with_iat_ms("ws-1", "user-1", revoked_at + iat_offset, SECRET);
    let path = format!("/rt/workspaces/ws-1?token={token}");

    if let Some(expected_status) = expected_status {
        assert_eq!(request_upgrade(gateway.port, &path).await, expected_status);
    } else {
        let mut socket = connect(&gateway.url(&path)).await;
        assert!(stays_open(&mut socket, 50).await);
        close_client(socket).await;
    }
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1165 "createRealtimeGateway (live) > revoke-marker iatMs boundary > handles a token minted before the revoke marker"
#[tokio::test]
async fn createrealtimegateway_live_revoke_marker_iatms_boundary_handles_a_token_minted_before_the_revoke_marker()
 {
    handles_revoke_marker_boundary(-1, true, Some(401)).await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1165 "createRealtimeGateway (live) > revoke-marker iatMs boundary > handles a token minted exactly at the revoke marker"
#[tokio::test]
async fn createrealtimegateway_live_revoke_marker_iatms_boundary_handles_a_token_minted_exactly_at_the_revoke_marker()
 {
    handles_revoke_marker_boundary(0, true, None).await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1165 "createRealtimeGateway (live) > revoke-marker iatMs boundary > handles a token minted after the revoke marker"
#[tokio::test]
async fn createrealtimegateway_live_revoke_marker_iatms_boundary_handles_a_token_minted_after_the_revoke_marker()
 {
    handles_revoke_marker_boundary(1, true, None).await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1165 "createRealtimeGateway (live) > revoke-marker iatMs boundary > handles a token with no revoke marker"
#[tokio::test]
async fn createrealtimegateway_live_revoke_marker_iatms_boundary_handles_a_token_with_no_revoke_marker()
 {
    handles_revoke_marker_boundary(0, false, None).await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1412 "createRealtimeGateway (live) > presence: dedupes two tabs of the same user and coalesces a second user's connect into one report"
#[tokio::test]
async fn createrealtimegateway_live_presence_dedupes_two_tabs_of_the_same_user_and_coalesces_a_second_user_s_connect_into_one_report()
 {
    // The fake `presenceTimerScheduler` is a coalesce window that never
    // fires by itself plus `flush_presence_reports()` (the manual fire).
    let mut cfg = gateway_config();
    cfg.presence_report_coalesce_ms = 3_600_000;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let token_a = member_token("ws-presence-1", "user-a", SECRET);
    let token_b = member_token("ws-presence-1", "user-b", SECRET);
    let url_a = gateway.url(&format!("/rt/workspaces/ws-presence-1?token={token_a}"));
    let url_b = gateway.url(&format!("/rt/workspaces/ws-presence-1?token={token_b}"));

    let (first_tab, second_tab, other_user) =
        tokio::join!(connect(&url_a), connect(&url_a), connect(&url_b));
    wait_for(IO_WAIT, || {
        gateway.rt.presence_users("ws-presence-1").len() == 2
    })
    .await;

    assert_eq!(gateway.rt.pending_presence_reports(), 1);
    gateway.rt.flush_presence_reports();
    wait_for(IO_WAIT, || {
        !gateway.fakes.presence.calls_for("ws-presence-1").is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    let calls = gateway.fakes.presence.calls_for("ws-presence-1");
    assert_eq!(calls.len(), 1);
    let mut user_ids = calls[0].clone();
    user_ids.sort();
    assert_eq!(user_ids, vec!["user-a".to_owned(), "user-b".to_owned()]);

    drop((first_tab, second_tab, other_user));
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway.test.ts:1489 "createRealtimeGateway (live) > presence: lets the last presence report expire after the final tab closes"
#[tokio::test]
async fn createrealtimegateway_live_presence_lets_the_last_presence_report_expire_after_the_final_tab_closes()
 {
    let mut cfg = gateway_config();
    cfg.presence_report_coalesce_ms = 30;
    cfg.presence_report_interval_ms = 10_000;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let token = member_token("ws-presence-2", "user-1", SECRET);
    let socket =
        connect(&gateway.url(&format!("/rt/workspaces/ws-presence-2?token={token}"))).await;
    let previous_presence_report_count = gateway.fakes.presence.calls().len();
    wait_for(IO_WAIT, || {
        gateway.fakes.presence.calls().len() > previous_presence_report_count
    })
    .await;
    gateway.fakes.presence.clear();

    close_client(socket).await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert!(gateway.fakes.presence.calls().is_empty());
    gateway.close().await;
}
