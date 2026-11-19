//! `schedule/handlers/google-ads-housekeeping.ts`: the 10-minute
//! housekeeping (status poll, then the stranded sweep) and the daily setup
//! sync, one run at a time each.

use std::sync::LazyLock;

use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::google_ads_conversion::DeliverDeps;
use chatbotx_business::google_ads_housekeeping as hk;
use chatbotx_db::repo::now_ms;
use chatbotx_jobs::JobError;
use tokio::sync::Mutex;

use crate::Deps;

// ponytail: one process (one binary), so `runExclusive` is a process lock;
// the original's Redis lock only adds the cross-process half.
static HOUSEKEEPING: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static SYNC_SETUPS: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Every 10 minutes: read Data Manager outcomes, then rescue stranded
/// deliveries. A poll failure still lets the sweep run; the first failure
/// fails the job.
pub(crate) async fn housekeeping(deps: &Deps) -> Result<(), JobError> {
    let Ok(_running) = HOUSEKEEPING.try_lock() else {
        tracing::warn!(
            name = "google-ads-housekeeping",
            "skipped because a local run is still in progress"
        );
        return Ok(());
    };
    let delivery = DeliverDeps {
        db: &deps.db,
        up: &HttpUpstream,
        jobs: deps.jobs.as_ref(),
        events: deps.events.as_ref(),
        app_url: &deps.app_url,
        matching: None,
    };
    let poll = hk::poll_processing_status(&delivery, now_ms()).await;
    let sweep = hk::sweep_stranded(&delivery, now_ms(), hk::SWEEP_LIMIT).await;
    for (step, result) in [("poll", &poll), ("sweep", &sweep)] {
        if let Err(error) = result {
            tracing::error!(step, error = %error.message(), "google ads housekeeping step failed");
        }
    }
    poll.and(sweep)
        .map_err(|error| JobError::retryable(error.message()))
}

/// Daily: re-read each account's conversion actions, surface revoked grants.
pub(crate) async fn sync_setups(deps: &Deps) -> Result<(), JobError> {
    let Ok(_running) = SYNC_SETUPS.try_lock() else {
        tracing::warn!(
            name = "google-ads-sync-setups",
            "skipped because a local run is still in progress"
        );
        return Ok(());
    };
    hk::sync_setups(
        &deps.db,
        &HttpUpstream,
        chatbotx_business::meta_catalog::process_encryption(),
        &deps.app_url,
    )
    .await
    .map_err(|error| JobError::retryable(error.message()))
}
