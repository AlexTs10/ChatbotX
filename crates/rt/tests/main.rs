//! Port of `apps/realtime/__tests__/main.test.ts` (P2.8 spec 4.6, R167,
//! R168). The signal handler and `process.exit` belong to the server
//! binary (one process); the gateway part is `Realtime::shutdown`, which is
//! idempotent. R168 (close failure -> exit code 1, forced exit) is not
//! applicable (user decision 2026-11-12, DECISIONS P2.8).

mod support;

use support::RecordingLog;
use support::gateway::{
    SECRET, TestGateway, connect, gateway_config, guest_token, member_token, retry_after,
    wait_for_close,
};
use support::{Fakes, IO_WAIT, wait_for};

/// Port of: apps/realtime/__tests__/main.test.ts:18 "realtime gateway shutdown > closes the gateway once when SIGTERM arrives"
#[tokio::test]
async fn realtime_gateway_shutdown_closes_the_gateway_once_when_sigterm_arrives() {
    let log = RecordingLog::install();
    let gateway = TestGateway::start(gateway_config(), Fakes::new()).await;
    let member_url = gateway.url(&format!(
        "/rt/workspaces/ws-1?token={}",
        member_token("ws-1", "user-1", SECRET)
    ));
    let guest_url = gateway.url(&format!(
        "/rt/guests/guest-1?token={}",
        guest_token("guest-1", "ws-1", SECRET)
    ));
    let mut member = connect(&member_url).await;
    let mut guest = connect(&guest_url).await;
    wait_for(IO_WAIT, || gateway.rt.active_connections() == 2).await;

    tokio::join!(gateway.rt.shutdown(), gateway.rt.shutdown());

    for socket in [&mut member, &mut guest] {
        let (code, reason) = wait_for_close(socket).await;
        assert_eq!(code, 4003);
        let retry_after_ms = retry_after(&reason);
        assert!((1000..5000).contains(&retry_after_ms), "{retry_after_ms}");
    }
    assert_eq!(log.with_message("Stopping realtime gateway").len(), 1);
    assert!(!gateway.rt.is_ready());
    gateway.close().await;
}
