//! Job schedulers (spec 1.3, 2.7, S13): the scheduler table, edition gating
//! (`apps/worker/src/schedule/handlers/register-schedules.ts`), the schedule
//! worker start (`schedule/worker.ts`) and the std-only cron evaluator.
//! Cron times are UTC only (DECISIONS P2.6).

use serde_json::{Value, json};

use crate::clock::Clock;
use crate::enqueue::{BoxFuture, JobsError};
use crate::queue::QueueName;
use crate::store::JobStore;

/// `NEXT_PUBLIC_EDITION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    Community,
    Enterprise,
    Cloud,
}

/// `PURGE_WORKSPACES_INTERVAL_MINUTES` default.
pub const PURGE_WORKSPACES_INTERVAL_MINUTES: u32 = 30;
/// `PURGE_BROADCASTS_INTERVAL_MINUTES` default.
pub const PURGE_BROADCASTS_INTERVAL_MINUTES: u32 = 5;
/// `QUOTA_SYNC_INTERVAL_SECONDS` default.
pub const QUOTA_SYNC_INTERVAL_SECONDS: u64 = 60;

/// Scheduler ids that only the cloud edition registers.
pub const CLOUD_ONLY_SCHEDULERS: [&str; 3] = [
    "syncUserQuota",
    "reconcileTenants",
    "unsubscribeExpiredTrials",
];

/// `{ pattern }` or `{ every }` of `upsertJobScheduler`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    /// Cron, 5 fields or 6 fields (seconds first), UTC.
    Pattern(String),
    /// `every` in ms.
    EveryMs(u64),
}

impl Repeat {
    /// The cron pattern, if this is one.
    pub fn pattern(&self) -> Option<&str> {
        match self {
            Repeat::Pattern(p) => Some(p),
            Repeat::EveryMs(_) => None,
        }
    }
}

/// The job template `{ name, data }` (`data` = `{ type, data }`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleTemplate {
    pub name: String,
    pub data: Value,
}

/// One scheduler of the table.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleDef {
    /// Scheduler id (= job name except `refreshShortLivedChannelTokens`).
    pub id: String,
    pub repeat: Repeat,
    pub template: ScheduleTemplate,
}

/// Inputs of the scheduler table (env values and the boot random minute).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleConfig {
    pub edition: Edition,
    pub quota_sync_interval_seconds: u64,
    pub purge_workspaces_interval_minutes: u32,
    pub purge_broadcasts_interval_minutes: u32,
    /// `LICENSE_REFRESH_MINUTE` (0-59, random per boot).
    pub license_refresh_minute: u32,
}

/// `randomInt(60)` from an injected `rand() -> [0, 1)`.
pub fn license_refresh_minute(rand: &dyn Fn() -> f64) -> u32 {
    ((rand() * 60.0).floor() as u32).min(59)
}

/// Second of the minute each every-minute scanner fires at, so they do not
/// all start at :00. Jobs that read the minute boundary stay on `* * * * *`.
pub const EVERY_MINUTE_SECOND_OFFSETS: [(&str, u32); 12] = [
    ("expireUnpaidOrders", 2),
    ("finishOverdueLeaderboards", 4),
    ("dispatchMarketingMessages", 8),
    ("scanAbandonedCarts", 16),
    ("reconcileBroadcasts", 20),
    ("dispatchProfileSnapshots", 24),
    ("scanCoexistRuns", 28),
    ("scanContactScans", 32),
    ("scanAiHandoverBulkRuns", 36),
    ("finalizeBroadcasts", 40),
    ("finalizeMarketingMessages", 44),
    ("reconcileMetaCatalogSyncs", 48),
];

/// `<offset> * * * * *` (6 fields, seconds first).
fn every_minute_at_offset(job: &str) -> String {
    let second = EVERY_MINUTE_SECOND_OFFSETS
        .iter()
        .find(|(name, _)| *name == job)
        .map_or(0, |(_, s)| *s);
    format!("{second} * * * * *")
}

fn template_with(name: &str, data: Value) -> ScheduleTemplate {
    ScheduleTemplate {
        name: name.to_owned(),
        data: json!({ "type": name, "data": data }),
    }
}

fn cron(name: &str, pattern: String) -> ScheduleDef {
    ScheduleDef {
        id: name.to_owned(),
        repeat: Repeat::Pattern(pattern),
        template: template_with(name, json!({})),
    }
}

fn every_def(name: &str, every_ms: u64) -> ScheduleDef {
    ScheduleDef {
        id: name.to_owned(),
        repeat: Repeat::EveryMs(every_ms),
        template: template_with(name, json!({})),
    }
}

/// Every scheduler the edition registers, in registration order.
pub fn schedules(config: &ScheduleConfig) -> Vec<ScheduleDef> {
    let cloud = config.edition == Edition::Cloud;
    // Template data is frozen at registration (the handlers ignore it).
    let boot_iso = iso_ms(crate::clock::SystemClock.now_ms());
    let mut out = Vec::with_capacity(48);
    out.push(cron(
        "scanAbandonedCarts",
        every_minute_at_offset("scanAbandonedCarts"),
    ));
    out.push(cron(
        "expireUnpaidOrders",
        every_minute_at_offset("expireUnpaidOrders"),
    ));
    out.push(cron(
        "dispatchProfileSnapshots",
        every_minute_at_offset("dispatchProfileSnapshots"),
    ));
    out.push(ScheduleDef {
        id: "enqueueBroadcast".to_owned(),
        repeat: Repeat::Pattern("* * * * *".to_owned()),
        template: template_with("enqueueBroadcast", json!({ "schedulesAt": boot_iso })),
    });
    out.push(cron("enqueueRecurringBroadcasts", "* * * * *".to_owned()));
    out.push(cron(
        "finalizeBroadcasts",
        every_minute_at_offset("finalizeBroadcasts"),
    ));
    out.push(cron(
        "reconcileBroadcasts",
        every_minute_at_offset("reconcileBroadcasts"),
    ));
    out.push(cron("enqueueMarketingMessages", "* * * * *".to_owned()));
    out.push(cron(
        "dispatchMarketingMessages",
        every_minute_at_offset("dispatchMarketingMessages"),
    ));
    out.push(cron(
        "finalizeMarketingMessages",
        every_minute_at_offset("finalizeMarketingMessages"),
    ));
    out.push(cron("evaluateTriggers", "* * * * *".to_owned()));
    out.push(cron("cleanupTriggers", "0 3 * * *".to_owned()));
    out.push(cron("evaluateDateTimeWebhooks", "* * * * *".to_owned()));
    out.push(cron("cleanupWebhookExecutions", "0 4 * * *".to_owned()));
    out.push(cron("scanSmartDelay", "*/5 * * * *".to_owned()));
    out.push(ScheduleDef {
        id: "scanAppointmentReminders".to_owned(),
        repeat: Repeat::Pattern("*/5 * * * *".to_owned()),
        template: template_with(
            "scanAppointmentReminders",
            json!({ "triggeredAt": boot_iso }),
        ),
    });
    if cloud {
        let every = config.quota_sync_interval_seconds.saturating_mul(1000);
        out.push(every_def("syncUserQuota", every));
        out.push(every_def("reconcileTenants", every));
    }
    out.push(cron("maintainMacPartitions", "0 1 * * *".to_owned()));
    out.push(cron(
        "maintainLeaderboardPartitions",
        "20 1 * * *".to_owned(),
    ));
    out.push(cron(
        "finishOverdueLeaderboards",
        every_minute_at_offset("finishOverdueLeaderboards"),
    ));
    out.push(cron("maintainLeaderboardRankings", "1 * * * *".to_owned()));
    out.push(cron(
        "scanCoexistRuns",
        every_minute_at_offset("scanCoexistRuns"),
    ));
    out.push(cron(
        "scanContactScans",
        every_minute_at_offset("scanContactScans"),
    ));
    out.push(cron(
        "scanAiHandoverBulkRuns",
        every_minute_at_offset("scanAiHandoverBulkRuns"),
    ));
    out.push(cron(
        "reconcileMetaCatalogSyncs",
        every_minute_at_offset("reconcileMetaCatalogSyncs"),
    ));
    out.push(cron("purgeCoexistStaging", "0 * * * *".to_owned()));
    out.push(cron("purgeErrorLogs", "0 3 * * *".to_owned()));
    out.push(cron(
        "redispatchInboundWebhookRequests",
        "*/5 * * * *".to_owned(),
    ));
    out.push(cron("redispatchTemplateJobs", "*/5 * * * *".to_owned()));
    out.push(cron("purgeInboundWebhookRequests", "30 3 * * *".to_owned()));
    out.push(cron(
        "purgeCommentAutomationEvents",
        "15 3 * * *".to_owned(),
    ));
    out.push(cron("purgeWhatsappSignupSessions", "0 * * * *".to_owned()));
    out.push(cron("purgeExpiredConnectSessions", "0 * * * *".to_owned()));
    out.push(cron(
        "purgeWorkspaces",
        format!("*/{} * * * *", config.purge_workspaces_interval_minutes),
    ));
    out.push(cron("clearExpiredSupportAccess", "0 3 * * *".to_owned()));
    out.push(cron("googleAdsHousekeeping", "*/10 * * * *".to_owned()));
    out.push(cron("googleAdsSyncSetups", "30 3 * * *".to_owned()));
    out.push(cron("purgeExpiredOAuthTokens", "0 4 * * *".to_owned()));
    out.push(cron(
        "purgeBroadcasts",
        format!("*/{} * * * *", config.purge_broadcasts_interval_minutes),
    ));
    out.push(cron("purgeAutomationThrottle", "0 * * * *".to_owned()));
    out.push(cron("refreshChannelTokens", "0 2 * * *".to_owned()));
    // Zalo and TikTok tokens live ~24 h: an extra midday run (id differs
    // from the job name).
    out.push(ScheduleDef {
        id: "refreshShortLivedChannelTokens".to_owned(),
        repeat: Repeat::Pattern("0 14 * * *".to_owned()),
        template: template_with(
            "refreshChannelTokens",
            json!({ "channels": ["zalo", "tiktok"] }),
        ),
    });
    if config.edition != Edition::Community {
        out.push(cron(
            "refreshLicense",
            format!("{} * * * *", config.license_refresh_minute),
        ));
    }
    if cloud {
        out.push(cron("unsubscribeExpiredTrials", "0 * * * *".to_owned()));
    }
    out.push(cron("purgeExpiredContactExports", "30 4 * * *".to_owned()));
    out.push(cron("purgeExpiredCallRecordings", "0 5 * * *".to_owned()));
    out.push(cron("sweepStaleWhatsappCalls", "*/5 * * * *".to_owned()));
    out
}

/// `new Date(ms).toISOString()`.
pub(crate) fn iso_ms(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// Scheduler ids the edition must remove (persisted from another edition).
pub fn removed_schedulers(edition: Edition) -> Vec<&'static str> {
    match edition {
        Edition::Cloud => Vec::new(),
        Edition::Enterprise => CLOUD_ONLY_SCHEDULERS.to_vec(),
        Edition::Community => {
            let mut out = CLOUD_ONLY_SCHEDULERS.to_vec();
            out.push("refreshLicense");
            out
        }
    }
}

/// `scheduleQueue.upsertJobScheduler` / `removeJobScheduler`.
pub trait SchedulerStore: Send + Sync {
    fn upsert_job_scheduler<'a>(
        &'a self,
        id: &'a str,
        repeat: &'a Repeat,
        template: &'a ScheduleTemplate,
    ) -> BoxFuture<'a, Result<(), JobsError>>;

    fn remove_job_scheduler<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, JobsError>>;
}

impl SchedulerStore for JobStore {
    fn upsert_job_scheduler<'a>(
        &'a self,
        id: &'a str,
        repeat: &'a Repeat,
        template: &'a ScheduleTemplate,
    ) -> BoxFuture<'a, Result<(), JobsError>> {
        Box::pin(self.upsert_scheduler(QueueName::Schedule, id, repeat, template))
    }

    fn remove_job_scheduler<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(self.remove_scheduler(QueueName::Schedule, id))
    }
}

/// `registerSchedules()`: upserts [`schedules`] and removes
/// [`removed_schedulers`] (S13 edition gating).
pub async fn register_schedules(
    config: &ScheduleConfig,
    store: &dyn SchedulerStore,
) -> Result<(), JobsError> {
    // A scheduler persisted by another edition keeps firing until removed.
    for id in removed_schedulers(config.edition) {
        store.remove_job_scheduler(id).await?;
    }
    for def in schedules(config) {
        store
            .upsert_job_scheduler(&def.id, &def.repeat, &def.template)
            .await?;
    }
    Ok(())
}

/// What the schedule worker start needs from the runtime.
pub trait ScheduleWorkerHost: Send + Sync {
    /// `createQueueWorker(schedule, ...)`.
    fn create_schedule_worker(&self) -> Result<(), JobsError>;
    /// `requestShutdown(reason)`.
    fn request_shutdown(&self, reason: &str);
}

/// `schedule/worker.ts` start: registers the schedulers; a registration
/// error is logged ("Error registering schedules", field `err`) and the
/// worker still starts; no shutdown is requested.
pub async fn start_schedule_worker(
    config: &ScheduleConfig,
    store: &dyn SchedulerStore,
    host: &dyn ScheduleWorkerHost,
) -> Result<(), JobsError> {
    // A registration failure must not take the process down: the consumer
    // still serves the schedulers registered before.
    match register_schedules(config, store).await {
        Ok(()) => tracing::info!("Schedules registered"),
        Err(err) => tracing::error!(err = %err, "Error registering schedules"),
    }
    host.create_schedule_worker()
}

/// A parsed cron pattern: `*`, `N`, `*/N` (also `a-b`, `a-b/N` and lists)
/// per field; 5 fields or 6 (seconds first); evaluated in UTC
/// (DECISIONS P2.6: no time zones).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronPattern {
    seconds: u64,
    minutes: u64,
    hours: u64,
    /// Bits 1..=31.
    days_of_month: u64,
    /// Bits 1..=12.
    months: u64,
    /// Bits 0..=6 (0 = Sunday).
    days_of_week: u64,
    /// The day-of-month field is not `*`-based.
    dom_restricted: bool,
    /// The day-of-week field is not `*`-based.
    dow_restricted: bool,
}

impl CronPattern {
    pub fn parse(pattern: &str) -> Result<CronPattern, JobsError> {
        let fields: Vec<&str> = pattern.split_whitespace().collect();
        let (second, rest) = match fields.len() {
            5 => ("0", &fields[..]),
            6 => (fields[0], &fields[1..]),
            n => {
                return Err(JobsError::Config(format!(
                    "cron pattern {pattern:?} has {n} fields (5 or 6 expected)"
                )));
            }
        };
        let field = |text: &str, min: u32, max: u32| parse_field(pattern, text, min, max);
        let mut days_of_week = field(rest[4], 0, 7)?;
        if days_of_week & (1 << 7) != 0 {
            days_of_week = (days_of_week & !(1 << 7)) | 1;
        }
        Ok(CronPattern {
            seconds: field(second, 0, 59)?,
            minutes: field(rest[0], 0, 59)?,
            hours: field(rest[1], 0, 23)?,
            days_of_month: field(rest[2], 1, 31)?,
            months: field(rest[3], 1, 12)?,
            days_of_week,
            dom_restricted: !rest[2].starts_with('*'),
            dow_restricted: !rest[4].starts_with('*'),
        })
    }

    fn day_matches(&self, day: u32, weekday: u32) -> bool {
        let dom = self.days_of_month & (1 << day) != 0;
        let dow = self.days_of_week & (1 << weekday) != 0;
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }

    /// The first fire time strictly after `after_ms` (UTC), in ms (always a
    /// whole second).
    pub fn next_after(&self, after_ms: i64) -> i64 {
        // Next whole second strictly after `after_ms`.
        let mut t = after_ms.div_euclid(1000) + 1;
        // Bounded: a valid pattern fires within a few years (Feb 29 at worst).
        for _ in 0..1_000_000 {
            let days = t.div_euclid(86_400);
            let secs = t.rem_euclid(86_400);
            let (_, month, day) = civil_from_days(days);
            let weekday = (days + 4).rem_euclid(7) as u32; // 1970-01-01 was a Thursday
            if self.months & (1 << month) == 0 || !self.day_matches(day, weekday) {
                t = (days + 1) * 86_400;
                continue;
            }
            let hour = (secs / 3600) as u32;
            if self.hours & (1 << hour) == 0 {
                t = days * 86_400 + (i64::from(hour) + 1) * 3600;
                continue;
            }
            let minute = ((secs % 3600) / 60) as u32;
            if self.minutes & (1 << minute) == 0 {
                t = days * 86_400 + i64::from(hour) * 3600 + (i64::from(minute) + 1) * 60;
                continue;
            }
            let second = (secs % 60) as u32;
            if self.seconds & (1 << second) == 0 {
                t += 1;
                continue;
            }
            return t * 1000;
        }
        i64::MAX
    }
}

/// One cron field as a bit set over `min..=max`.
fn parse_field(pattern: &str, text: &str, min: u32, max: u32) -> Result<u64, JobsError> {
    let bad = || JobsError::Config(format!("invalid cron field {text:?} in {pattern:?}"));
    let number = |s: &str| -> Result<u32, JobsError> {
        let n: u32 = s.parse().map_err(|_| bad())?;
        if (min..=max).contains(&n) {
            Ok(n)
        } else {
            Err(bad())
        }
    };
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step.parse().map_err(|_| bad())?;
                if step == 0 {
                    return Err(bad());
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (number(a)?, number(b)?)
        } else {
            let n = number(range)?;
            // `N/step` runs from N to the end of the field.
            (n, if step > 1 { max } else { n })
        };
        if start > end {
            return Err(bad());
        }
        let mut v = start;
        while v <= end {
            bits |= 1 << v;
            v += step;
        }
    }
    if bits == 0 { Err(bad()) } else { Ok(bits) }
}

/// Days since 1970-01-01 to `(year, month 1..=12, day 1..=31)` (Howard
/// Hinnant's `civil_from_days`).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `(year, month, day)` to days since 1970-01-01 (`days_from_civil`).
#[cfg(test)]
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2024-01-10T06:00:00Z etc., built with `days_from_civil`.
    fn utc(y: i64, mo: u32, d: u32, h: i64, mi: i64, s: i64) -> i64 {
        (days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s) * 1000
    }

    #[test]
    fn civil_round_trip() {
        for days in [
            -1000_i64, -1, 0, 1, 59, 60, 365, 11_016, 19_723, 20_000, 60_000,
        ] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn next_fire_times_match_cron_parser() {
        // Expected values as cron-parser computes them in UTC.
        let base = utc(2024, 1, 10, 6, 0, 0);
        let cases: [(&str, i64, i64); 12] = [
            ("* * * * *", base, utc(2024, 1, 10, 6, 1, 0)),
            ("* * * * *", base + 1, utc(2024, 1, 10, 6, 1, 0)),
            ("16 * * * * *", base, utc(2024, 1, 10, 6, 0, 16)),
            (
                "16 * * * * *",
                utc(2024, 1, 10, 6, 0, 16),
                utc(2024, 1, 10, 6, 1, 16),
            ),
            (
                "*/5 * * * *",
                utc(2024, 1, 10, 6, 3, 0),
                utc(2024, 1, 10, 6, 5, 0),
            ),
            (
                "*/10 * * * *",
                utc(2024, 1, 10, 6, 59, 59),
                utc(2024, 1, 10, 7, 0, 0),
            ),
            ("0 * * * *", base, utc(2024, 1, 10, 7, 0, 0)),
            ("1 * * * *", base, utc(2024, 1, 10, 6, 1, 0)),
            ("30 3 * * *", base, utc(2024, 1, 11, 3, 30, 0)),
            (
                "0 14 * * *",
                utc(2024, 12, 31, 15, 0, 0),
                utc(2025, 1, 1, 14, 0, 0),
            ),
            ("0 0 29 2 *", base, utc(2024, 2, 29, 0, 0, 0)),
            ("0 0 * * 0", base, utc(2024, 1, 14, 0, 0, 0)),
        ];
        for (pattern, after, expected) in cases {
            let cron = CronPattern::parse(pattern).expect(pattern);
            assert_eq!(cron.next_after(after), expected, "{pattern}");
        }
    }

    #[test]
    fn every_registered_pattern_parses() {
        for edition in [Edition::Community, Edition::Enterprise, Edition::Cloud] {
            let config = ScheduleConfig {
                edition,
                quota_sync_interval_seconds: QUOTA_SYNC_INTERVAL_SECONDS,
                purge_workspaces_interval_minutes: PURGE_WORKSPACES_INTERVAL_MINUTES,
                purge_broadcasts_interval_minutes: PURGE_BROADCASTS_INTERVAL_MINUTES,
                license_refresh_minute: 59,
            };
            for def in schedules(&config) {
                if let Some(p) = def.repeat.pattern() {
                    CronPattern::parse(p).expect(p);
                }
            }
        }
    }

    #[test]
    fn rejects_bad_patterns() {
        for bad in [
            "",
            "* * * *",
            "60 * * * *",
            "*/0 * * * *",
            "a * * * *",
            "* * 0 * *",
        ] {
            assert!(CronPattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn edition_counts() {
        let count = |edition| {
            schedules(&ScheduleConfig {
                edition,
                quota_sync_interval_seconds: 60,
                purge_workspaces_interval_minutes: 30,
                purge_broadcasts_interval_minutes: 5,
                license_refresh_minute: 1,
            })
            .len()
        };
        assert_eq!(count(Edition::Community), 44);
        assert_eq!(count(Edition::Enterprise), 45);
        assert_eq!(count(Edition::Cloud), 48);
    }
}
