//! The play runtime of `minigame/minigame-contact-service.ts`:
//! `resolvePlayState` / `resolveOpenerPlayState`, `recordPlay`,
//! `grantReferralBonus`, `creditSharedLinkReferral`, `recordPlayAndDispatch`
//! and the outcome messages (`sendWinMessage` / `sendLoseMessage`), plus
//! `play-window.ts`, `resolve-prize.ts` and the minigame side of
//! `leaderboard/enqueue.ts` (`enqueueLeaderboardAward`).
//!
//! Jobs go through the [`Enqueuer`] of the API state (`chat`, `integration`
//! and `leaderboard` queues); with no enqueuer the jobs are dropped.

use std::sync::Arc;

use chatbotx_core::utils::spintax::apply_spintax_with;
use chatbotx_db::Db;
use chatbotx_db::repo::message::{self, CreateMessageInput};
use chatbotx_db::repo::models::{Minigame, MinigameContact};
use chatbotx_db::repo::{self, find_one, leaderboard, returning};
use chatbotx_db::rusqlite::types::ValueRef;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName, Retention};
use serde_json::{Map, Value, json};

use crate::contact_custom_field::{self, SetValuesInput};
use crate::errors::{BusinessError, Result};
use crate::events::BoxFuture;
use crate::{tag, tx};

use super::contact::derive_remaining;

const ONE_DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// `MINIGAME_PRIZE_NAME_TOKEN`.
const PRIZE_NAME_TOKEN: &str = "{{prize_name}}";
const MAX_PERCENT: f64 = 100.0;
/// `LEADERBOARD_LATE_EVENT_GRACE_MS`.
const LEADERBOARD_LATE_EVENT_GRACE_MS: i64 = 10 * 60 * 1000;
/// `AWARD_JOB_RETENTION_SECONDS`.
const AWARD_JOB_RETENTION_SECONDS: u64 = 24 * 60 * 60;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// A JSON id (number or numeric string; ids are strings in the original).
fn json_id(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn json_ids(value: Option<&Value>) -> Vec<i64> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(json_id).collect())
        .unwrap_or_default()
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

/// A JS number as JSON (`10`, not `10.0`).
fn js_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// `new Date(value).getTime()` of an ISO string (or a number of ms).
fn date_ms(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_f64().map(|v| v as i64),
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.timestamp_millis())
            .ok()
            .or_else(|| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                    .ok()
                    .map(|d| d.and_utc().timestamp_millis())
            })
            .or_else(|| {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|d| d.and_utc().timestamp_millis())
            }),
        _ => None,
    }
}

/// One row as a JSON object (column name -> stored value).
fn row_json(conn: &Connection, table: &str, id: i64) -> chatbotx_db::rusqlite::Result<Value> {
    let sql = format!(r#"SELECT * FROM "{table}" WHERE "id" = ?1"#);
    let mut stmt = conn.prepare(&sql)?;
    let names: Vec<String> = stmt.column_names().map(|n| n.to_string()).collect();
    stmt.query_row([id], |row| {
        let mut map = Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null | ValueRef::Blob(_) => Value::Null,
                ValueRef::Integer(v) => json!(v),
                ValueRef::Real(v) => json!(v),
                ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
            };
            map.insert(name.clone(), value);
        }
        Ok(Value::Object(map))
    })
}

/// `ContactVariableResolver`: resolves contact variables in the
/// outcome text; an `Err` keeps the unresolved text.
pub type ContactVariableResolver =
    Arc<dyn Fn(String) -> BoxFuture<'static, std::result::Result<String, String>> + Send + Sync>;

/// The services of the play runtime. `random` is `Math.random` (the prize
/// roll and the spintax branch).
#[derive(Clone)]
pub struct PlayDeps<'a> {
    pub db: &'a Db,
    pub jobs: Option<Arc<dyn Enqueuer>>,
    pub random: Arc<dyn Fn() -> f64 + Send + Sync>,
}

/// `tagSyncService.enqueueAttach` for the newly linked tags.
async fn sync_attached(deps: &PlayDeps<'_>, workspace_id: i64, contact_id: i64, tag_ids: &[i64]) {
    if let Some(jobs) = deps.jobs.as_deref() {
        let pairs: Vec<(i64, i64)> = tag_ids.iter().map(|t| (contact_id, *t)).collect();
        crate::tag_sync::enqueue_attach_many(jobs, workspace_id, &pairs).await;
    }
}

/// `Math.random`: 53 random bits in `[0, 1)`.
fn system_random() -> f64 {
    let mut bytes = [0u8; 8];
    if ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).is_err() {
        return 0.0;
    }
    (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
}

impl<'a> PlayDeps<'a> {
    pub fn new(db: &'a Db, jobs: Option<Arc<dyn Enqueuer>>) -> PlayDeps<'a> {
        PlayDeps {
            db,
            jobs,
            random: Arc::new(system_random),
        }
    }

    /// Replaces `Math.random`.
    pub fn with_random(self, random: impl Fn() -> f64 + Send + Sync + 'static) -> PlayDeps<'a> {
        PlayDeps {
            random: Arc::new(random),
            ..self
        }
    }
}

/// The input of `resolvePlayState`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayStateInput {
    pub minigame_id: i64,
    pub contact_id: i64,
    /// `Minigame.playerSettings` (JSON).
    pub player_settings: Value,
    pub contact_inbox_id: Option<i64>,
    pub referrer_contact_id: Option<i64>,
}

/// `isMinigameWithinPlayWindow`: `playedAtFrom <= now <= playedAtTo`
/// (`generalSettings`; an unparsable date is never inside).
pub fn is_within_play_window(general_settings: &Value, now_ms: i64) -> bool {
    match (
        date_ms(general_settings.get("playedAtFrom")),
        date_ms(general_settings.get("playedAtTo")),
    ) {
        (Some(from), Some(to)) => now_ms >= from && now_ms <= to,
        _ => false,
    }
}

fn find_state(
    conn: &Connection,
    minigame_id: i64,
    contact_id: i64,
) -> Result<Option<MinigameContact>> {
    find_one::<MinigameContact>(
        conn,
        r#""minigameId" = ?1 AND "contactId" = ?2"#,
        &[&minigame_id, &contact_id],
    )
    .map_err(BusinessError::Repo)
}

fn set_remaining(conn: &Connection, id: i64, remaining: i64) -> Result<MinigameContact> {
    returning::<MinigameContact>(
        conn,
        r#"UPDATE "MinigameContact" SET "remaining" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
        &[&remaining, &now_ms(), &id],
    )?
    .pop()
    .ok_or_else(|| BusinessError::Repo(repo::Error::msg("MinigameContact row vanished")))
}

/// `resolvePlayState` inside the caller's write transaction. Returns the
/// play-state row and whether this call inserted it.
pub fn resolve_play_state(
    conn: &Connection,
    input: &PlayStateInput,
) -> Result<(MinigameContact, bool)> {
    let settings = &input.player_settings;
    let draws = number(settings, "drawsPerPerson").unwrap_or(0.0) as i64;
    let existing = match find_state(conn, input.minigame_id, input.contact_id)? {
        Some(row) => row,
        None => {
            let referrer = input
                .referrer_contact_id
                .filter(|referrer| *referrer != input.contact_id);
            let now = now_ms();
            let created = returning::<MinigameContact>(
                conn,
                r#"INSERT INTO "MinigameContact"
                     ("createdAt", "updatedAt", "minigameId", "contactId", "contactInboxId", "openedAt",
                      "remaining", "played", "referrerContactId")
                   VALUES (?1, ?1, ?2, ?3, ?4, ?1, ?5, 0, ?6) ON CONFLICT DO NOTHING"#,
                &[
                    &now,
                    &input.minigame_id,
                    &input.contact_id,
                    &input.contact_inbox_id,
                    &draws,
                    &referrer,
                ],
            )?;
            if let Some(row) = created.into_iter().next() {
                conn.execute(
                    r#"UPDATE "Minigame" SET "participantsCount" = "participantsCount" + 1, "updatedAt" = ?1
                       WHERE "id" = ?2"#,
                    params![now, input.minigame_id],
                )
                .map_err(db_error)?;
                return Ok((row, true));
            }
            find_state(conn, input.minigame_id, input.contact_id)?.ok_or_else(|| {
                BusinessError::Repo(repo::Error::msg(
                    "MinigameContact insert conflicted but no existing row found",
                ))
            })?
        }
    };

    match settings.get("resetPolicy").and_then(Value::as_str) {
        Some("everyNDays") => {
            let interval = number(settings, "resetIntervalDays").unwrap_or(f64::NAN);
            if ((now_ms() - existing.updated_at) as f64) >= interval * ONE_DAY_MS as f64 {
                return Ok((set_remaining(conn, existing.id, draws)?, false));
            }
        }
        Some("never") => {
            let expected = derive_remaining(settings, existing.played, existing.shares_count);
            if existing.remaining != expected {
                return Ok((set_remaining(conn, existing.id, expected)?, false));
            }
        }
        _ => {}
    }
    Ok((existing, false))
}

/// `resolveOpenerPlayState`: [`resolve_play_state`] in a transaction of its
/// own.
pub async fn resolve_opener_play_state(
    db: &Db,
    input: PlayStateInput,
) -> Result<(MinigameContact, bool)> {
    tx::write(db, move |conn| resolve_play_state(conn, &input)).await
}

/// The input of `grantReferralBonus`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferralGrant {
    pub minigame_id: i64,
    pub referrer_contact_id: i64,
    pub invitee_contact_id: i64,
    pub player_settings: Value,
}

/// `grantReferralBonus` (private in the original): one bonus draw for the
/// referrer, capped by `maxSharesPerPerson` inside the `UPDATE`.
/// `updatedAt` keeps its value (it is the reset-cycle marker).
pub async fn grant_referral_bonus(db: &Db, input: ReferralGrant) -> Result<bool> {
    // Legacy `playerSettings` has no `maxSharesPerPerson` key at all.
    let cap = number(&input.player_settings, "maxSharesPerPerson").unwrap_or(0.0);
    if cap <= 0.0 || input.referrer_contact_id == input.invitee_contact_id {
        return Ok(false);
    }
    tx::write(db, move |conn| {
        let granted: Option<i64> = conn
            .query_row(
                r#"UPDATE "MinigameContact"
                   SET "sharesCount" = "sharesCount" + 1, "remaining" = "remaining" + 1
                   WHERE "minigameId" = ?1 AND "contactId" = ?2 AND "sharesCount" < ?3
                   RETURNING "id""#,
                params![input.minigame_id, input.referrer_contact_id, cap],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if granted.is_none() {
            return Ok(false);
        }
        conn.execute(
            r#"UPDATE "Minigame" SET "sharesCount" = "sharesCount" + 1, "updatedAt" = ?1 WHERE "id" = ?2"#,
            params![now_ms(), input.minigame_id],
        )
        .map_err(db_error)?;
        Ok(true)
    })
    .await
}

/// `creditSharedLinkReferral`.
pub async fn credit_shared_link_referral(
    deps: &PlayDeps<'_>,
    minigame: &Minigame,
    contact_id: i64,
    contact_inbox_id: Option<i64>,
    referrer_contact_id: i64,
) -> Result<bool> {
    if referrer_contact_id == contact_id {
        return Ok(false);
    }
    if !is_within_play_window(&minigame.general_settings, now_ms()) {
        return Ok(false);
    }
    let (state, created) = resolve_opener_play_state(
        deps.db,
        PlayStateInput {
            minigame_id: minigame.id,
            contact_id,
            player_settings: minigame.player_settings.0.clone(),
            contact_inbox_id,
            referrer_contact_id: Some(referrer_contact_id),
        },
    )
    .await?;
    let Some(referrer) = state.referrer_contact_id.filter(|_| created) else {
        return Ok(false);
    };

    // Tagging is about the friend having arrived, not about the grant.
    let tag_ids = json_ids(minigame.general_settings.get("newFriendTagIds"));
    let linked =
        tag::attach_to_contact(deps.db, minigame.workspace_id, contact_id, &tag_ids).await?;
    sync_attached(deps, minigame.workspace_id, contact_id, &linked).await;

    let granted = grant_referral_bonus(
        deps.db,
        ReferralGrant {
            minigame_id: minigame.id,
            referrer_contact_id: referrer,
            invitee_contact_id: contact_id,
            player_settings: minigame.player_settings.0.clone(),
        },
    )
    .await?;
    if granted {
        enqueue_leaderboard_award(
            deps,
            LeaderboardAward {
                workspace_id: minigame.workspace_id,
                contact_id: referrer,
                inbox_id: None,
                minigame_id: Some(minigame.id),
                rule: "inGameShare".into(),
                source_ref: contact_id.to_string(),
                event_at: state.created_at,
                context: None,
            },
        )
        .await;
    }
    Ok(granted)
}

/// `MinigamePlayResult`: the won prize item (JSON) or `nonWinning`.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayResult {
    Prize(Value),
    NonWinning,
}

/// `resolveMinigamePrize` with the roll `random() * 100`.
pub fn resolve_prize(prize_settings: &Value, roll: f64) -> PlayResult {
    let mut cumulative = 0.0;
    for prize in prize_settings
        .get("prizes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        cumulative += number(prize, "winRate").unwrap_or(f64::NAN);
        if roll < cumulative {
            return PlayResult::Prize(prize.clone());
        }
    }
    PlayResult::NonWinning
}

/// The result of `recordPlay`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedPlay {
    pub contact_state: MinigameContact,
    pub result: PlayResult,
    /// The `MinigamePlay` row (id, `createdAt` unix ms).
    pub play_id: i64,
    pub play_created_at: i64,
}

/// `drawPrize`: the prize roll over the stored `prizeSettings` (the write
/// transaction holds the lock); a tracked prize at quantity 0 is skipped.
fn draw_prize(prize_settings: &Value, roll: f64) -> PlayResult {
    let prizes = prize_settings
        .get("prizes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !prizes.iter().any(|prize| prize.get("quantity").is_some()) {
        return resolve_prize(prize_settings, roll);
    }
    let available: Vec<Value> = prizes
        .into_iter()
        .filter(|prize| match prize.get("quantity") {
            None => true,
            Some(quantity) => quantity.as_f64().is_some_and(|q| q > 0.0),
        })
        .collect();
    let mut settings = prize_settings.clone();
    settings["prizes"] = Value::Array(available);
    resolve_prize(&settings, roll)
}

/// `recordPlay`: 403 `minigameNotActive` outside the play window, 403
/// `minigameNoDrawsLeft` with no draws left.
pub async fn record_play(
    deps: &PlayDeps<'_>,
    minigame: &Minigame,
    contact_id: i64,
    contact_inbox_id: i64,
) -> Result<RecordedPlay> {
    if !is_within_play_window(&minigame.general_settings, now_ms()) {
        return Err(BusinessError::exception(
            "This minigame is not currently active",
            "minigameNotActive",
            403,
        ));
    }
    let roll = (deps.random)() * MAX_PERCENT;
    let minigame_id = minigame.id;
    let input = PlayStateInput {
        minigame_id,
        contact_id,
        player_settings: minigame.player_settings.0.clone(),
        contact_inbox_id: Some(contact_inbox_id),
        referrer_contact_id: None,
    };
    tx::write(deps.db, move |conn| {
        let (state, _) = resolve_play_state(conn, &input)?;
        if state.remaining <= 0 {
            return Err(BusinessError::exception(
                "No draws remaining for this contact",
                "minigameNoDrawsLeft",
                403,
            ));
        }
        let stored: String = conn
            .query_row(
                r#"SELECT "prizeSettings" FROM "Minigame" WHERE "id" = ?1"#,
                [minigame_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let mut prize_settings: Value = serde_json::from_str(&stored).unwrap_or(Value::Null);
        let result = draw_prize(&prize_settings, roll);
        let now = now_ms();

        if let PlayResult::Prize(prize) = &result
            && let Some(quantity) = prize.get("quantity")
        {
            let left = js_number(quantity.as_f64().unwrap_or(0.0) - 1.0);
            if let Some(items) = prize_settings
                .get_mut("prizes")
                .and_then(Value::as_array_mut)
            {
                for item in items.iter_mut() {
                    if item.get("id") == prize.get("id") {
                        item["quantity"] = left.clone();
                    }
                }
            }
            conn.execute(
                r#"UPDATE "Minigame" SET "prizeSettings" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
                params![prize_settings.to_string(), now, minigame_id],
            )
            .map_err(db_error)?;
        }

        let won = matches!(result, PlayResult::Prize(_));
        conn.execute(
            r#"UPDATE "Minigame" SET "playsCount" = "playsCount" + 1,
                 "winnersCount" = "winnersCount" + ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
            params![i64::from(won), now, minigame_id],
        )
        .map_err(db_error)?;

        let contact_state = returning::<MinigameContact>(
            conn,
            r#"UPDATE "MinigameContact" SET "remaining" = ?1, "played" = ?2, "updatedAt" = ?3 WHERE "id" = ?4"#,
            &[&(state.remaining - 1), &(state.played + 1), &now, &state.id],
        )?
        .pop()
        .ok_or_else(|| BusinessError::Repo(repo::Error::msg("MinigameContact row vanished")))?;

        let (prize_id, prize_name) = match &result {
            PlayResult::Prize(prize) => (
                prize.get("id").and_then(Value::as_str).map(str::to_string),
                prize.get("name").and_then(Value::as_str).map(str::to_string),
            ),
            PlayResult::NonWinning => (None, None),
        };
        let (play_id, play_created_at): (i64, i64) = conn
            .query_row(
                r#"INSERT INTO "MinigamePlay"
                     ("createdAt", "updatedAt", "minigameId", "contactId", "isWinning", "prizeId", "prizeName")
                   VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6) RETURNING "id", "createdAt""#,
                params![now, minigame_id, contact_id, i64::from(won), prize_id, prize_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db_error)?;
        Ok(RecordedPlay {
            contact_state,
            result,
            play_id,
            play_created_at,
        })
    })
    .await
}

/// The `ContactInbox` fields the outcome messages use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayContactInbox {
    pub id: i64,
    pub contact_id: i64,
}

/// `DEFAULT_OUTCOME_MESSAGE`: a legacy row has no `outcomeMessage` key.
fn outcome_message(settings: &Value) -> Value {
    settings
        .get("outcomeMessage")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({ "enabled": false, "mode": "text", "text": "" }))
}

fn enabled(outcome: &Value) -> bool {
    outcome
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// `recordPlayAndDispatch`: records the play, then the leaderboard award,
/// the player tags, the prize-name custom field and the outcome message.
/// The side effects after the play never fail the call (the tags do, as in
/// the original).
pub async fn record_play_and_dispatch(
    deps: &PlayDeps<'_>,
    minigame: &Minigame,
    contact_id: i64,
    contact_inbox: PlayContactInbox,
    resolver: Option<ContactVariableResolver>,
) -> Result<(MinigameContact, PlayResult)> {
    let played = record_play(deps, minigame, contact_id, contact_inbox.id).await?;
    let workspace_id = minigame.workspace_id;

    if matches!(played.result, PlayResult::Prize(_)) {
        enqueue_leaderboard_award(
            deps,
            LeaderboardAward {
                workspace_id,
                contact_id,
                inbox_id: None,
                minigame_id: Some(minigame.id),
                rule: "minigameWin".into(),
                source_ref: played.play_id.to_string(),
                event_at: played.play_created_at,
                context: None,
            },
        )
        .await;
    }

    let tag_ids = json_ids(minigame.general_settings.get("playerTagIds"));
    let linked = tag::attach_to_contact(deps.db, workspace_id, contact_id, &tag_ids).await?;
    sync_attached(deps, workspace_id, contact_id, &linked).await;

    let prize_name = match &played.result {
        PlayResult::Prize(prize) => prize.get("name"),
        PlayResult::NonWinning => minigame
            .prize_settings
            .get("nonWinning")
            .and_then(|n| n.get("title")),
    }
    .and_then(Value::as_str)
    .unwrap_or_default()
    .to_string();

    if let Some(field_id) = minigame
        .prize_settings
        .get("prizeNameCustomFieldId")
        .and_then(json_id)
    {
        // Best-effort personalization write.
        let _ = contact_custom_field::set_values(
            deps.db,
            SetValuesInput {
                workspace_id,
                contact_id,
                fields: vec![(field_id, prize_name.clone())],
                ..SetValuesInput::default()
            },
        )
        .await;
    }

    let message = |outcome: Value| OutcomeMessage {
        workspace_id,
        contact_id,
        contact_inbox,
        prize_name: prize_name.clone(),
        outcome_message: outcome,
        resolver: resolver.clone(),
    };
    match &played.result {
        PlayResult::NonWinning => {
            let outcome = outcome_message(&minigame.non_winning_message_settings);
            if enabled(&outcome) {
                send_lose_message(deps, message(outcome)).await;
            }
        }
        PlayResult::Prize(_) => {
            let outcome = outcome_message(&minigame.winning_message_settings);
            if enabled(&outcome) {
                send_win_message(deps, message(outcome)).await;
            }
        }
    }
    Ok((played.contact_state, played.result))
}

/// The input of `sendWinMessage` / `sendLoseMessage`.
#[derive(Clone)]
pub struct OutcomeMessage {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub contact_inbox: PlayContactInbox,
    pub prize_name: String,
    /// `MinigameOutcomeMessage` (JSON).
    pub outcome_message: Value,
    pub resolver: Option<ContactVariableResolver>,
}

/// `sendWinMessage` (best-effort: failures are logged).
pub async fn send_win_message(deps: &PlayDeps<'_>, input: OutcomeMessage) {
    send_outcome_message(deps, input, "win").await;
}

/// `sendLoseMessage` (best-effort: failures are logged).
pub async fn send_lose_message(deps: &PlayDeps<'_>, input: OutcomeMessage) {
    send_outcome_message(deps, input, "lose").await;
}

/// `renderOutcomeText`: spintax (author copy only), then `{{prize_name}}`,
/// then the contact variables.
async fn render_outcome_text(deps: &PlayDeps<'_>, input: &OutcomeMessage, text: &str) -> String {
    let random = Arc::clone(&deps.random);
    let mut pick = move |count: usize| (random() * count as f64).floor() as usize;
    let with_prize_name =
        apply_spintax_with(text, &mut pick).replace(PRIZE_NAME_TOKEN, &input.prize_name);
    let Some(resolver) = &input.resolver else {
        return with_prize_name;
    };
    match resolver(with_prize_name.clone()).await {
        Ok(resolved) => resolved,
        Err(error) => {
            tracing::warn!(
                err = %error,
                workspace_id = input.workspace_id,
                contact_id = input.contact_id,
                "Failed to resolve variables in minigame outcome message, sending raw text"
            );
            with_prize_name
        }
    }
}

async fn send_outcome_message(deps: &PlayDeps<'_>, input: OutcomeMessage, context: &str) {
    if !enabled(&input.outcome_message) {
        return;
    }
    if let Err(error) = try_send_outcome_message(deps, &input).await {
        tracing::warn!(
            err = %error,
            workspace_id = input.workspace_id,
            contact_id = input.contact_id,
            "Failed to send minigame {context} message"
        );
    }
}

async fn try_send_outcome_message(
    deps: &PlayDeps<'_>,
    input: &OutcomeMessage,
) -> std::result::Result<(), String> {
    let (workspace_id, contact_id) = (input.workspace_id, input.contact_id);
    // `findDMByContact`: the DM conversation is `sourceId IS NULL`.
    let conversation_id: Option<i64> = tx::read(deps.db, move |conn| {
        conn.query_row(
            r#"SELECT "id" FROM "Conversation"
               WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL
               ORDER BY "id" LIMIT 1"#,
            params![workspace_id, contact_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(conversation_id) = conversation_id else {
        return Ok(());
    };
    let outcome = &input.outcome_message;
    let mode = outcome
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("text");

    if mode == "flow" || mode == "node" {
        let Some(flow_id) = outcome.get("flowId").and_then(json_id) else {
            return Ok(());
        };
        let mut data = json!({
            "conversationId": conversation_id.to_string(),
            "contactInboxId": input.contact_inbox.id.to_string(),
            "flowId": flow_id.to_string(),
        });
        if mode == "node"
            && let Some(node_id) = outcome.get("nodeId").and_then(Value::as_str)
        {
            data["nodeId"] = json!(node_id);
        }
        if let Some(jobs) = &deps.jobs {
            jobs.add(
                QueueName::Integration,
                "sendFlow",
                json!({ "type": "sendFlow", "data": data }),
                JobOptions::default(),
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }

    let Some(text) = outcome
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    else {
        return Ok(());
    };
    // Rendered before the row is written: the stored text is the sent text.
    let text = render_outcome_text(deps, input, text).await;
    let contact_inbox = input.contact_inbox;
    let payload = tx::write(deps.db, move |conn| {
        let created_at = now_ms();
        let message = message::create(
            conn,
            &CreateMessageInput {
                contact_inbox_id: contact_inbox.id,
                conversation_id,
                workspace_id,
                content_type: "text".into(),
                message_type: "outgoing".into(),
                sender_type: "system".into(),
                text: Some(text),
                created_at: Some(created_at),
                ..CreateMessageInput::default()
            },
        )?;
        conn.execute(
            r#"UPDATE "Conversation" SET "lastActivityAt" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
            params![created_at, now_ms(), conversation_id],
        )
        .map_err(db_error)?;
        // `contactInboxService.updateTracking` (`firstInteractionAt` keeps the
        // earliest, `lastMessageAt` is set).
        conn.execute(
            r#"UPDATE "ContactInbox"
               SET "firstInteractionAt" = CASE WHEN "firstInteractionAt" IS NULL OR "firstInteractionAt" > ?1
                     THEN ?1 ELSE "firstInteractionAt" END,
                   "lastMessageAt" = ?1, "updatedAt" = ?2
               WHERE "id" = ?3 AND "contactId" = ?4
                 AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?5)"#,
            params![
                message.created_at,
                now_ms(),
                contact_inbox.id,
                contact_inbox.contact_id,
                workspace_id
            ],
        )
        .map_err(db_error)?;
        Ok(json!({
            "conversation": row_json(conn, "Conversation", conversation_id).map_err(db_error)?,
            "contactInbox": row_json(conn, "ContactInbox", contact_inbox.id).map_err(db_error)?,
            "message": row_json(conn, "Message", message.id).map_err(db_error)?,
        }))
    })
    .await
    .map_err(|e| e.to_string())?;

    if let Some(jobs) = &deps.jobs {
        jobs.add(
            QueueName::Chat,
            "sendChannelMessage",
            json!({ "type": "sendChannelMessage", "data": payload }),
            JobOptions::default(),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `EnqueueLeaderboardAwardInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderboardAward {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub inbox_id: Option<i64>,
    pub minigame_id: Option<i64>,
    pub rule: String,
    pub source_ref: String,
    /// Unix ms of the source event.
    pub event_at: i64,
    pub context: Option<Value>,
}

/// `minigameHasLeaderboardRule` / `inboxHasLeaderboardRule` (no live cache:
/// one indexed read).
async fn is_listening(db: &Db, input: &LeaderboardAward) -> bool {
    let (column_minigame, id) = match (input.minigame_id, input.inbox_id) {
        (Some(id), _) => (true, id),
        (None, Some(id)) => (false, id),
        (None, None) => return false,
    };
    let rule = input.rule.clone();
    let since = now_ms() - LEADERBOARD_LATE_EVENT_GRACE_MS;
    let found = tx::read(db, move |conn| {
        let boards = if column_minigame {
            leaderboard::find_scorable_for_minigame(conn, id, since)?
        } else {
            leaderboard::find_scorable_for_inbox(conn, id, since)?
        };
        Ok(boards.iter().any(|board| {
            board
                .rules
                .get(&rule)
                .and_then(|r| r.get("enabled"))
                .is_some_and(|enabled| enabled.as_bool().unwrap_or(!enabled.is_null()))
        }))
    })
    .await;
    match found {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(err = %error, "leaderboard live lookup failed");
            false
        }
    }
}

/// `enqueueLeaderboardAward`: queues the `award` job when a live leaderboard
/// of the minigame (or inbox) scores the rule. Never fails.
pub async fn enqueue_leaderboard_award(deps: &PlayDeps<'_>, input: LeaderboardAward) -> bool {
    let Some(jobs) = &deps.jobs else {
        return false;
    };
    if !is_listening(deps.db, &input).await {
        return false;
    }
    let mut data = json!({
        "workspaceId": input.workspace_id.to_string(),
        "contactId": input.contact_id.to_string(),
        "inboxId": input.inbox_id.map(|id| id.to_string()),
        "minigameId": input.minigame_id.map(|id| id.to_string()),
        "rule": input.rule,
        "sourceRef": input.source_ref,
        "eventAt": iso(input.event_at),
    });
    if let Some(context) = &input.context {
        data["context"] = context.clone();
    }
    let host = match input.minigame_id {
        Some(id) => format!("mg{id}"),
        None => format!("in{}", input.inbox_id.unwrap_or_default()),
    };
    let job_id = format!(
        "lb-award-{}-{host}-{}-{}",
        input.workspace_id, input.rule, input.source_ref
    )
    .replace(':', "-");
    let opts = JobOptions {
        job_id: Some(job_id),
        remove_on_complete: Some(Retention::KeepAge {
            age_secs: AWARD_JOB_RETENTION_SECONDS,
            count: None,
        }),
        ..JobOptions::default()
    };
    match jobs
        .add(
            QueueName::Leaderboard,
            "award",
            json!({ "type": "award", "data": data }),
            opts,
        )
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(
                err = %error,
                rule = %input.rule,
                workspace_id = input.workspace_id,
                "failed to enqueue leaderboard award"
            );
            false
        }
    }
}
