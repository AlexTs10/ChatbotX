//! Port of `apps/realtime/__tests__/gateway-socket.test.ts` (P2.8 spec 4.6,
//! R124-R126). The uWS behavior objects with a fake `res.onAborted` become
//! the live axum route with a client that drops its TCP connection (axum
//! drops the handler future; the RAII guards release the slots). R125 runs
//! the open-time steps on a `FakeSink` whose cursor send fails.

mod support;

use std::sync::Arc;

use chatbotx_rt::{OpenOutcome, TestClock};
use support::gateway::{
    SECRET, TestGateway, gateway_config, guest_token, member_token, start_upgrade,
};
use support::{
    Deferred, Fakes, IO_WAIT, ReadStep, SendMode, member_data, member_socket, test_realtime,
    wait_for,
};

/// Port of: apps/realtime/__tests__/gateway-socket.test.ts:41 "gateway socket upgrades > releases a workspace stream reference when an admitted upgrade aborts"
#[tokio::test]
async fn gateway_socket_upgrades_releases_a_workspace_stream_reference_when_an_admitted_upgrade_aborts()
 {
    // The fake `activateWorkspace` promise is a blocking first marker read.
    let fakes = Fakes::new();
    let started = Deferred::new();
    let release = Deferred::new();
    fakes.revoke_store.push_read(ReadStep::Block {
        started: started.clone(),
        release: release.clone(),
    });
    let mut cfg = gateway_config();
    cfg.secret = "gateway-socket-test-secret".to_owned();
    cfg.max_connections_per_workspace = 10;
    let gateway = TestGateway::start(cfg, fakes).await;
    let token = member_token("workspace-1", "user-1", "gateway-socket-test-secret");

    let request = start_upgrade(
        gateway.port,
        &format!("/rt/workspaces/workspace-1?token={token}"),
    )
    .await;
    tokio::time::timeout(IO_WAIT, started.wait()).await.unwrap();
    drop(request);
    release.resolve();

    wait_for(IO_WAIT, || {
        gateway.rt.hub().workspace_refs("workspace-1") == 0 && gateway.rt.pending_upgrades() == 0
    })
    .await;
    gateway.close().await;
}

/// Port of: apps/realtime/__tests__/gateway-socket.test.ts:92 "gateway socket upgrades > does not add presence or arm a timer when sending the open-time cursor closes the workspace socket"
#[tokio::test(start_paused = true)]
async fn gateway_socket_upgrades_does_not_add_presence_or_arm_a_timer_when_sending_the_open_time_cursor_closes_the_workspace_socket()
 {
    let (rt, fakes) = test_realtime(Arc::new(TestClock::new(1_000)));
    let (socket, sink) = member_socket(member_data(), SendMode::Fail);
    let attached = rt.attach_member_socket(&socket, None);

    let outcome = rt.open_member_socket(&socket, &attached);

    assert_eq!(outcome, OpenOutcome::Closed);
    assert!(socket.is_closed());
    assert_eq!(sink.closed().map(|(code, _)| code), Some(4002));
    assert!(rt.presence_users("workspace-1").is_empty());
    assert_eq!(rt.pending_presence_reports(), 0);
    assert!(fakes.presence.calls().is_empty());
}

/// Port of: apps/realtime/__tests__/gateway-socket.test.ts:144 "gateway socket upgrades > releases a guest stream reference when an admitted upgrade aborts"
#[tokio::test]
async fn gateway_socket_upgrades_releases_a_guest_stream_reference_when_an_admitted_upgrade_aborts()
{
    // Adapted: a guest upgrade awaits nothing in Rust; the client drops the
    // connection right after the request.
    let mut cfg = gateway_config();
    cfg.max_connections_per_guest = 10;
    cfg.max_guest_connections = 10;
    cfg.max_guest_connections_per_workspace = 10;
    let gateway = TestGateway::start(cfg, Fakes::new()).await;
    let token = guest_token("guest-1", "workspace-1", SECRET);

    let request = start_upgrade(gateway.port, &format!("/rt/guests/guest-1?token={token}")).await;
    drop(request);

    wait_for(IO_WAIT, || {
        gateway.rt.hub().workspace_refs("workspace-1") == 0
            && gateway.rt.pending_upgrades() == 0
            && gateway.rt.pending_guest_upgrades() == 0
    })
    .await;
    gateway.close().await;
}
