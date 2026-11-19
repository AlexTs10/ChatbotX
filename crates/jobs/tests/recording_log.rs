//! Regression check for the `RecordingLog` helper (no TS original): a log
//! site first hit on a thread without a subscriber must still reach a
//! recorder that is live on another thread.

mod support;

use support::RecordingLog;
use tracing::Level;

fn log_site() {
    tracing::warn!("shared callsite");
}

#[test]
fn a_callsite_first_hit_on_a_thread_without_a_subscriber_still_records() {
    let log = RecordingLog::install();
    std::thread::spawn(log_site).join().unwrap();
    log_site();
    assert_eq!(log.at(Level::WARN).len(), 1);
}
