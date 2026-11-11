//! Port of `apps/worker/__tests__/register-schedules-edition.test.ts`
//! (`registerSchedules` edition gating and every-minute cadence, spec 1.3 /
//! S13). The `scheduleQueue` mock is a recording `SchedulerStore` fake; the
//! env mock is a `ScheduleConfig` with the same values.

#[path = "support/deferral.rs"]
mod deferral;

use chatbotx_jobs::schedules::{
    Edition, ScheduleConfig, license_refresh_minute, register_schedules,
};
use deferral::FakeSchedulerStore;
use serde_json::json;

const CLOUD_ONLY: [&str; 3] = [
    "syncUserQuota",
    "reconcileTenants",
    "unsubscribeExpiredTrials",
];

const ON_THE_MINUTE: [&str; 5] = [
    "enqueueBroadcast",
    "enqueueRecurringBroadcasts",
    "enqueueMarketingMessages",
    "evaluateTriggers",
    "evaluateDateTimeWebhooks",
];

const SPREAD_OVER_THE_MINUTE: [&str; 12] = [
    "expireUnpaidOrders",
    "finishOverdueLeaderboards",
    "dispatchMarketingMessages",
    "scanAbandonedCarts",
    "reconcileBroadcasts",
    "dispatchProfileSnapshots",
    "scanCoexistRuns",
    "scanContactScans",
    "scanAiHandoverBulkRuns",
    "finalizeBroadcasts",
    "finalizeMarketingMessages",
    "reconcileMetaCatalogSyncs",
];

/// The env mock: `PURGE_WORKSPACES_INTERVAL_MINUTES: 10`,
/// `PURGE_BROADCASTS_INTERVAL_MINUTES: 5`, `QUOTA_SYNC_INTERVAL_SECONDS: 60`.
fn config(edition: Edition) -> ScheduleConfig {
    ScheduleConfig {
        edition,
        quota_sync_interval_seconds: 60,
        purge_workspaces_interval_minutes: 10,
        purge_broadcasts_interval_minutes: 5,
        license_refresh_minute: license_refresh_minute(&|| 0.7),
    }
}

async fn registered(edition: Edition) -> FakeSchedulerStore {
    let store = FakeSchedulerStore::default();
    register_schedules(&config(edition), &store)
        .await
        .expect("registerSchedules");
    store
}

fn is_number_1_or_2_digits(s: &str) -> bool {
    (1..=2).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// `/^\d{1,2} \* \* \* \*$/`
fn is_hourly_cron(pattern: &str) -> bool {
    match pattern.strip_suffix(" * * * *") {
        Some(minute) => is_number_1_or_2_digits(minute),
        None => false,
    }
}

/// `/^(?:(\d{1,2}) )?\* \* \* \* \*$/`: `None` = no match; `Some(group 1)`.
fn every_minute_second(pattern: &str) -> Option<Option<u32>> {
    if pattern == "* * * * *" {
        return Some(None);
    }
    let second = pattern.strip_suffix(" * * * * *")?;
    is_number_1_or_2_digits(second).then(|| second.parse().ok())
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:104 "registerSchedules — edition gating > cloud registers the quota/trial schedulers and removes none"
#[tokio::test]
async fn cloud_registers_the_quota_trial_schedulers_and_removes_none() {
    let store = registered(Edition::Cloud).await;

    let names = store.upserted_names();
    for name in CLOUD_ONLY {
        assert!(names.iter().any(|n| n == name), "{name} missing");
    }
    assert!(store.removed().is_empty());
    assert!(names.iter().any(|n| n == "refreshLicense"));
    let pattern = store.pattern_for("refreshLicense").expect("pattern");
    assert!(is_hourly_cron(&pattern), "{pattern}");
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:120 "registerSchedules — edition gating > community removes all persisted cloud and license schedulers"
#[tokio::test]
async fn community_removes_all_persisted_cloud_and_license_schedulers() {
    let store = registered(Edition::Community).await;

    let names = store.upserted_names();
    let removed = store.removed();
    for name in CLOUD_ONLY {
        assert!(!names.iter().any(|n| n == name), "{name} upserted");
        assert!(removed.iter().any(|n| n == name), "{name} not removed");
    }
    assert!(!names.iter().any(|n| n == "refreshLicense"));
    assert!(removed.iter().any(|n| n == "refreshLicense"));
    assert_eq!(removed.len(), CLOUD_ONLY.len() + 1);
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:135 "registerSchedules — edition gating > enterprise registers the license scheduler and removes cloud-only schedulers"
#[tokio::test]
async fn enterprise_registers_the_license_scheduler_and_removes_cloud() {
    let store = registered(Edition::Enterprise).await;

    let names = store.upserted_names();
    let removed = store.removed();
    for name in CLOUD_ONLY {
        assert!(!names.iter().any(|n| n == name), "{name} upserted");
        assert!(removed.iter().any(|n| n == name), "{name} not removed");
    }
    assert!(names.iter().any(|n| n == "refreshLicense"));
    let pattern = store.pattern_for("refreshLicense").expect("pattern");
    assert!(is_hourly_cron(&pattern), "{pattern}");
    assert_eq!(removed.len(), CLOUD_ONLY.len());
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:152 "registerSchedules — edition gating > all-edition schedulers register regardless of edition"
#[tokio::test]
async fn all_edition_schedulers_register_regardless_of_edition() {
    let store = registered(Edition::Community).await;

    let names = store.upserted_names();
    for name in [
        "purgeWorkspaces",
        "purgeExpiredOAuthTokens",
        "purgeBroadcasts",
        "maintainMacPartitions",
        "maintainLeaderboardPartitions",
        "finishOverdueLeaderboards",
        "maintainLeaderboardRankings",
        "enqueueBroadcast",
        "purgeExpiredConnectSessions",
        "googleAdsHousekeeping",
        "googleAdsSyncSetups",
    ] {
        assert!(names.iter().any(|n| n == name), "{name} missing");
    }
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:171 "registerSchedules — edition gating > google ads crons run every 10 minutes and daily at 03:30"
#[tokio::test]
async fn google_ads_crons_run_every_10_minutes_and_daily_at_03_30() {
    let store = registered(Edition::Community).await;

    assert_eq!(
        store.pattern_for("googleAdsHousekeeping").as_deref(),
        Some("*/10 * * * *")
    );
    assert_eq!(
        store.pattern_for("googleAdsSyncSetups").as_deref(),
        Some("30 3 * * *")
    );
    let call = store.upsert_for("googleAdsHousekeeping").expect("upsert");
    assert_eq!(call.template.name, "googleAdsHousekeeping");
    assert_eq!(
        call.template.data,
        json!({ "type": "googleAdsHousekeeping", "data": {} })
    );
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:194 "registerSchedules — edition gating > purgeBroadcasts registers with a pattern derived from PURGE_BROADCASTS_INTERVAL_MINUTES"
#[tokio::test]
async fn purgebroadcasts_registers_with_a_pattern_derived_from() {
    let store = registered(Edition::Community).await;

    assert_eq!(
        store.pattern_for("purgeBroadcasts").as_deref(),
        Some("*/5 * * * *")
    );
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:238 "registerSchedules — every-minute cadence > keeps the minute-boundary jobs at second :00"
#[tokio::test]
async fn keeps_the_minute_boundary_jobs_at_second_00() {
    let store = registered(Edition::Community).await;

    for name in ON_THE_MINUTE {
        assert_eq!(
            store.pattern_for(name).as_deref(),
            Some("* * * * *"),
            "{name}"
        );
    }
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:248 "registerSchedules — every-minute cadence > moves every spread job to its own second between :01 and :59"
#[tokio::test]
async fn moves_every_spread_job_to_its_own_second_between_01_and_59() {
    let store = registered(Edition::Community).await;

    let mut seconds = Vec::new();
    for name in SPREAD_OVER_THE_MINUTE {
        let pattern = store.pattern_for(name).unwrap_or_default();
        let second = every_minute_second(&pattern).flatten();
        let second = second.unwrap_or_else(|| panic!("{name}: {pattern:?} has no second"));
        assert!((1..=59).contains(&second), "{name}: {second}");
        seconds.push(second);
    }
    seconds.sort_unstable();
    seconds.dedup();
    assert_eq!(seconds.len(), SPREAD_OVER_THE_MINUTE.len());
}

/// Port of: apps/worker/__tests__/register-schedules-edition.test.ts:270 "registerSchedules — every-minute cadence > registers no every-minute job outside the two classified groups"
#[tokio::test]
async fn registers_no_every_minute_job_outside_the_two_classified() {
    let store = registered(Edition::Community).await;

    let mut every_minute: Vec<String> = store
        .upserts
        .lock()
        .unwrap()
        .iter()
        .filter(|u| {
            u.repeat
                .pattern()
                .is_some_and(|p| every_minute_second(p).is_some())
        })
        .map(|u| u.id.clone())
        .collect();
    every_minute.sort();
    let mut expected: Vec<String> = ON_THE_MINUTE
        .iter()
        .chain(SPREAD_OVER_THE_MINUTE.iter())
        .map(|s| (*s).to_owned())
        .collect();
    expected.sort();
    assert_eq!(every_minute, expected);
}
