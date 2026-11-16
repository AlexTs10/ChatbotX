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

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Minigame, MinigameContact};
use chatbotx_db::rusqlite::Connection;
use chatbotx_jobs::Enqueuer;
use serde_json::Value;

use crate::errors::Result;
use crate::events::BoxFuture;

/// `MinigameContactVariableResolver`: resolves contact variables in the
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

impl<'a> PlayDeps<'a> {
    pub fn new(db: &'a Db, jobs: Option<Arc<dyn Enqueuer>>) -> PlayDeps<'a> {
        let _ = (db, jobs);
        todo!("P3.6-D1")
    }

    /// Replaces `Math.random`.
    pub fn with_random(self, random: impl Fn() -> f64 + Send + Sync + 'static) -> PlayDeps<'a> {
        let _ = random;
        todo!("P3.6-D1")
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
    let _ = (general_settings, now_ms);
    todo!("P3.6-D1")
}

/// `resolvePlayState` inside the caller's write transaction. Returns the
/// play-state row and whether this call inserted it.
pub fn resolve_play_state(
    conn: &Connection,
    input: &PlayStateInput,
) -> Result<(MinigameContact, bool)> {
    let _ = (conn, input);
    todo!("P3.6-D1")
}

/// `resolveOpenerPlayState`: [`resolve_play_state`] in a transaction of its
/// own.
pub async fn resolve_opener_play_state(
    db: &Db,
    input: PlayStateInput,
) -> Result<(MinigameContact, bool)> {
    let _ = (db, input);
    todo!("P3.6-D1")
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
pub async fn grant_referral_bonus(db: &Db, input: ReferralGrant) -> Result<bool> {
    let _ = (db, input);
    todo!("P3.6-D1")
}

/// `creditSharedLinkReferral`.
pub async fn credit_shared_link_referral(
    deps: &PlayDeps<'_>,
    minigame: &Minigame,
    contact_id: i64,
    contact_inbox_id: Option<i64>,
    referrer_contact_id: i64,
) -> Result<bool> {
    let _ = (
        deps,
        minigame,
        contact_id,
        contact_inbox_id,
        referrer_contact_id,
    );
    todo!("P3.6-D1")
}

/// `MinigamePlayResult`: the won prize item (JSON) or `nonWinning`.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayResult {
    Prize(Value),
    NonWinning,
}

/// `resolveMinigamePrize` with the roll `random() * 100`.
pub fn resolve_prize(prize_settings: &Value, roll: f64) -> PlayResult {
    let _ = (prize_settings, roll);
    todo!("P3.6-D1")
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

/// `recordPlay`: 403 `minigameNotActive` outside the play window, 403
/// `minigameNoDrawsLeft` with no draws left.
pub async fn record_play(
    deps: &PlayDeps<'_>,
    minigame: &Minigame,
    contact_id: i64,
    contact_inbox_id: i64,
) -> Result<RecordedPlay> {
    let _ = (deps, minigame, contact_id, contact_inbox_id);
    todo!("P3.6-D1")
}

/// The `ContactInbox` fields the outcome messages use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayContactInbox {
    pub id: i64,
    pub contact_id: i64,
}

/// `recordPlayAndDispatch`: records the play, then the leaderboard award,
/// the player tags, the prize-name custom field and the outcome message.
/// The side effects after the play never fail the call.
pub async fn record_play_and_dispatch(
    deps: &PlayDeps<'_>,
    minigame: &Minigame,
    contact_id: i64,
    contact_inbox: PlayContactInbox,
    resolver: Option<ContactVariableResolver>,
) -> Result<(MinigameContact, PlayResult)> {
    let _ = (deps, minigame, contact_id, contact_inbox, resolver);
    todo!("P3.6-D1")
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
    let _ = (deps, input);
    todo!("P3.6-D1")
}

/// `sendLoseMessage` (best-effort: failures are logged).
pub async fn send_lose_message(deps: &PlayDeps<'_>, input: OutcomeMessage) {
    let _ = (deps, input);
    todo!("P3.6-D1")
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

/// `enqueueLeaderboardAward`: queues the `award` job when a live leaderboard
/// of the minigame (or inbox) scores the rule. Never fails.
pub async fn enqueue_leaderboard_award(deps: &PlayDeps<'_>, input: LeaderboardAward) -> bool {
    let _ = (deps, input);
    todo!("P3.6-D1")
}
