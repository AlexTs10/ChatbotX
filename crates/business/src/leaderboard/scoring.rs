//! `leaderboard/scoring.ts`: the one score write path and the rule, flow
//! and referral awards.

use chatbotx_db::Db;
use chatbotx_db::repo::leaderboard_participant::{
    self as participants, CHANGE_REJECTED, ScoreChangeInput, ScoreChangeResult, ScoreLimits,
};
use chatbotx_db::repo::models::Leaderboard;
use chatbotx_db::repo::{leaderboard, new_id, now_ms};
use chatbotx_db::rusqlite::Connection;
use serde_json::Value;

use super::{FOLLOWER_CHECK_PLATFORMS, LATE_EVENT_GRACE_MS, MAX_SCORE, day_key, event_id};
use super::{is_within_window, rule_of};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// Statuses a rule award may be written in (finished: only before rewards).
const RULE_STATUSES: &[&str] = &["active", "finished"];
/// Bot scoring (flow step, referral) needs a running campaign.
const ACTIVE_ONLY: &[&str] = &["active"];
/// `FLOW_DEDUP_LOOKBACK_MS`.
const FLOW_DEDUP_LOOKBACK_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// A score change without the leaderboard fields (`LeaderboardScoreChangeInput`).
#[derive(Debug, Clone, Default)]
pub struct Change {
    pub contact_id: i64,
    pub delta: i64,
    pub source: &'static str,
    pub rule: Option<String>,
    pub breakdown_key: Option<String>,
    pub breakdown_step: Option<i64>,
    pub source_ref: Option<String>,
    pub event_id: i64,
    pub event_at: i64,
    pub limits: Option<ScoreLimits>,
    pub referrer_contact_id: Option<i64>,
    pub allowed_statuses: &'static [&'static str],
    pub dedup_since: Option<i64>,
}

/// What runs inside the transaction when the change applied (the audit row).
pub type AfterApply = Box<dyn FnOnce(&Connection) -> Result<()> + Send>;

/// `applyLeaderboardScoreChange`: the status guard, idempotency, daily cap
/// and cooldown in one write transaction. A rejected change rolls back.
pub async fn apply_change(
    db: &Db,
    board: &Leaderboard,
    change: Change,
    after_apply: Option<AfterApply>,
) -> Result<ScoreChangeResult> {
    let input = ScoreChangeInput {
        leaderboard_id: board.id,
        workspace_id: board.workspace_id,
        contact_id: change.contact_id,
        delta: change.delta,
        source: change.source.to_owned(),
        rule: change.rule,
        breakdown_key: change.breakdown_key,
        breakdown_step: change.breakdown_step,
        source_ref: change.source_ref,
        event_id: change.event_id,
        event_at: change.event_at,
        limits: change.limits,
        referrer_contact_id: change.referrer_contact_id,
        max_score: MAX_SCORE,
        allowed_statuses: change
            .allowed_statuses
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        dedup_since: change.dedup_since,
        now: now_ms(),
    };
    let outcome = tx::write(db, move |conn| {
        let result = participants::apply_change_in_transaction(conn, &input)?;
        if matches!(result, ScoreChangeResult::Applied { .. })
            && let Some(after) = after_apply
        {
            after(conn)?;
        }
        Ok(result)
    })
    .await;
    match outcome {
        Err(BusinessError::Repo(error)) if error.code() == Some(CHANGE_REJECTED) => {
            Ok(ScoreChangeResult::Rejected)
        }
        other => other,
    }
}

/// `{ status, ... }` of a change result.
pub fn result_json(result: &ScoreChangeResult) -> Value {
    match result {
        ScoreChangeResult::Duplicate => serde_json::json!({ "status": "duplicate" }),
        ScoreChangeResult::Rejected => serde_json::json!({ "status": "rejected" }),
        ScoreChangeResult::Closed => serde_json::json!({ "status": "closed" }),
        ScoreChangeResult::Applied { score, is_new, .. } => {
            serde_json::json!({ "status": "applied", "score": score, "isNew": is_new })
        }
    }
}

/// `LeaderboardAwardInput` (the `award` / `reverse` job data).
#[derive(Debug, Clone, Default)]
pub struct Award {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub inbox_id: Option<i64>,
    pub minigame_id: Option<i64>,
    pub rule: String,
    pub source_ref: String,
    pub event_at: i64,
    /// `LeaderboardAwardContext`: `postId`, `textLength`, `postPublishedAt`, `chatId`.
    pub context: Value,
}

/// `checkFollower`: whether the contact follows the host account.
pub trait FollowerCheck: Send + Sync {
    fn follows(&self, board: &Leaderboard) -> crate::events::BoxFuture<'_, bool>;
}

/// The scorable leaderboards of the event's minigame or inbox.
async fn scorable(
    db: &Db,
    inbox_id: Option<i64>,
    minigame_id: Option<i64>,
) -> Result<Vec<Leaderboard>> {
    let since = now_ms() - LATE_EVENT_GRACE_MS;
    tx::read(db, move |conn| {
        Ok(match (minigame_id, inbox_id) {
            (Some(id), _) => leaderboard::find_scorable_for_minigame(conn, id, since)?,
            (None, Some(id)) => leaderboard::find_scorable_for_inbox(conn, id, since)?,
            _ => Vec::new(),
        })
    })
    .await
}

fn ids(value: &Value) -> Vec<i64> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_i64()))
                .collect()
        })
        .unwrap_or_default()
}

fn has_str(list: &Value, needle: Option<&str>) -> bool {
    needle.is_some_and(|n| {
        list.as_array()
            .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(n)))
    })
}

/// `skipReason`: why a live leaderboard does not score the event.
async fn skip_reason(
    db: &Db,
    board: &Leaderboard,
    input: &Award,
    follower: Option<&dyn FollowerCheck>,
) -> Result<Option<&'static str>> {
    let Some(rule) = rule_of(&board.rules.0, &input.rule).filter(|r| r.enabled && r.points > 0)
    else {
        return Ok(Some("ruleDisabled"));
    };
    if !is_within_window(board, input.event_at) {
        return Ok(Some("outsideWindow"));
    }
    let context = &input.context;
    let scope = &board.scope.0;
    if matches!(input.rule.as_str(), "comments" | "liveComments")
        && scope["postScope"] == "specific"
        && !has_str(&scope["postIds"], context["postId"].as_str())
    {
        return Ok(Some("postNotInScope"));
    }
    if input.rule == "comments"
        && rule.min_chars.is_some_and(|min| min > 0)
        && context["textLength"].as_i64().unwrap_or(0) < rule.min_chars.unwrap_or(0)
    {
        return Ok(Some("tooShort"));
    }
    if matches!(input.rule.as_str(), "joinGroup" | "groupDiscussion")
        && !has_str(&scope["telegramChatIds"], context["chatId"].as_str())
    {
        return Ok(Some("chatNotInScope"));
    }
    let tags = ids(&board.tag_ids.0);
    if !tags.is_empty() {
        let contact = input.contact_id;
        let tagged = tx::read(db, move |conn| {
            Ok(participants::contact_has_any_tag(conn, contact, &tags)?)
        })
        .await?;
        if !tagged {
            return Ok(Some("missingTag"));
        }
    }
    if board.followers_only
        && FOLLOWER_CHECK_PLATFORMS.contains(&board.platform.as_str())
        && let Some(check) = follower
        && !check.follows(board).await
    {
        return Ok(Some("notFollower"));
    }
    Ok(None)
}

/// `rulePoints`: the rule points plus the golden-hour bonus.
fn rule_points(board: &Leaderboard, input: &Award) -> i64 {
    let rule = rule_of(&board.rules.0, &input.rule).unwrap_or_default();
    let mut points = rule.points;
    if input.rule == "comments"
        && let (Some(bonus), Some(minutes)) = (rule.bonus_points, rule.bonus_minutes)
        && bonus > 0
        && minutes > 0
        && let Some(published) = input.context["postPublishedAt"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
    {
        let age_minutes = (input.event_at - published.timestamp_millis()) as f64 / 60_000.0;
        if age_minutes >= 0.0 && age_minutes <= minutes as f64 {
            points += bonus;
        }
    }
    points
}

/// One leaderboard's outcome: `Ok(result)` or `Err(skip reason)`.
pub type AwardOutcome = (i64, std::result::Result<ScoreChangeResult, &'static str>);

/// `award`: the rule on every live leaderboard of the inbox (or minigame).
pub async fn award(
    db: &Db,
    input: &Award,
    follower: Option<&dyn FollowerCheck>,
) -> Result<Vec<AwardOutcome>> {
    let mut outcomes = Vec::new();
    for board in scorable(db, input.inbox_id, input.minigame_id).await? {
        if board.workspace_id != input.workspace_id {
            continue;
        }
        if let Some(reason) = skip_reason(db, &board, input, follower).await? {
            outcomes.push((board.id, Err(reason)));
            continue;
        }
        let rule = rule_of(&board.rules.0, &input.rule).unwrap_or_default();
        let board_id = board.id.to_string();
        let change = Change {
            contact_id: input.contact_id,
            delta: rule_points(&board, input),
            source: "rule",
            rule: Some(input.rule.clone()),
            source_ref: Some(input.source_ref.clone()),
            event_id: event_id(&[&board_id, &input.rule, &input.source_ref]),
            event_at: input.event_at,
            limits: Some(ScoreLimits {
                day_bucket: day_key(now_ms(), &board.reset_time),
                max_per_day: rule.max_per_day,
                cooldown_seconds: board.cooldown_seconds,
            }),
            allowed_statuses: RULE_STATUSES,
            ..Change::default()
        };
        let result = apply_change(db, &board, change, None).await?;
        outcomes.push((board.id, Ok(result)));
    }
    Ok(outcomes)
}

/// `reverse`: takes back a reversible rule the contact earned before.
pub async fn reverse(db: &Db, input: &Award) -> Result<Vec<AwardOutcome>> {
    let Some(inbox_id) = input.inbox_id else {
        return Ok(Vec::new());
    };
    let mut outcomes = Vec::new();
    for board in scorable(db, Some(inbox_id), None).await? {
        let Some(rule) = rule_of(&board.rules.0, &input.rule).filter(|r| r.enabled) else {
            continue;
        };
        if board.workspace_id != input.workspace_id || !is_within_window(&board, input.event_at) {
            continue;
        }
        if input.rule == "joinGroup"
            && !has_str(
                &board.scope.0["telegramChatIds"],
                input.context["chatId"].as_str(),
            )
        {
            continue;
        }
        let (board_id, contact) = (board.id, input.contact_id);
        let participant = tx::read(db, move |conn| {
            Ok(participants::find(conn, board_id, contact)?)
        })
        .await?;
        let earned = participant
            .as_ref()
            .and_then(|p| p.breakdown.0.get(&input.rule).and_then(Value::as_i64))
            .unwrap_or(0);
        if earned <= 0 {
            continue;
        }
        let id = board.id.to_string();
        let change = Change {
            contact_id: input.contact_id,
            delta: -rule.points,
            source: "rule",
            rule: Some(input.rule.clone()),
            breakdown_step: Some(-1),
            source_ref: Some(format!("reverse:{}", input.source_ref)),
            event_id: event_id(&[&id, &input.rule, "reverse", &input.source_ref]),
            event_at: input.event_at,
            allowed_statuses: RULE_STATUSES,
            ..Change::default()
        };
        let result = apply_change(db, &board, change, None).await?;
        outcomes.push((board.id, Ok(result)));
    }
    Ok(outcomes)
}

/// The input of [`change_from_flow`] (the "Change gamification scores" step).
#[derive(Debug, Clone)]
pub struct FlowChange {
    pub workspace_id: i64,
    pub leaderboard_id: i64,
    pub contact_id: i64,
    pub subtract: bool,
    pub points: f64,
    pub user: bool,
    pub referrer: bool,
    /// The step execution key and, when known, the triggering time.
    pub dedup: Option<(String, Option<i64>)>,
}

/// `changeFromFlow` status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowStatus {
    Applied,
    Blocked,
    NotFound,
}

/// `changeFromFlow`: blocked unless the leaderboard is active.
pub async fn change_from_flow(
    db: &Db,
    input: FlowChange,
) -> Result<(FlowStatus, Vec<(i64, ScoreChangeResult)>)> {
    let (ws, id) = (input.workspace_id, input.leaderboard_id);
    let Some(board) = tx::read(db, move |conn| {
        Ok(leaderboard::find_by_id_and_workspace(conn, id, ws)?.map(|w| w.leaderboard))
    })
    .await?
    else {
        return Ok((FlowStatus::NotFound, Vec::new()));
    };
    let now = now_ms();
    if board.status != "active" || !is_within_window(&board, now) {
        return Ok((FlowStatus::Blocked, Vec::new()));
    }
    let magnitude = (input.points.trunc().abs() as i64).min(MAX_SCORE);
    let delta = if input.subtract {
        -magnitude
    } else {
        magnitude
    };
    let mut recipients = Vec::new();
    if input.user {
        recipients.push(input.contact_id);
    }
    if input.referrer {
        let contact = input.contact_id;
        let found = tx::read(db, move |conn| Ok(participants::find(conn, id, contact)?)).await?;
        if let Some(referrer) = found.and_then(|p| p.referrer_contact_id) {
            recipients.push(referrer);
        }
    }
    let board_id = board.id.to_string();
    let mut results = Vec::new();
    for contact_id in recipients {
        let (event_id, event_at, dedup_since) = match &input.dedup {
            Some((key, at)) => (
                event_id(&[&board_id, "flow", key, &contact_id.to_string()]),
                at.unwrap_or(now),
                at.is_none().then_some(now - FLOW_DEDUP_LOOKBACK_MS),
            ),
            None => (new_id(), now, None),
        };
        let change = Change {
            contact_id,
            delta,
            source: "flow",
            breakdown_key: Some("flow".to_owned()),
            event_id,
            event_at,
            dedup_since,
            allowed_statuses: ACTIVE_ONLY,
            ..Change::default()
        };
        results.push((contact_id, apply_change(db, &board, change, None).await?));
    }
    if !results.is_empty() && results.iter().all(|(_, r)| *r == ScoreChangeResult::Closed) {
        return Ok((FlowStatus::Blocked, results));
    }
    Ok((FlowStatus::Applied, results))
}

/// `creditReferral` outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Referral {
    Credited,
    NotNew,
    Ignored,
}

/// `creditReferral`: the invitee joins (stamping the referrer); the
/// referrer is credited only for a NEW invitee.
pub async fn credit_referral(
    db: &Db,
    workspace_id: i64,
    leaderboard_id: i64,
    invitee: i64,
    referrer: i64,
) -> Result<Referral> {
    if invitee == referrer {
        return Ok(Referral::Ignored);
    }
    let found = tx::read(db, move |conn| {
        let board = leaderboard::find_by_id_and_workspace(conn, leaderboard_id, workspace_id)?
            .map(|w| w.leaderboard);
        let contact: Option<i64> =
            chatbotx_db::rusqlite::OptionalExtension::optional(conn.query_row(
                r#"SELECT 1 FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                chatbotx_db::rusqlite::params![referrer, workspace_id],
                |r| r.get(0),
            ))?;
        let banned =
            participants::find(conn, leaderboard_id, referrer)?.is_some_and(|p| p.is_banned);
        Ok((board, contact.is_some(), banned))
    })
    .await?;
    let (Some(board), true, false) = found else {
        return Ok(Referral::Ignored);
    };
    if board.status != "active" || !is_within_window(&board, now_ms()) {
        return Ok(Referral::Ignored);
    }
    let id = board.id.to_string();
    let joined = apply_change(
        db,
        &board,
        Change {
            contact_id: invitee,
            source: "referral",
            referrer_contact_id: Some(referrer),
            event_id: event_id(&[&id, "join", &invitee.to_string()]),
            event_at: board.starts_at,
            allowed_statuses: ACTIVE_ONLY,
            ..Change::default()
        },
        None,
    )
    .await?;
    let joined_now = matches!(joined, ScoreChangeResult::Applied { is_new: true, .. });
    if !joined_now {
        let referred = tx::read(db, move |conn| {
            Ok(participants::find(conn, leaderboard_id, invitee)?
                .is_some_and(|p| p.referrer_contact_id == Some(referrer)))
        })
        .await?;
        if !referred {
            return Ok(Referral::NotNew);
        }
    }
    let Some(rule) = rule_of(&board.rules.0, "referral").filter(|r| r.enabled && r.points > 0)
    else {
        return Ok(Referral::Credited);
    };
    let credit = apply_change(
        db,
        &board,
        Change {
            contact_id: referrer,
            delta: rule.points,
            source: "referral",
            rule: Some("referral".to_owned()),
            source_ref: Some(invitee.to_string()),
            event_id: event_id(&[&id, "referral", &invitee.to_string()]),
            event_at: now_ms(),
            limits: Some(ScoreLimits {
                day_bucket: day_key(now_ms(), &board.reset_time),
                max_per_day: rule.max_per_day,
                cooldown_seconds: 0,
            }),
            allowed_statuses: ACTIVE_ONLY,
            ..Change::default()
        },
        None,
    )
    .await?;
    Ok(
        if joined_now || matches!(credit, ScoreChangeResult::Applied { .. }) {
            Referral::Credited
        } else {
            Referral::NotNew
        },
    )
}
