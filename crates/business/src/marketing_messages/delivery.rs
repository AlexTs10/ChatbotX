//! `facebookMarketingMessageDeliveryService`
//! (`facebook-marketing-message-delivery/{service,webhook}.ts`): the campaign
//! state machine (stop / resume / send, the worker steps prepare / startDay /
//! dispatch / finalize) and the `marketing_message_*` webhook.

use std::collections::HashMap;
use std::time::Instant;

use chatbotx_db::Db;
use chatbotx_db::repo::facebook_marketing_message as repo;
use chatbotx_db::repo::facebook_marketing_message_audience as audiences;
use chatbotx_db::repo::facebook_marketing_message_recipient::{
    self as recipients, FailRemainingInput,
};
use chatbotx_db::repo::models::{FacebookMarketingMessage, FacebookMarketingMessageRecipient};
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::Value;

use super::graph::{self, ErrorClass, GraphError, INVALID_TOKEN, SendResult};
use super::{content, grant_token, mark_invalid, mm_error, page_token};
use crate::errors::Result;
use crate::tx;

const ACTIVE: &[&str] = &["scheduled", "sending", "waitingNextDay"];
const OPT_IN_TOPIC_TITLE: &str = "Updates and promotions";
const RETRY_DELAY_MS: i64 = 12 * 60 * 60 * 1000;
const SENDS_PER_MINUTE: i64 = 500;
const UNCONFIRMED_GRACE_MS: i64 = 60 * 60 * 1000;
const PREPARE_RETRY_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
const DISPATCH_WALL_MS: u128 = 45_000;
const BACKOFF_MS: i64 = 5 * 60 * 1000;
const LIFETIME_FALLBACK_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const UNKNOWN_ERROR_BREAKER: u32 = 5;
const INSERT_CHUNK: usize = 1000;

fn text(v: &str) -> Sql {
    Sql::Text(v.to_owned())
}
fn int(v: Option<i64>) -> Sql {
    v.map_or(Sql::Null, Sql::Integer)
}
fn opt_text(v: Option<&str>) -> Sql {
    v.map_or(Sql::Null, text)
}

/// `clearedReason` (`statusReason` + `clearedError`).
fn cleared_reason() -> Patch {
    vec![
        ("statusReason", Sql::Null),
        ("statusErrorCode", Sql::Null),
        ("statusErrorMessage", Sql::Null),
    ]
}

async fn transition(
    db: &Db,
    id: i64,
    from: &'static [&'static str],
    patch: Patch,
) -> Result<Option<FacebookMarketingMessage>> {
    tx::write(db, move |tx| Ok(repo::transition(tx, id, from, &patch)?)).await
}

async fn find_by_id(db: &Db, id: i64) -> Result<Option<FacebookMarketingMessage>> {
    tx::read(db, move |conn| Ok(repo::find_by_id(conn, id)?)).await
}

async fn count_outcomes(db: &Db, id: i64) -> Result<recipients::Outcomes> {
    tx::read(db, move |conn| Ok(recipients::count_outcomes(conn, id)?)).await
}

async fn update_recipient(db: &Db, id: i64, patch: Patch) -> Result<()> {
    tx::write(db, move |tx| Ok(recipients::update(tx, id, &patch)?)).await
}

/// `pause`: an active campaign → `paused` (a lost race is a no-op).
pub async fn pause(
    db: &Db,
    id: i64,
    reason: &str,
    code: Option<i64>,
    message: Option<&str>,
) -> Result<()> {
    transition(
        db,
        id,
        ACTIVE,
        vec![
            ("status", text("paused")),
            ("statusReason", text(reason)),
            ("statusErrorCode", int(code)),
            ("statusErrorMessage", opt_text(message)),
        ],
    )
    .await
    .map(drop)
}

async fn fail(
    db: &Db,
    id: i64,
    reason: &str,
    code: Option<i64>,
    message: Option<&str>,
) -> Result<()> {
    transition(
        db,
        id,
        ACTIVE,
        vec![
            ("status", text("failed")),
            ("statusReason", text(reason)),
            ("statusErrorCode", int(code)),
            ("statusErrorMessage", opt_text(message)),
            ("completedAt", Sql::Integer(now_ms())),
        ],
    )
    .await
    .map(drop)
}

/// The token-snapshot error outcome (`classifyFetchError`).
enum Fetch {
    RetryLater(Option<i64>, String),
    GrantInvalid,
    Failed(Option<i64>, String),
}

fn classify_fetch(error: &GraphError, retry_deadline: i64) -> Fetch {
    if error.code == Some(INVALID_TOKEN) {
        return Fetch::GrantInvalid;
    }
    let retryable = match error.code {
        None => error.is_transport(),
        Some(code) => matches!(
            graph::classify(Some(code), None),
            ErrorClass::Transient | ErrorClass::NotReady
        ),
    };
    if retryable && now_ms() < retry_deadline {
        Fetch::RetryLater(error.code, error.message.clone())
    } else {
        Fetch::Failed(error.code, error.message.clone())
    }
}

/// `prepare`: snapshot the subscription tokens into recipient rows and move
/// `scheduled → sending` (only when due and past any backoff).
pub async fn prepare(db: &Db, campaign_row_id: i64) -> Result<()> {
    let Some(campaign) = find_by_id(db, campaign_row_id).await? else {
        return Ok(());
    };
    let now = now_ms();
    if campaign.status != "scheduled"
        || campaign.starts_at.is_some_and(|at| at > now)
        || campaign.next_day_at.is_some_and(|at| at > now)
    {
        return Ok(());
    }
    let ws = campaign.workspace_id;
    let grant = grant_token(db, ws).await?;
    let page = page_token(db, ws, &campaign.page_id).await?;
    let Some(grant) = grant else {
        return pause(db, campaign.id, "grantInvalid", None, None).await;
    };
    let Some((page_access, page_version)) = page else {
        return pause(db, campaign.id, "pageNotConnected", None, None).await;
    };
    let mut custom_audience_id = None;
    if campaign.target_type == "customAudience" {
        let audience = match campaign.audience_id {
            Some(id) => {
                tx::read(db, move |conn| {
                    Ok(audiences::find_for_workspace(conn, id, ws)?)
                })
                .await?
            }
            None => None,
        };
        let Some(audience) = audience else {
            return fail(
                db,
                campaign.id,
                "tokenFetchFailed",
                None,
                Some("Custom audience not found"),
            )
            .await;
        };
        custom_audience_id = Some(audience.custom_audience_id);
    }
    let snapshot = async {
        let (ad_set_id, end_time) = graph::campaign_schedule(&grant, &campaign.campaign_id).await?;
        let timezone = graph::ad_account_timezone(&grant, &campaign.ad_account_id).await?;
        let tokens = graph::notification_tokens(
            &page_access,
            &campaign.page_id,
            custom_audience_id.as_deref(),
            &page_version,
        )
        .await?;
        Ok::<_, GraphError>((ad_set_id, end_time, timezone, tokens))
    }
    .await;
    let (ad_set_id, end_time, timezone, tokens) = match snapshot {
        Ok(found) => found,
        Err(error) => {
            let deadline =
                campaign.starts_at.unwrap_or(campaign.created_at) + PREPARE_RETRY_WINDOW_MS;
            tracing::warn!(campaign_row_id, message = %error.message, "Marketing message token snapshot failed");
            return match classify_fetch(&error, deadline) {
                Fetch::GrantInvalid => {
                    mark_invalid(db, ws).await?;
                    pause(db, campaign.id, "grantInvalid", None, None).await
                }
                Fetch::RetryLater(code, message) => transition(
                    db,
                    campaign.id,
                    &["scheduled"],
                    vec![
                        ("statusErrorCode", int(code)),
                        ("statusErrorMessage", text(&message)),
                        ("nextDayAt", Sql::Integer(now_ms() + BACKOFF_MS)),
                    ],
                )
                .await
                .map(drop),
                Fetch::Failed(code, message) => {
                    fail(db, campaign.id, "tokenFetchFailed", code, Some(&message)).await
                }
            };
        }
    };
    let mut seen = HashMap::new();
    let mut unique = Vec::new();
    for t in tokens {
        if let Some(&at) = seen.get(&t.token) {
            unique[at] = t;
        } else {
            seen.insert(t.token.clone(), unique.len());
            unique.push(t);
        }
    }
    if unique.is_empty() {
        return fail(db, campaign.id, "noTokens", None, None).await;
    }
    let matched: Vec<_> = if campaign.target_type == "optIn" {
        unique
            .iter()
            .filter(|t| t.topic_title.as_deref() == Some(OPT_IN_TOPIC_TITLE))
            .cloned()
            .collect()
    } else {
        unique.clone()
    };
    if matched.is_empty() {
        let mut titles: Vec<String> = Vec::new();
        for t in unique
            .iter()
            .filter_map(|t| t.topic_title.clone())
            .filter(|t| !t.is_empty())
        {
            if !titles.contains(&t) {
                titles.push(t);
            }
        }
        let titles = titles.join(", ");
        return fail(
            db,
            campaign.id,
            "noMatchingTopic",
            None,
            (!titles.is_empty()).then_some(titles.as_str()),
        )
        .await;
    }
    for chunk in matched.chunks(INSERT_CHUNK) {
        let rows: Vec<Patch> = chunk
            .iter()
            .map(|t| {
                vec![
                    ("workspaceId", Sql::Integer(ws)),
                    ("campaignRowId", Sql::Integer(campaign.id)),
                    ("subscriptionToken", text(&t.token)),
                    ("psid", opt_text(t.psid.as_deref())),
                    ("nextEligibleAt", int(t.next_eligible_at)),
                    ("status", text("pending")),
                    ("attempts", Sql::Integer(0)),
                ]
            })
            .collect();
        tx::write(db, move |tx| Ok(recipients::insert_many(tx, &rows)?)).await?;
    }
    let meta_ends_at = end_time.or_else(|| {
        (campaign.budget_type == "lifetime").then_some(campaign.created_at + LIFETIME_FALLBACK_MS)
    });
    let mut patch = vec![
        ("status", text("sending")),
        ("tokenCount", Sql::Integer(matched.len() as i64)),
        ("startedAt", Sql::Integer(now_ms())),
        ("adSetId", text(&ad_set_id)),
        ("metaEndsAt", int(meta_ends_at)),
        ("timezoneName", text(&timezone)),
        ("nextDayAt", Sql::Null),
    ];
    patch.extend(cleared_reason());
    let started = transition(db, campaign.id, &["scheduled"], patch).await?;
    if started.is_some_and(|s| s.budget_type == "daily") {
        start_day(db, campaign.id).await?;
    }
    Ok(())
}

/// `startDay` (daily campaigns): today's target from the delivery estimate.
pub async fn start_day(db: &Db, campaign_row_id: i64) -> Result<()> {
    let Some(campaign) = find_by_id(db, campaign_row_id).await? else {
        return Ok(());
    };
    if campaign.budget_type != "daily"
        || campaign.ad_set_id.is_none()
        || !matches!(campaign.status.as_str(), "sending" | "waitingNextDay")
    {
        return Ok(());
    }
    let Some(grant) = grant_token(db, campaign.workspace_id).await? else {
        return pause(db, campaign.id, "grantInvalid", None, None).await;
    };
    let estimate = graph::delivery_estimate_upper(
        &grant,
        &campaign.ad_account_id,
        &campaign.page_id,
        campaign.budget_minor_units,
    )
    .await;
    let upper = match estimate {
        Ok(upper) => upper,
        Err(error) => {
            tracing::warn!(campaign_row_id, message = %error.message, "Marketing message delivery estimate failed");
            if error.code == Some(INVALID_TOKEN) {
                mark_invalid(db, campaign.workspace_id).await?;
                return pause(
                    db,
                    campaign.id,
                    "grantInvalid",
                    Some(INVALID_TOKEN),
                    Some(&error.message),
                )
                .await;
            }
            let retry = error.code.is_none()
                || matches!(
                    graph::classify(error.code, None),
                    ErrorClass::Transient | ErrorClass::NotReady
                );
            if retry {
                return transition(
                    db,
                    campaign.id,
                    &["sending", "waitingNextDay"],
                    vec![
                        ("status", text("waitingNextDay")),
                        ("nextDayAt", Sql::Integer(now_ms() + BACKOFF_MS)),
                        ("statusErrorCode", int(error.code)),
                        ("statusErrorMessage", text(&error.message)),
                    ],
                )
                .await
                .map(drop);
            }
            return pause(
                db,
                campaign.id,
                "metaBlocker",
                error.code,
                Some(&error.message),
            )
            .await;
        }
    };
    let pending = count_outcomes(db, campaign.id).await?.pending;
    let target = if pending > 0 {
        upper.min(pending).max(1)
    } else {
        0
    };
    transition(
        db,
        campaign.id,
        &["sending", "waitingNextDay"],
        vec![
            ("status", text("sending")),
            ("dayTarget", Sql::Integer(target)),
            ("daySent", Sql::Integer(0)),
            ("nextDayAt", Sql::Null),
            ("statusErrorCode", Sql::Null),
            ("statusErrorMessage", Sql::Null),
        ],
    )
    .await
    .map(drop)
}

/// `endCampaign`: the status first, then the remaining recipients, in one
/// transaction (a lost race leaves the recipients for a resume).
async fn end_campaign(
    db: &Db,
    campaign: &FacebookMarketingMessage,
    status: &'static str,
    reason: &'static str,
    code: Option<i64>,
    message: String,
    now: i64,
) -> Result<()> {
    let id = campaign.id;
    tx::write(db, move |tx| {
        let failed = status == "failed";
        let row = repo::transition(
            tx,
            id,
            &["sending"],
            &vec![
                ("status", text(status)),
                ("statusReason", text(reason)),
                (
                    "statusErrorCode",
                    if failed { int(code) } else { Sql::Null },
                ),
                (
                    "statusErrorMessage",
                    if failed { text(&message) } else { Sql::Null },
                ),
                ("completedAt", Sql::Integer(now)),
            ],
        )?;
        if row.is_none() {
            return Ok(());
        }
        recipients::fail_remaining(
            tx,
            &FailRemainingInput {
                campaign_row_id: id,
                statuses: Some(vec!["pending".to_owned()]),
                error_code: code,
                error_message: message.clone(),
                error_source: Some("send".to_owned()),
                now,
            },
        )?;
        recipients::fail_remaining(
            tx,
            &FailRemainingInput {
                campaign_row_id: id,
                statuses: Some(vec!["unconfirmed".to_owned()]),
                error_code: None,
                error_message: "outcomeUnknown".to_owned(),
                error_source: Some("send".to_owned()),
                now,
            },
        )?;
        Ok(())
    })
    .await
}

/// `nextLocalMidnight`.
fn next_local_midnight(now: i64, timezone: &str) -> i64 {
    use chrono::{Duration as Days, TimeZone};
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let Some(local) = tz.timestamp_millis_opt(now).single() else {
        return now + 86_400_000;
    };
    let date = local.date_naive() + Days::days(1);
    let midnight = date.and_hms_opt(0, 0, 0).unwrap_or_default();
    tz.from_local_datetime(&midnight)
        .earliest()
        .map_or(now + 86_400_000, |d| d.timestamp_millis())
}

async fn move_to_waiting_next_day(db: &Db, campaign: &FacebookMarketingMessage) -> Result<()> {
    let at = next_local_midnight(now_ms(), campaign.timezone_name.as_deref().unwrap_or("UTC"));
    transition(
        db,
        campaign.id,
        &["sending"],
        vec![
            ("status", text("waitingNextDay")),
            ("nextDayAt", Sql::Integer(at)),
        ],
    )
    .await
    .map(drop)
}

/// `DispatchOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    DayDone,
    Paused,
    Ended,
}

/// What the batch ends with (`terminal`).
enum Terminal {
    Pause(&'static str, Option<i64>, String),
    End(&'static str, &'static str, Option<i64>, String),
}

/// `remainingQuota` of the owner: the tighter of the lifetime and monthly
/// bot-message slots (`None` = unlimited).
async fn remaining_quota(db: &Db, workspace_id: i64) -> Result<Option<i64>> {
    use crate::quota_enforcement::dual_remaining_slots;
    use crate::user_quota::QuotaMetric;
    tx::read(db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension;
        let owner: Option<Option<i64>> = conn
            .query_row(
                "SELECT \"ownerId\" FROM \"Workspace\" WHERE \"id\" = ?1",
                [workspace_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(owner) = owner.flatten() else {
            return Ok(None);
        };
        let monthly = dual_remaining_slots(conn, owner, QuotaMetric::MonthlyBotMessages)?;
        let total = dual_remaining_slots(conn, owner, QuotaMetric::BotMessages)?;
        Ok(match (monthly, total) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        })
    })
    .await
}

/// `recordMarketingMessageUsage`: the three bot-message increments.
async fn record_usage(db: &Db, workspace_id: i64, count: i64) -> Result<()> {
    use crate::quota_enforcement::increment_by;
    use crate::user_quota::QuotaMetric;
    use crate::workspace_usage::{WorkspaceUsageMetric, adjust};
    tx::write(db, move |tx| {
        let owner: i64 = tx.query_row(
            "SELECT \"ownerId\" FROM \"Workspace\" WHERE \"id\" = ?1",
            [workspace_id],
            |r| r.get(0),
        )?;
        increment_by(tx, owner, QuotaMetric::BotMessages, count)?;
        increment_by(tx, owner, QuotaMetric::MonthlyBotMessages, count)?;
        adjust(tx, workspace_id, WorkspaceUsageMetric::BotMessages, count)?;
        Ok(())
    })
    .await
}

fn failed_patch(code: Option<i64>, message: &str, now: i64) -> Patch {
    vec![
        ("status", text("failed")),
        ("errorCode", int(code)),
        ("errorMessage", text(message)),
        ("errorSource", text("send")),
        ("failedAt", Sql::Integer(now)),
    ]
}

/// `dispatchBatch`: one minute's worth of due recipients of a `sending`
/// campaign. The claim (pending → unconfirmed) is the double-send guard; an
/// ambiguous send stays `unconfirmed`, never `pending`.
pub async fn dispatch_batch(db: &Db, campaign_row_id: i64) -> Result<(i64, Outcome)> {
    let Some(campaign) = find_by_id(db, campaign_row_id).await? else {
        return Ok((0, Outcome::Continue));
    };
    if campaign.status != "sending" {
        return Ok((0, Outcome::Continue));
    }
    let id = campaign.id;
    let ws = campaign.workspace_id;
    if crate::broadcast::send::is_blocked_workspace(db, ws).await? {
        pause(db, id, "workspaceBlocked", None, None).await?;
        return Ok((0, Outcome::Paused));
    }
    let quota = remaining_quota(db, ws).await?;
    if quota.is_some_and(|q| q <= 0) {
        pause(db, id, "quotaReached", None, None).await?;
        return Ok((0, Outcome::Paused));
    }
    let Some((_, page_version)) = page_token(db, ws, &campaign.page_id).await? else {
        pause(db, id, "pageNotConnected", None, None).await?;
        return Ok((0, Outcome::Paused));
    };
    let Some(grant) = grant_token(db, ws).await? else {
        pause(db, id, "grantInvalid", None, None).await?;
        return Ok((0, Outcome::Paused));
    };
    let is_daily = campaign.budget_type == "daily";
    let mut limit = SENDS_PER_MINUTE;
    if is_daily {
        limit = limit.min(campaign.day_target.unwrap_or(0) - campaign.day_sent);
    }
    if let Some(q) = quota {
        limit = limit.min(q);
    }
    if limit <= 0 {
        if count_outcomes(db, id).await?.pending == 0 {
            return Ok((0, Outcome::Continue));
        }
        move_to_waiting_next_day(db, &campaign).await?;
        return Ok((0, Outcome::DayDone));
    }
    let Some(parsed) = content::parse(&campaign.content.0) else {
        end_campaign(
            db,
            &campaign,
            "failed",
            "contentRejected",
            None,
            "The stored message content is invalid".to_owned(),
            now_ms(),
        )
        .await?;
        return Ok((0, Outcome::Ended));
    };
    let message = content::build_payload(&parsed);
    let now = now_ms();
    let started = Instant::now();
    let due = tx::read(db, move |conn| {
        Ok(recipients::list_due(conn, id, now, limit)?)
    })
    .await?;

    let mut sends = 0i64;
    let mut outcome = Outcome::Continue;
    let mut terminal: Option<Terminal> = None;
    let mut warning: Option<(Option<i64>, String)> = None;
    let mut content_locked = campaign.first_sent_at.is_some();
    let mut streak: Option<(Option<i64>, u32)> = None;
    let mut failure = None;

    for recipient in due {
        if started.elapsed().as_millis() >= DISPATCH_WALL_MS {
            break;
        }
        let rid = recipient.id;
        let claimed: Option<FacebookMarketingMessageRecipient> = match tx::write(db, move |tx| {
            Ok(recipients::claim_for_send(tx, rid, now_ms())?)
        })
        .await
        {
            Ok(c) => c,
            Err(e) => {
                failure = Some(e);
                break;
            }
        };
        let Some(claimed) = claimed else {
            if find_by_id(db, id)
                .await?
                .is_none_or(|c| c.status != "sending")
            {
                break;
            }
            continue;
        };
        if !content_locked {
            tx::write(db, move |tx| Ok(repo::set_first_sent_at(tx, id)?)).await?;
            content_locked = true;
        }
        let result = graph::send_marketing_message(
            &grant,
            &campaign.ad_account_id,
            &campaign.campaign_id,
            &recipient.subscription_token,
            &message,
            &page_version,
        )
        .await;
        let revert: Patch = vec![
            ("status", text("pending")),
            ("attempts", Sql::Integer(claimed.attempts - 1)),
            ("sentAt", Sql::Null),
        ];
        let (code, status, error_message) = match result {
            SendResult::Ok(tracking) => {
                sends += 1;
                streak = None;
                let patch = vec![
                    ("status", text("sent")),
                    ("trackingId", text(&tracking)),
                    ("errorCode", Sql::Null),
                    ("errorMessage", Sql::Null),
                ];
                tx::write(db, move |tx| Ok(recipients::settle_send(tx, rid, &patch)?)).await?;
                continue;
            }
            SendResult::Uncertain(message) => {
                sends += 1;
                streak = None;
                let patch = vec![
                    ("errorCode", Sql::Null),
                    ("errorMessage", text(&message)),
                    ("errorSource", text("send")),
                ];
                tx::write(db, move |tx| Ok(recipients::settle_send(tx, rid, &patch)?)).await?;
                continue;
            }
            SendResult::Meta {
                code,
                status,
                message,
            } => (code, status, message),
        };
        let class = graph::classify(code, Some(status));
        if class != ErrorClass::Unknown {
            streak = None;
        }
        let blocker = |m: &str| Terminal::Pause("metaBlocker", code, m.to_owned());
        match class {
            ErrorClass::DailyBudget => {
                update_recipient(db, rid, revert).await?;
                if is_daily {
                    outcome = Outcome::DayDone;
                } else {
                    outcome = Outcome::Paused;
                    terminal = Some(blocker(&error_message));
                }
            }
            ErrorClass::NotReady | ErrorClass::Transient => {
                update_recipient(db, rid, revert).await?;
                warning = Some((code, error_message));
            }
            ErrorClass::LifetimeBudget => {
                update_recipient(db, rid, revert).await?;
                outcome = Outcome::Ended;
                terminal = Some(Terminal::End(
                    "completed",
                    "lifetimeBudgetReached",
                    code,
                    error_message,
                ));
            }
            ErrorClass::ContentInvalid => {
                update_recipient(db, rid, revert).await?;
                outcome = Outcome::Ended;
                terminal = Some(Terminal::End(
                    "failed",
                    "contentRejected",
                    code,
                    error_message,
                ));
            }
            ErrorClass::CampaignInactive => {
                update_recipient(db, rid, revert).await?;
                if campaign.meta_ends_at.is_some_and(|at| at <= now) {
                    outcome = Outcome::Ended;
                    terminal = Some(Terminal::End(
                        "completed",
                        "campaignEnded",
                        code,
                        error_message,
                    ));
                } else {
                    outcome = Outcome::Paused;
                    terminal = Some(blocker(&error_message));
                }
            }
            ErrorClass::CampaignBlocker => {
                update_recipient(db, rid, revert).await?;
                outcome = Outcome::Paused;
                if code == Some(INVALID_TOKEN) {
                    mark_invalid(db, ws).await?;
                    terminal = Some(Terminal::Pause("grantInvalid", code, error_message));
                } else {
                    terminal = Some(blocker(&error_message));
                }
            }
            ErrorClass::RecipientTiming => {
                let patch = if claimed.attempts - 1 == 0 {
                    let retry_at = recipient
                        .next_eligible_at
                        .filter(|&at| at > now)
                        .unwrap_or(now + RETRY_DELAY_MS);
                    vec![
                        ("status", text("pending")),
                        ("attempts", Sql::Integer(claimed.attempts)),
                        ("nextAttemptAt", Sql::Integer(retry_at)),
                        ("errorCode", int(code)),
                        ("errorMessage", text(&error_message)),
                        ("errorSource", text("send")),
                    ]
                } else {
                    failed_patch(code, &error_message, now)
                };
                update_recipient(db, rid, patch).await?;
                continue;
            }
            ErrorClass::RecipientPermanent => {
                update_recipient(db, rid, failed_patch(code, &error_message, now)).await?;
                continue;
            }
            ErrorClass::Unknown => {
                let count = match streak {
                    Some((c, n)) if c == code => n + 1,
                    _ => 1,
                };
                streak = Some((code, count));
                if count < UNKNOWN_ERROR_BREAKER {
                    update_recipient(db, rid, failed_patch(code, &error_message, now)).await?;
                    continue;
                }
                update_recipient(db, rid, revert).await?;
                outcome = Outcome::Paused;
                terminal = Some(blocker(&error_message));
            }
        }
        break;
    }
    // `finally`: the sends of this batch count even when it stopped early.
    if sends > 0 {
        tx::write(db, move |tx| Ok(repo::increment_day_sent(tx, id, sends)?)).await?;
        if let Err(error) = record_usage(db, ws, sends).await {
            tracing::error!(%error, campaign_row_id, count = sends, "Marketing message usage increment failed");
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    match terminal {
        Some(Terminal::Pause(reason, code, message)) => {
            pause(db, id, reason, code, Some(&message)).await?;
        }
        Some(Terminal::End(status, reason, code, message)) => {
            end_campaign(db, &campaign, status, reason, code, message, now).await?;
        }
        None if outcome == Outcome::DayDone => move_to_waiting_next_day(db, &campaign).await?,
        None => {
            if let Some((code, message)) = warning {
                transition(
                    db,
                    id,
                    ACTIVE,
                    vec![
                        ("statusErrorCode", int(code)),
                        ("statusErrorMessage", text(&message)),
                    ],
                )
                .await?;
            } else if sends > 0
                && (campaign.status_error_code.is_some() || campaign.status_error_message.is_some())
            {
                transition(
                    db,
                    id,
                    &["sending"],
                    vec![
                        ("statusErrorCode", Sql::Null),
                        ("statusErrorMessage", Sql::Null),
                    ],
                )
                .await?;
            }
        }
    }
    Ok((sends, outcome))
}

/// `finalize`: completes a `sending` campaign once nothing is pending;
/// unconfirmed rows get a grace window for the webhook.
pub async fn finalize(db: &Db, campaign_row_id: i64) -> Result<()> {
    let Some(campaign) = find_by_id(db, campaign_row_id).await? else {
        return Ok(());
    };
    if campaign.status != "sending" {
        return Ok(());
    }
    let counts = count_outcomes(db, campaign_row_id).await?;
    if counts.pending > 0 {
        return Ok(());
    }
    let now = now_ms();
    if counts.unconfirmed > 0 {
        tx::write(db, move |tx| {
            Ok(recipients::fail_stale_unconfirmed(
                tx,
                campaign_row_id,
                now - UNCONFIRMED_GRACE_MS,
                now,
            )?)
        })
        .await?;
        let after = count_outcomes(db, campaign_row_id).await?;
        if after.pending > 0 || after.unconfirmed > 0 {
            return Ok(());
        }
    }
    let mut patch = vec![
        ("status", text("completed")),
        ("completedAt", Sql::Integer(now)),
    ];
    patch.extend(cleared_reason());
    transition(db, campaign_row_id, &["sending"], patch)
        .await
        .map(drop)
}

async fn load(db: &Db, workspace_id: i64, id: &str) -> Result<FacebookMarketingMessage> {
    super::campaigns::find(db, workspace_id, id)
        .await?
        .ok_or_else(|| mm_error("mmNotFound"))
}

/// `stop`: never touches Meta.
pub async fn stop(db: &Db, workspace_id: i64, id: &str) -> Result<FacebookMarketingMessage> {
    let campaign = load(db, workspace_id, id).await?;
    let mut patch = vec![("status", text("stopped"))];
    patch.extend(cleared_reason());
    transition(
        db,
        campaign.id,
        &["scheduled", "sending", "waitingNextDay", "paused"],
        patch,
    )
    .await?
    .ok_or_else(|| mm_error("mmInvalidState"))
}

/// `resume`: before the snapshot → `scheduled` now; daily → `waitingNextDay`
/// now (re-estimated by startDay); lifetime → `sending`.
pub async fn resume(db: &Db, workspace_id: i64, id: &str) -> Result<FacebookMarketingMessage> {
    let campaign = load(db, workspace_id, id).await?;
    if !matches!(campaign.status.as_str(), "paused" | "stopped") {
        return Err(mm_error("mmInvalidState"));
    }
    let now = now_ms();
    if campaign.meta_ends_at.is_some_and(|at| at <= now) {
        return Err(mm_error("mmCampaignEnded"));
    }
    if grant_token(db, workspace_id).await?.is_none() {
        return Err(mm_error("mmGrantInvalid"));
    }
    let mut patch: Patch = if campaign.token_count.is_none() {
        vec![
            ("status", text("scheduled")),
            ("startsAt", Sql::Integer(now)),
            ("nextDayAt", Sql::Null),
        ]
    } else if campaign.budget_type == "daily" {
        vec![
            ("status", text("waitingNextDay")),
            ("nextDayAt", Sql::Integer(now)),
        ]
    } else {
        vec![("status", text("sending"))]
    };
    patch.push(("resumeCount", Sql::Integer(campaign.resume_count + 1)));
    patch.extend(cleared_reason());
    transition(db, campaign.id, &["paused", "stopped"], patch)
        .await?
        .ok_or_else(|| mm_error("mmInvalidState"))
}

/// `send`: manual start of a `notSent` campaign; the Meta end time is read
/// first (a non-token failure keeps the stored one).
pub async fn send(db: &Db, workspace_id: i64, id: &str) -> Result<FacebookMarketingMessage> {
    let campaign = load(db, workspace_id, id).await?;
    if campaign.status != "notSent" {
        return Err(mm_error("mmInvalidState"));
    }
    let Some(grant) = grant_token(db, workspace_id).await? else {
        return Err(mm_error("mmGrantInvalid"));
    };
    let mut meta_ends_at = campaign.meta_ends_at;
    match graph::campaign_schedule(&grant, &campaign.campaign_id).await {
        Ok((_, end)) => meta_ends_at = end,
        Err(error) if error.code == Some(INVALID_TOKEN) => {
            mark_invalid(db, workspace_id).await?;
            return Err(mm_error("mmGrantInvalid"));
        }
        Err(error) => {
            tracing::warn!(campaign_row_id = campaign.id, message = %error.message, "Could not read the Meta campaign schedule before send");
        }
    }
    if meta_ends_at.is_some_and(|at| at <= now_ms()) {
        return Err(mm_error("mmCampaignEnded"));
    }
    let mut patch = vec![
        ("status", text("scheduled")),
        ("startsAt", Sql::Integer(now_ms())),
        ("metaEndsAt", int(meta_ends_at)),
    ];
    patch.extend(cleared_reason());
    transition(db, campaign.id, &["notSent"], patch)
        .await?
        .ok_or_else(|| mm_error("mmInvalidState"))
}

/// `MarketingMessageWebhookEvent`.
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookEvent {
    pub field: String,
    pub campaign_id: String,
    pub subscription_token: String,
    pub tracking_id: Option<String>,
    pub psid: Option<String>,
    pub mid: Option<String>,
    /// `delivery_failed` only.
    pub error_message: Option<String>,
    pub timestamp: i64,
}

fn id_string(value: &Value) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::String(s) if !s.is_empty() => Some(Some(s.clone())),
        Value::Number(n) => Some(Some(n.to_string())),
        _ => None,
    }
}

/// `parseMarketingMessageWebhook`: `None` for an unknown field or an invalid value.
pub fn parse_webhook(field: &str, value: &Value) -> Option<WebhookEvent> {
    if !matches!(
        field,
        "marketing_message_echoes"
            | "marketing_message_deliveries"
            | "marketing_message_delivery_failed"
    ) || !value.is_object()
    {
        return None;
    }
    let required = |key: &str| id_string(&value[key]).flatten();
    let optional = |key: &str| match value.get(key) {
        None => Some(None),
        Some(v) => id_string(v),
    };
    let campaign_id = required("message_id")?;
    let subscription_token = required("messenger_subscription_token")?;
    let ts = match &value["timestamp"] {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if !s.is_empty() => chatbotx_core::js::to_number(s),
        _ => return None,
    };
    if !ts.is_finite() || ts <= 0.0 {
        return None;
    }
    let timestamp = (if ts < 1e12 { ts * 1000.0 } else { ts }) as i64;
    let (tracking_id, psid, mid) = (
        optional("marketing_message_tracking_id")?,
        optional("recipient_id")?,
        optional("mid")?,
    );
    let error_message = match value.get("error_message") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return None,
    };
    let failed = field == "marketing_message_delivery_failed";
    Some(WebhookEvent {
        field: field.to_owned(),
        campaign_id,
        subscription_token,
        tracking_id: (!failed).then_some(tracking_id).flatten(),
        psid: (!failed).then_some(psid).flatten(),
        mid: (!failed).then_some(mid).flatten(),
        error_message: failed.then(|| {
            error_message
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "deliveryFailed".to_owned())
        }),
        timestamp,
    })
}

fn is_outcome_unknown(r: &FacebookMarketingMessageRecipient) -> bool {
    r.status == "failed"
        && r.error_code.is_none()
        && r.error_message.as_deref() == Some("outcomeUnknown")
}

fn webhook_patch(r: &FacebookMarketingMessageRecipient, event: &WebhookEvent) -> Patch {
    if let Some(message) = &event.error_message {
        return vec![
            ("status", text("failed")),
            ("errorSource", text("webhook")),
            ("errorCode", Sql::Null),
            ("errorMessage", text(message)),
            ("failedAt", Sql::Integer(event.timestamp)),
        ];
    }
    let mut patch = Vec::new();
    if let Some(psid) = event.psid.as_deref().filter(|_| r.psid.is_none()) {
        patch.push(("psid", text(psid)));
    }
    if let Some(mid) = event.mid.as_deref().filter(|_| r.mid.is_none()) {
        patch.push(("mid", text(mid)));
    }
    if let Some(t) = event
        .tracking_id
        .as_deref()
        .filter(|_| r.tracking_id.is_none())
    {
        patch.push(("trackingId", text(t)));
    }
    let promotable =
        matches!(r.status.as_str(), "pending" | "unconfirmed") || is_outcome_unknown(r);
    let clear = [
        ("errorCode", Sql::Null),
        ("errorMessage", Sql::Null),
        ("errorSource", Sql::Null),
        ("failedAt", Sql::Null),
    ];
    if event.field == "marketing_message_echoes" {
        if promotable {
            patch.push(("status", text("sent")));
            if r.status == "failed" {
                patch.extend(clear);
            }
        }
    } else if promotable || r.status == "sent" {
        patch.push(("status", text("delivered")));
        patch.push(("deliveredAt", Sql::Integer(event.timestamp)));
        if r.status == "failed" {
            patch.extend(clear);
        }
    }
    patch
}

/// `applyMarketingMessageWebhook`: the campaign of an applied event.
pub async fn apply_webhook(
    db: &Db,
    event: &WebhookEvent,
) -> Result<Option<FacebookMarketingMessage>> {
    let event = event.clone();
    tx::write(db, move |tx| {
        let Some(campaign) = repo::find_by_campaign_id(tx, &event.campaign_id)? else {
            tracing::debug!(field = %event.field, "marketing message webhook for unknown campaign");
            return Ok(None);
        };
        let mut recipient = None;
        if let Some(t) = event.tracking_id.as_deref() {
            recipient = recipients::find_by_tracking_id(tx, t)?.filter(|r| r.campaign_row_id == campaign.id);
        }
        if recipient.is_none() {
            recipient = recipients::find_by_campaign_token(tx, campaign.id, &event.subscription_token)?;
        }
        let Some(recipient) = recipient else {
            tracing::debug!(campaign_row_id = campaign.id, field = %event.field, "marketing message webhook for unknown token");
            return Ok(None);
        };
        let patch = webhook_patch(&recipient, &event);
        if !patch.is_empty() {
            recipients::update(tx, recipient.id, &patch)?;
        }
        Ok(Some(campaign))
    })
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_the_three_fields_and_rejects_bad_values() {
        let echo = parse_webhook(
            "marketing_message_echoes",
            &json!({ "message_id": 12, "messenger_subscription_token": "t", "timestamp": 1_700_000_000, "recipient_id": "p", "mid": "m" }),
        )
        .unwrap();
        assert_eq!(echo.campaign_id, "12");
        assert_eq!(echo.timestamp, 1_700_000_000_000);
        assert_eq!(echo.psid.as_deref(), Some("p"));
        let failed = parse_webhook(
            "marketing_message_delivery_failed",
            &json!({ "message_id": "1", "messenger_subscription_token": "t", "timestamp": "1700000000000" }),
        )
        .unwrap();
        assert_eq!(failed.error_message.as_deref(), Some("deliveryFailed"));
        assert!(parse_webhook("other", &json!({})).is_none());
        assert!(
            parse_webhook(
                "marketing_message_deliveries",
                &json!({ "message_id": "1", "messenger_subscription_token": "t", "timestamp": 0 })
            )
            .is_none()
        );
    }

    #[test]
    fn next_midnight_is_in_the_campaign_zone() {
        // 2024-01-01T23:30:00Z is 2024-01-02 06:30 in Bangkok (UTC+7).
        let now = 1_704_151_800_000;
        assert_eq!(next_local_midnight(now, "Asia/Bangkok"), 1_704_214_800_000);
        assert_eq!(next_local_midnight(now, "UTC"), 1_704_153_600_000);
    }
}
