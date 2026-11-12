//! Port of `apps/realtime/__tests__/presence-report.test.ts` (P2.8 spec 4.6,
//! R169-R172). The `ky.post` to the builder becomes an in-process
//! `PresenceSink` call (decision D3). R169 (POST URL, bearer token,
//! `bodyHash`) is not applicable (user decision 2026-11-12, DECISIONS P2.8).

mod support;

use chatbotx_rt::presence::report_workspace_presence;
use chatbotx_rt::protocol::MAX_PRESENCE_USER_IDS_PER_REPORT;
use support::{RecordingLog, RecordingPresence, strings};
use tracing::Level;

/// Port of: apps/realtime/__tests__/presence-report.test.ts:60 "reportWorkspacePresence > truncates an over-cap userIds batch before hashing and sending (LOW-7)"
#[tokio::test]
async fn reportworkspacepresence_truncates_an_over_cap_userids_batch_before_hashing_and_sending_low_7()
 {
    let sink = RecordingPresence::new();
    let user_ids: Vec<String> = (0..MAX_PRESENCE_USER_IDS_PER_REPORT + 5)
        .map(|i| format!("u_{i}"))
        .collect();

    report_workspace_presence(&*sink, "ws_1", &user_ids).await;

    let calls = sink.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1.len(), MAX_PRESENCE_USER_IDS_PER_REPORT);
}

/// Port of: apps/realtime/__tests__/presence-report.test.ts:81 "reportWorkspacePresence > reports success to the caller once the builder accepted it"
#[tokio::test]
async fn reportworkspacepresence_reports_success_to_the_caller_once_the_builder_accepted_it() {
    let sink = RecordingPresence::new();

    assert!(report_workspace_presence(&*sink, "ws_1", &strings(&["u_1"])).await);
}

/// Port of: apps/realtime/__tests__/presence-report.test.ts:92 "reportWorkspacePresence > logs and swallows a failed report — never throws (no retry, the next report supersedes it)"
#[tokio::test]
async fn reportworkspacepresence_logs_and_swallows_a_failed_report_never_throws_no_retry_the_next_report_supersedes_it()
 {
    let log = RecordingLog::install();
    let sink = RecordingPresence::failing();

    assert!(!report_workspace_presence(&*sink, "ws_1", &strings(&["u_1"])).await);

    assert_eq!(sink.calls().len(), 1);
    let errors = log.at(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field("err"), Some("network error"));
    assert_eq!(errors[0].field("workspace_id"), Some("ws_1"));
}
