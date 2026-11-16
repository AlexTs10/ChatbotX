//! Ports of the play-runtime describes of
//! `packages/business/__tests__/minigame-referral-bonus.test.ts`
//! (`grantReferralBonus`, `resolvePlayState — referral binding`,
//! `creditSharedLinkReferral`), `minigame-leaderboard-award.test.ts` and
//! `minigame-outcome-message-text.test.ts`.
//!
//! Adapted: the mocked `update ... set ... where` / `insert ... values`
//! chains become real `MinigameContact` / `Minigame` rows; "the transaction
//! was not called" becomes "no row changed"; the mocked `recordPlay` /
//! `resolveOpenerPlayState` / `grantReferralBonus` become real plays and
//! grants (a 100 % or 0 % prize, a referrer row at or below the cap); the
//! `enqueueLeaderboardAward` mock becomes a live `Leaderboard` row and the
//! recorded `leaderboard` job; the mocked message repository and
//! `chatQueue.add` become the `Message` row and the recorded `chat` job;
//! `Math.random` = 0 becomes `PlayDeps::with_random(|| 0.0)`.

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::minigame;
use chatbotx_business::minigame::play::{
    self, ContactVariableResolver, OutcomeMessage, PlayContactInbox, PlayDeps, PlayStateInput,
    ReferralGrant,
};
use chatbotx_db::repo::models::Minigame;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use chatbotx_jobs::{Enqueuer, QueueName, Retention};
use common::{WS, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::{RecordingEnqueuer, RecordingLog};

const GAME: i64 = 5101;
const REFERRER: i64 = 3101;
const INVITEE: i64 = 3102;
const PLAYER: i64 = 3103;
const NEW_FRIEND_TAG: i64 = 801;
const INBOX: i64 = 4101;
const PLAYER_INBOX: i64 = 4201;
const CONVERSATION: i64 = 4301;
const ONE_DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// An old `updatedAt` (unix ms) the grant must not move.
const OLD_UPDATED_AT: i64 = 1_600_000_000_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .expect("time")
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn never_policy(max_shares_per_person: i64) -> Value {
    json!({ "drawsPerPerson": 1, "maxSharesPerPerson": max_shares_per_person, "resetPolicy": "never" })
}

/// `generalSettings` with a play window around now (or ended a day ago).
fn general_settings(window_open: bool) -> Value {
    let now = now_ms();
    let to = if window_open {
        now + ONE_DAY_MS
    } else {
        now - ONE_DAY_MS
    };
    json!({
        "name": "Spin",
        "newFriendTagIds": [NEW_FRIEND_TAG.to_string()],
        "playerTagIds": [],
        "playedAtFrom": iso(now - 2 * ONE_DAY_MS),
        "playedAtTo": iso(to),
    })
}

/// A prize "Mug" with this win rate, the non-winning title "No luck".
fn prize_settings(win_rate: i64) -> Value {
    json!({
        "prizes": [{ "id": "p-1", "name": "Mug", "winRate": win_rate }],
        "nonWinning": { "title": "No luck", "loseRate": 100 - win_rate },
    })
}

fn minigame_row(
    tx: &Connection,
    general: &Value,
    player: &Value,
    prizes: &Value,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Minigame"
             ("id", "workspaceId", "name", "type", "generalSettings", "appearance", "playerSettings",
              "prizeSettings", "winningMessageSettings", "nonWinningMessageSettings")
           VALUES (?1, ?2, 'Spin', 'jackpot', ?3, '{}', ?4, ?5,
                   '{"outcomeMessage":{"enabled":false}}', '{"outcomeMessage":{"enabled":false}}')"#,
        params![
            GAME,
            WS,
            general.to_string(),
            player.to_string(),
            prizes.to_string()
        ],
    )?;
    Ok(GAME)
}

/// A `MinigameContact` row of `contact_id` (`updatedAt` = [`OLD_UPDATED_AT`]).
fn player_row(
    tx: &Connection,
    id: i64,
    contact_id: i64,
    played: i64,
    remaining: i64,
    shares_count: i64,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "MinigameContact"
             ("id", "createdAt", "updatedAt", "minigameId", "contactId", "openedAt", "played", "remaining", "sharesCount")
           VALUES (?1, ?2, ?2, ?3, ?4, ?2, ?5, ?6, ?7)"#,
        params![id, OLD_UPDATED_AT, GAME, contact_id, played, remaining, shares_count],
    )?;
    Ok(id)
}

/// A live leaderboard of the minigame that scores `rule`.
fn leaderboard_row(tx: &Connection, rule: &str) -> Result<i64, DbError> {
    let now = now_ms();
    tx.execute(
        r#"INSERT INTO "Leaderboard"
             ("id", "workspaceId", "name", "startsAt", "endsAt", "status", "platform", "minigameId",
              "scope", "rules", "tagIds", "rewardSettings")
           VALUES (6101, ?1, 'Board', ?2, ?3, 'active', 'gamification', ?4, '{}', ?5, '[]', '{}')"#,
        params![
            WS,
            now - ONE_DAY_MS,
            now + ONE_DAY_MS,
            GAME,
            json!({ rule: { "enabled": true, "points": 1 } }).to_string()
        ],
    )?;
    Ok(6101)
}

/// Workspace `WS` with a minigame (`maxSharesPerPerson` 3 unless
/// `player` is given), the referrer / invitee / player contacts and the tag.
async fn game_db(general: Value, player: Value, prizes: Value) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        minigame_row(tx, &general, &player, &prizes)?;
        common::contact(tx, REFERRER, WS)?;
        common::contact(tx, INVITEE, WS)?;
        common::contact(tx, PLAYER, WS)?;
        common::tag(tx, NEW_FRIEND_TAG, WS, "New friend")?;
        Ok(())
    })
    .await;
    db
}

async fn default_game_db() -> common::TestDb {
    game_db(general_settings(true), never_policy(3), prize_settings(100)).await
}

async fn load_game(db: &Db) -> Minigame {
    minigame::find(db, WS, GAME).await.expect("minigame")
}

fn deps<'a>(db: &'a Db, jobs: &Arc<RecordingEnqueuer>) -> PlayDeps<'a> {
    let jobs: Arc<dyn Enqueuer> = jobs.clone();
    PlayDeps::new(db, Some(jobs)).with_random(|| 0.0)
}

/// `(played, remaining, sharesCount, updatedAt, referrerContactId)` of the
/// contact's play-state row.
async fn player_state(db: &Db, contact_id: i64) -> Option<(i64, i64, i64, i64, Option<i64>)> {
    let rows = common::rows(
        db,
        r#"SELECT "played", "remaining", "sharesCount", "updatedAt", "referrerContactId"
           FROM "MinigameContact" WHERE "contactId" = ?1"#,
        contact_id,
    )
    .await;
    rows.first().map(|row| {
        (
            row["played"].as_i64().unwrap(),
            row["remaining"].as_i64().unwrap(),
            row["sharesCount"].as_i64().unwrap(),
            row["updatedAt"].as_i64().unwrap(),
            row["referrerContactId"].as_i64(),
        )
    })
}

async fn game_counter(db: &Db, column: &'static str) -> i64 {
    let sql: &'static str = match column {
        "sharesCount" => r#"SELECT "sharesCount" FROM "Minigame" WHERE "id" = ?1"#,
        "participantsCount" => r#"SELECT "participantsCount" FROM "Minigame" WHERE "id" = ?1"#,
        _ => unreachable!("column"),
    };
    common::scalar_i64(db, sql, GAME).await.expect("minigame")
}

async fn player_rows(db: &Db) -> i64 {
    common::scalar_i64(
        db,
        r#"SELECT count(*) FROM "MinigameContact" WHERE "minigameId" = ?1"#,
        GAME,
    )
    .await
    .unwrap_or(0)
}

async fn grant(db: &Db, player_settings: Value, referrer: i64, invitee: i64) -> bool {
    play::grant_referral_bonus(
        db,
        ReferralGrant {
            minigame_id: GAME,
            referrer_contact_id: referrer,
            invitee_contact_id: invitee,
            player_settings,
        },
    )
    .await
    .expect("grant")
}

fn leaderboard_calls(jobs: &RecordingEnqueuer) -> Vec<x1_support::AddCall> {
    jobs.calls()
        .into_iter()
        .filter(|call| call.queue == QueueName::Leaderboard)
        .collect()
}

// --- MinigameContactService.grantReferralBonus ------------------------------------

#[tokio::test]
async fn grant_skips_the_grant_when_player_settings_predates_max_shares_per_person() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0).map(|_| ())
    })
    .await;

    let legacy = json!({ "drawsPerPerson": 1, "resetPolicy": "never" });
    assert!(!grant(&db, legacy, REFERRER, INVITEE).await);
    assert_eq!(
        player_state(&db, REFERRER).await,
        Some((1, 0, 0, OLD_UPDATED_AT, None))
    );
    assert_eq!(game_counter(&db, "sharesCount").await, 0);
}

#[tokio::test]
async fn grant_skips_the_grant_when_the_cap_is_0() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0).map(|_| ())
    })
    .await;

    assert!(!grant(&db, never_policy(0), REFERRER, INVITEE).await);
    assert_eq!(
        player_state(&db, REFERRER).await,
        Some((1, 0, 0, OLD_UPDATED_AT, None))
    );
    assert_eq!(game_counter(&db, "sharesCount").await, 0);
}

#[tokio::test]
async fn grant_skips_the_grant_on_self_referral() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0).map(|_| ())
    })
    .await;

    assert!(!grant(&db, never_policy(3), REFERRER, REFERRER).await);
    assert_eq!(
        player_state(&db, REFERRER).await,
        Some((1, 0, 0, OLD_UPDATED_AT, None))
    );
    assert_eq!(game_counter(&db, "sharesCount").await, 0);
}

#[tokio::test]
async fn grant_enforces_the_cap_inside_the_update_predicate_not_by_a_prior_read() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 3, 3)?;
        player_row(tx, 7002, PLAYER, 1, 2, 2)?;
        Ok(())
    })
    .await;

    // At the cap: nothing moves.
    assert!(!grant(&db, never_policy(3), REFERRER, INVITEE).await);
    assert_eq!(
        player_state(&db, REFERRER).await,
        Some((1, 3, 3, OLD_UPDATED_AT, None))
    );
    // One below the cap: the credit lands and stops at the cap.
    assert!(grant(&db, never_policy(3), PLAYER, INVITEE).await);
    assert_eq!(
        player_state(&db, PLAYER).await,
        Some((1, 3, 3, OLD_UPDATED_AT, None))
    );
    assert!(!grant(&db, never_policy(3), PLAYER, REFERRER).await);
    assert_eq!(player_state(&db, PLAYER).await.map(|s| s.2), Some(3));
}

#[tokio::test]
async fn grant_self_assigns_updated_at_so_the_reset_cycle_and_last_played_at_do_not_move() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0).map(|_| ())
    })
    .await;

    assert!(grant(&db, never_policy(3), REFERRER, INVITEE).await);
    assert_eq!(
        player_state(&db, REFERRER).await,
        Some((1, 1, 1, OLD_UPDATED_AT, None))
    );
}

#[tokio::test]
async fn grant_bumps_the_minigame_wide_total_only_when_the_per_contact_credit_lands() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0).map(|_| ())
    })
    .await;

    assert!(grant(&db, never_policy(3), REFERRER, INVITEE).await);
    assert_eq!(game_counter(&db, "sharesCount").await, 1);

    // The player has no play state for this minigame.
    assert!(!grant(&db, never_policy(3), PLAYER, INVITEE).await);
    assert_eq!(game_counter(&db, "sharesCount").await, 1);
}

// --- MinigameContactService.resolvePlayState — referral binding -------------------

fn state_input(contact_id: i64, referrer: Option<i64>, player_settings: Value) -> PlayStateInput {
    PlayStateInput {
        minigame_id: GAME,
        contact_id,
        player_settings,
        contact_inbox_id: None,
        referrer_contact_id: referrer,
    }
}

#[tokio::test]
async fn resolve_play_state_stamps_referrer_contact_id_on_the_insert_path() {
    let db = default_game_db().await;

    let (state, created) =
        play::resolve_opener_play_state(&db, state_input(INVITEE, Some(REFERRER), never_policy(3)))
            .await
            .expect("resolve");

    assert!(created);
    assert_eq!(state.referrer_contact_id, Some(REFERRER));
    assert_eq!(state.remaining, 1);
    assert_eq!(state.played, 0);
    assert_eq!(
        player_state(&db, INVITEE).await.map(|s| s.4),
        Some(Some(REFERRER))
    );
    assert_eq!(game_counter(&db, "participantsCount").await, 1);
}

#[tokio::test]
async fn resolve_play_state_drops_a_self_referral_instead_of_stamping_it() {
    let db = default_game_db().await;

    let (state, created) =
        play::resolve_opener_play_state(&db, state_input(INVITEE, Some(INVITEE), never_policy(3)))
            .await
            .expect("resolve");

    assert!(created);
    assert_eq!(state.referrer_contact_id, None);
    assert_eq!(player_state(&db, INVITEE).await.map(|s| s.4), Some(None));
}

#[tokio::test]
async fn resolve_play_state_never_stamps_a_referrer_onto_an_existing_row() {
    let db = default_game_db().await;
    setup(&db, |tx| player_row(tx, 7001, PLAYER, 1, 0, 0).map(|_| ())).await;

    let (state, created) =
        play::resolve_opener_play_state(&db, state_input(PLAYER, Some(REFERRER), never_policy(3)))
            .await
            .expect("resolve");

    assert!(!created);
    assert_eq!(state.id, 7001);
    assert_eq!(state.referrer_contact_id, None);
    assert_eq!(
        player_state(&db, PLAYER).await,
        Some((1, 0, 0, OLD_UPDATED_AT, None))
    );
    assert_eq!(player_rows(&db).await, 1);
    assert_eq!(game_counter(&db, "participantsCount").await, 0);
}

// --- MinigameContactService.creditSharedLinkReferral ------------------------------

async fn credit(db: &Db, jobs: &Arc<RecordingEnqueuer>, contact_id: i64, referrer: i64) -> bool {
    let game = load_game(db).await;
    play::credit_shared_link_referral(&deps(db, jobs), &game, contact_id, None, referrer)
        .await
        .expect("credit")
}

#[tokio::test]
async fn credit_shared_link_referral_credits_when_this_call_created_the_invitee_row() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0).map(|_| ())
    })
    .await;
    let jobs = RecordingEnqueuer::new();

    assert!(credit(&db, &jobs, INVITEE, REFERRER).await);

    // Per-contact credit, then the minigame-wide total.
    assert_eq!(player_state(&db, REFERRER).await.map(|s| s.2), Some(1));
    assert_eq!(game_counter(&db, "sharesCount").await, 1);
    assert_eq!(
        common::linked_tag_ids(&db, INVITEE).await,
        vec![NEW_FRIEND_TAG]
    );
    assert_eq!(
        player_state(&db, INVITEE).await.map(|s| s.4),
        Some(Some(REFERRER))
    );
}

#[tokio::test]
async fn credit_shared_link_referral_never_credits_when_the_invitee_already_had_a_row() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0)?;
        player_row(tx, 7002, INVITEE, 0, 1, 0)?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();

    assert!(!credit(&db, &jobs, INVITEE, REFERRER).await);

    assert_eq!(player_state(&db, REFERRER).await.map(|s| s.2), Some(0));
    assert_eq!(game_counter(&db, "sharesCount").await, 0);
    assert!(common::linked_tag_ids(&db, INVITEE).await.is_empty());
}

#[tokio::test]
async fn credit_shared_link_referral_returns_early_on_self_referral_without_touching_the_database()
{
    let db = default_game_db().await;
    let jobs = RecordingEnqueuer::new();

    assert!(!credit(&db, &jobs, INVITEE, INVITEE).await);

    assert_eq!(player_rows(&db).await, 0);
    assert_eq!(game_counter(&db, "participantsCount").await, 0);
    assert!(common::linked_tag_ids(&db, INVITEE).await.is_empty());
}

#[tokio::test]
async fn credit_shared_link_referral_still_tags_the_invitee_when_the_referrer_was_already_at_the_cap()
 {
    let db = default_game_db().await;
    // The capped UPDATE matches no row.
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 3, 3).map(|_| ())
    })
    .await;
    let jobs = RecordingEnqueuer::new();

    assert!(!credit(&db, &jobs, INVITEE, REFERRER).await);

    assert_eq!(
        common::linked_tag_ids(&db, INVITEE).await,
        vec![NEW_FRIEND_TAG]
    );
    assert_eq!(player_state(&db, REFERRER).await.map(|s| s.2), Some(3));
    assert_eq!(game_counter(&db, "sharesCount").await, 0);
}

#[tokio::test]
async fn credit_shared_link_referral_never_credits_outside_the_configured_play_window() {
    let db = game_db(
        general_settings(false),
        never_policy(3),
        prize_settings(100),
    )
    .await;
    setup(&db, |tx| {
        player_row(tx, 7001, REFERRER, 1, 0, 0).map(|_| ())
    })
    .await;
    let jobs = RecordingEnqueuer::new();

    assert!(!credit(&db, &jobs, INVITEE, REFERRER).await);

    assert_eq!(player_state(&db, INVITEE).await, None);
    assert_eq!(player_state(&db, REFERRER).await.map(|s| s.2), Some(0));
    assert!(common::linked_tag_ids(&db, INVITEE).await.is_empty());
    assert_eq!(game_counter(&db, "participantsCount").await, 0);
}

// --- minigame leaderboard awards --------------------------------------------------

/// The player's contact inbox (`PLAYER_INBOX`).
async fn with_player_inbox(db: &Db) -> PlayContactInbox {
    setup(db, |tx| {
        common::inbox(tx, INBOX, WS, "messenger")?;
        common::contact_inbox(tx, PLAYER_INBOX, PLAYER, INBOX, "messenger", "psid-1")?;
        Ok(())
    })
    .await;
    PlayContactInbox {
        id: PLAYER_INBOX,
        contact_id: PLAYER,
    }
}

#[tokio::test]
async fn leaderboard_a_prize_win_awards_minigame_win_keyed_by_the_minigame_play_row() {
    let db = default_game_db().await;
    setup(&db, |tx| leaderboard_row(tx, "minigameWin").map(|_| ())).await;
    let inbox = with_player_inbox(&db).await;
    let jobs = RecordingEnqueuer::new();
    let game = load_game(&db).await;

    let (_, result) = play::record_play_and_dispatch(&deps(&db, &jobs), &game, PLAYER, inbox, None)
        .await
        .expect("play");
    assert!(matches!(result, play::PlayResult::Prize(_)));

    let rows = common::rows(
        &db,
        r#"SELECT "id", "createdAt" FROM "MinigamePlay" WHERE "contactId" = ?1"#,
        PLAYER,
    )
    .await;
    assert_eq!(rows.len(), 1);
    let play_id = rows[0]["id"].as_i64().unwrap();
    let played_at = rows[0]["createdAt"].as_i64().unwrap();

    let calls = leaderboard_calls(&jobs);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "award");
    assert_eq!(
        calls[0].data,
        json!({
            "type": "award",
            "data": {
                "workspaceId": WS.to_string(),
                "contactId": PLAYER.to_string(),
                "inboxId": null,
                "minigameId": GAME.to_string(),
                "rule": "minigameWin",
                "sourceRef": play_id.to_string(),
                "eventAt": iso(played_at),
            }
        })
    );
    assert_eq!(
        calls[0].opts.job_id.as_deref(),
        Some(format!("lb-award-{WS}-mg{GAME}-minigameWin-{play_id}").as_str())
    );
    assert_eq!(
        calls[0].opts.remove_on_complete,
        Some(Retention::KeepAge {
            age_secs: 24 * 60 * 60,
            count: None
        })
    );
}

#[tokio::test]
async fn leaderboard_a_losing_play_awards_nothing() {
    let db = game_db(general_settings(true), never_policy(3), prize_settings(0)).await;
    setup(&db, |tx| leaderboard_row(tx, "minigameWin").map(|_| ())).await;
    let inbox = with_player_inbox(&db).await;
    let jobs = RecordingEnqueuer::new();
    let game = load_game(&db).await;

    let (_, result) = play::record_play_and_dispatch(&deps(&db, &jobs), &game, PLAYER, inbox, None)
        .await
        .expect("play");

    assert_eq!(result, play::PlayResult::NonWinning);
    assert!(leaderboard_calls(&jobs).is_empty());
}

#[tokio::test]
async fn leaderboard_a_credited_share_awards_in_game_share_to_the_referrer_keyed_by_the_invitee() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        leaderboard_row(tx, "inGameShare")?;
        player_row(tx, 7001, REFERRER, 1, 0, 0)?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();

    assert!(credit(&db, &jobs, INVITEE, REFERRER).await);

    let joined_at = common::scalar_i64(
        &db,
        r#"SELECT "createdAt" FROM "MinigameContact" WHERE "contactId" = ?1"#,
        INVITEE,
    )
    .await
    .expect("invitee row");
    let calls = leaderboard_calls(&jobs);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].data["data"],
        json!({
            "workspaceId": WS.to_string(),
            "contactId": REFERRER.to_string(),
            "inboxId": null,
            "minigameId": GAME.to_string(),
            "rule": "inGameShare",
            "sourceRef": INVITEE.to_string(),
            "eventAt": iso(joined_at),
        })
    );
}

#[tokio::test]
async fn leaderboard_no_share_credit_cap_reached_awards_nothing() {
    let db = default_game_db().await;
    setup(&db, |tx| {
        leaderboard_row(tx, "inGameShare")?;
        player_row(tx, 7001, REFERRER, 1, 3, 3)?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();

    assert!(!credit(&db, &jobs, INVITEE, REFERRER).await);
    assert!(leaderboard_calls(&jobs).is_empty());
}

// --- minigame outcome message — text mode ----------------------------------------

/// The player's contact inbox and DM conversation.
async fn outcome_db() -> common::TestDb {
    let db = default_game_db().await;
    with_player_inbox(&db).await;
    setup(&db, |tx| {
        common::conversation(tx, CONVERSATION, WS, PLAYER)?;
        Ok(())
    })
    .await;
    db
}

fn resolver(f: fn(String) -> Result<String, String>) -> ContactVariableResolver {
    Arc::new(move |text| Box::pin(async move { f(text) }))
}

async fn send_win(
    db: &Db,
    jobs: &Arc<RecordingEnqueuer>,
    prize_name: &str,
    text: &str,
    resolve: Option<ContactVariableResolver>,
) {
    play::send_win_message(
        &deps(db, jobs),
        OutcomeMessage {
            workspace_id: WS,
            contact_id: PLAYER,
            contact_inbox: PlayContactInbox {
                id: PLAYER_INBOX,
                contact_id: PLAYER,
            },
            prize_name: prize_name.to_string(),
            outcome_message: json!({ "enabled": true, "mode": "text", "text": text }),
            resolver: resolve,
        },
    )
    .await;
}

async fn sent_text(db: &Db) -> Option<String> {
    common::scalar_text(
        db,
        r#"SELECT "text" FROM "Message" WHERE "conversationId" = ?1"#,
        CONVERSATION,
    )
    .await
}

fn chat_calls(jobs: &RecordingEnqueuer) -> Vec<x1_support::AddCall> {
    jobs.calls()
        .into_iter()
        .filter(|call| call.queue == QueueName::Chat)
        .collect()
}

#[tokio::test]
async fn outcome_text_spins_substitutes_the_prize_name_then_resolves_contact_variables() {
    let db = outcome_db().await;
    let jobs = RecordingEnqueuer::new();

    send_win(
        &db,
        &jobs,
        "Áo thun",
        "{Chúc mừng|Xin chúc mừng} {{first_name}}! {{prize_name}}",
        Some(resolver(|text| Ok(text.replace("{{first_name}}", "Nam")))),
    )
    .await;

    assert_eq!(
        sent_text(&db).await.as_deref(),
        Some("Chúc mừng Nam! Áo thun")
    );
}

#[tokio::test]
async fn outcome_text_never_spins_the_prize_name_it_substitutes() {
    let db = outcome_db().await;
    let jobs = RecordingEnqueuer::new();

    send_win(
        &db,
        &jobs,
        "Combo {áo|quần}",
        "Bạn trúng {{prize_name}}",
        None,
    )
    .await;

    assert_eq!(
        sent_text(&db).await.as_deref(),
        Some("Bạn trúng Combo {áo|quần}")
    );
}

#[tokio::test]
async fn outcome_text_resolves_prize_name_with_no_resolver_injected() {
    let db = outcome_db().await;
    let jobs = RecordingEnqueuer::new();

    send_win(&db, &jobs, "Áo thun", "Bạn trúng {{prize_name}}", None).await;

    assert_eq!(sent_text(&db).await.as_deref(), Some("Bạn trúng Áo thun"));
}

#[tokio::test]
async fn outcome_text_leaves_a_contact_variable_literal_with_no_resolver_injected() {
    let db = outcome_db().await;
    let jobs = RecordingEnqueuer::new();

    send_win(&db, &jobs, "Áo thun", "Chào {{first_name}}", None).await;

    assert_eq!(sent_text(&db).await.as_deref(), Some("Chào {{first_name}}"));
}

#[tokio::test]
async fn outcome_text_falls_back_to_the_unresolved_text_when_the_resolver_throws() {
    let db = outcome_db().await;
    let jobs = RecordingEnqueuer::new();
    let log = RecordingLog::install();

    send_win(
        &db,
        &jobs,
        "Áo thun",
        "{Chúc mừng|Xin chúc mừng} {{first_name}}",
        Some(resolver(|_| Err("db down".to_string()))),
    )
    .await;

    assert_eq!(
        sent_text(&db).await.as_deref(),
        Some("Chúc mừng {{first_name}}")
    );
    assert!(
        log.entries()
            .iter()
            .any(|entry| entry.level == tracing::Level::WARN)
    );
    // The message still ships.
    assert_eq!(chat_calls(&jobs).len(), 1);
}

#[tokio::test]
async fn outcome_text_persists_the_same_text_it_enqueues() {
    let db = outcome_db().await;
    let jobs = RecordingEnqueuer::new();

    send_win(&db, &jobs, "Áo thun", "{Chúc mừng|Xin chúc mừng} bạn", None).await;

    assert_eq!(sent_text(&db).await.as_deref(), Some("Chúc mừng bạn"));
    let message_id = common::scalar_i64(
        &db,
        r#"SELECT "id" FROM "Message" WHERE "conversationId" = ?1"#,
        CONVERSATION,
    )
    .await
    .expect("message row");
    let calls = chat_calls(&jobs);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "sendChannelMessage");
    assert_eq!(calls[0].data["type"], json!("sendChannelMessage"));
    assert_eq!(calls[0].data["data"]["message"]["id"], json!(message_id));
    assert_eq!(
        calls[0].data["data"]["message"]["text"],
        json!("Chúc mừng bạn")
    );
    assert_eq!(
        calls[0].data["data"]["conversation"]["id"],
        json!(CONVERSATION)
    );
    assert_eq!(
        calls[0].data["data"]["contactInbox"]["id"],
        json!(PLAYER_INBOX)
    );
}
