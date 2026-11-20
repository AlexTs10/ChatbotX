//! Ports of `packages/business/__tests__/leaderboard-helpers.test.ts`,
//! `leaderboard-participants-security.test.ts` and the status / reward guards
//! of `leaderboard-scoring.test.ts` and `leaderboard-rewards.test.ts`.
//! Adapted: the mocked repositories become a real migrated database; the
//! Redis ranking assertions are gone (the ranking is the SQLite
//! `LeaderboardParticipant` table, DECISIONS.md).

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::leaderboard::service::{self, Deps};
use chatbotx_business::leaderboard::{
    UpsertInput, assert_valid_config, day_key, day_start, default_rules, event_id, mask_name,
    mask_phone,
};
use chatbotx_db::repo::parse_iso;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use chatbotx_jobs::{Enqueuer, QueueName};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const BOARD: i64 = 6201;
const CONTACT: i64 = 3201;
const FOREIGN_CONTACT: i64 = 3202;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn at(text: &str) -> i64 {
    parse_iso(text).expect("iso")
}

fn keys(rules: &Value) -> Vec<String> {
    rules.as_object().expect("rules").keys().cloned().collect()
}

fn base_config() -> UpsertInput {
    UpsertInput {
        platform: "messenger".into(),
        inbox_id: Some(10),
        rules: default_rules("messenger"),
        scope: json!({ "postScope": "all", "postIds": [], "telegramChatIds": [] }),
        starts_at: at("2026-10-01T00:00:00Z"),
        ends_at: at("2026-11-01T00:00:00Z"),
        ..UpsertInput::default()
    }
}

fn rejected(input: &UpsertInput) -> String {
    assert_valid_config(input).expect_err("rejected").message()
}

// --- leaderboard-helpers.test.ts ---------------------------------------------

#[test]
fn the_day_restarts_at_the_reset_time() {
    assert_eq!(day_key(at("2026-10-12T21:59:00Z"), "22:00"), "2026-10-11");
    assert_eq!(day_key(at("2026-10-12T22:00:00Z"), "22:00"), "2026-10-12");
    assert_eq!(
        day_start(at("2026-10-13T03:00:00Z"), "22:00"),
        at("2026-10-12T22:00:00Z")
    );
}

#[test]
fn the_event_id_is_stable_and_positive() {
    let a = event_id(&["1", "comments", "c-1"]);
    assert_eq!(a, event_id(&["1", "comments", "c-1"]));
    assert_ne!(a, event_id(&["1", "comments", "c-2"]));
    assert!(a > 0);
}

#[test]
fn masks_participant_names() {
    assert_eq!(mask_name(Some("Nguyễn Văn An")), "Nguyễn V*** An");
    assert_eq!(mask_name(Some("Trần Bình")), "Trần *** Bình");
    assert_eq!(mask_name(Some("Hoa")), "H***");
    assert_eq!(mask_name(None), "***");
}

#[test]
fn default_rules_follow_the_spec_per_platform() {
    let messenger = default_rules("messenger");
    assert_eq!(messenger["comments"]["enabled"], true);
    assert_eq!(messenger["comments"]["points"], 30);
    assert_eq!(messenger["comments"]["minChars"], 30);
    assert_eq!(messenger["comments"]["bonusMinutes"], 60);
    assert_eq!(messenger["liveComments"]["points"], 300);
    let instagram = default_rules("instagram");
    assert_eq!(instagram["storyMention"]["points"], 100);
    assert_eq!(instagram["storyMention"]["maxPerDay"], 2);
    let mut email = keys(&default_rules("email"));
    email.sort();
    assert_eq!(email, ["emailClick", "emailOpen"]);
}

#[test]
fn accepts_the_default_config() {
    assert!(assert_valid_config(&base_config()).is_ok());
}

#[test]
fn rejects_a_rule_the_platform_does_not_offer() {
    let mut config = base_config();
    config.rules["emailOpen"] = config.rules["comments"].clone();
    assert!(rejected(&config).contains("not available"));
}

#[test]
fn rejects_an_enabled_rule_without_points() {
    let mut config = base_config();
    config.rules["keywords"]["enabled"] = json!(true);
    config.rules["keywords"]["points"] = json!(0);
    assert!(rejected(&config).contains("points"));
}

#[test]
fn rejects_comment_only_fields_on_other_rules() {
    let mut config = base_config();
    config.rules["liveComments"]["minChars"] = json!(10);
    assert!(rejected(&config).contains("comments rule"));
}

#[test]
fn requires_a_channel_or_a_minigame_for_gamification() {
    let no_inbox = UpsertInput {
        inbox_id: None,
        ..base_config()
    };
    assert!(rejected(&no_inbox).contains("channel"));
    let game = UpsertInput {
        platform: "gamification".into(),
        rules: default_rules("gamification"),
        ..base_config()
    };
    assert!(rejected(&game).contains("minigame"));
}

#[test]
fn rejects_an_end_before_the_start_and_campaigns_over_three_years() {
    let config = base_config();
    let same = UpsertInput {
        ends_at: config.starts_at,
        ..config.clone()
    };
    assert!(rejected(&same).contains("after start"));
    let long = UpsertInput {
        ends_at: at("2030-01-01T00:00:00Z"),
        ..config
    };
    assert!(rejected(&long).contains("3 years"));
}

#[test]
fn tiktok_scores_comments_and_keywords_threads_comments_only() {
    let mut tiktok = keys(&default_rules("tiktok"));
    tiktok.sort();
    assert_eq!(tiktok, ["comments", "keywords"]);
    let threads = default_rules("threads");
    assert_eq!(keys(&threads), ["comments"]);
    assert_eq!(threads["comments"]["minChars"], 20);
    assert!(threads["comments"]["bonusMinutes"].is_null());
    let config = UpsertInput {
        platform: "threads".into(),
        rules: threads,
        ..base_config()
    };
    assert!(assert_valid_config(&config).is_ok());
}

#[test]
fn golden_hour_and_specific_posts_need_tracked_posts() {
    let mut rules = default_rules("tiktok");
    rules["comments"]["bonusPoints"] = json!(10);
    rules["comments"]["bonusMinutes"] = json!(60);
    let tiktok = UpsertInput {
        platform: "tiktok".into(),
        rules,
        ..base_config()
    };
    assert!(rejected(&tiktok).contains("golden-hour bonus is not available"));
    let threads = UpsertInput {
        platform: "threads".into(),
        rules: default_rules("threads"),
        scope: json!({ "postScope": "specific", "postIds": ["1"], "telegramChatIds": [] }),
        ..base_config()
    };
    assert!(rejected(&threads).contains("specific posts is not available"));
}

#[test]
fn the_specific_post_scope_needs_posts() {
    let config = UpsertInput {
        scope: json!({ "postScope": "specific", "postIds": [], "telegramChatIds": [] }),
        ..base_config()
    };
    assert!(rejected(&config).contains("post"));
}

// --- leaderboard-participants-security.test.ts --------------------------------

#[test]
fn masks_participant_phones() {
    assert_eq!(
        mask_phone(Some("+84 912 345 678")).as_deref(),
        Some("••••••••678")
    );
    assert_eq!(mask_phone(Some("12")).as_deref(), Some("••"));
    assert_eq!(mask_phone(None), None);
}

fn board_row(tx: &Connection, status: &str, rules: &Value, rewards: &Value) -> Result<(), DbError> {
    let now = chatbotx_core::js::now_ms();
    tx.execute(
        r#"INSERT INTO "Leaderboard"
             ("id", "workspaceId", "name", "startsAt", "endsAt", "status", "platform",
              "scope", "rules", "tagIds", "rewardSettings")
           VALUES (?1, ?2, 'Board', ?3, ?4, ?5, 'messenger', '{}', ?6, '[]', ?7)"#,
        params![
            BOARD,
            WS,
            now - DAY_MS,
            now + DAY_MS,
            status,
            rules.to_string(),
            rewards.to_string()
        ],
    )?;
    Ok(())
}

async fn board_db(status: &'static str, rules: Value, rewards: Value) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        board_row(tx, status, &rules, &rewards)?;
        common::contact(tx, CONTACT, WS)?;
        common::contact(tx, FOREIGN_CONTACT, OTHER_WS)?;
        Ok(())
    })
    .await;
    db
}

fn enabled_rules() -> Value {
    json!({ "comments": { "enabled": true, "points": 5 } })
}

fn deps(db: &Db, jobs: Option<Arc<RecordingEnqueuer>>) -> Deps<'_> {
    Deps {
        db,
        jobs: jobs.map(|j| j as Arc<dyn Enqueuer>),
    }
}

#[tokio::test]
async fn a_manual_adjust_rejects_a_contact_of_another_workspace() {
    let db = board_db("active", enabled_rules(), json!({})).await;
    let error = service::adjust(&db, WS, BOARD, FOREIGN_CONTACT, 1, "probe", None)
        .await
        .expect_err("foreign contact");
    assert_eq!(error.code(), Some("notFound"));
    let stored = common::scalar_i64(
        &db,
        r#"SELECT COUNT(*) FROM "LeaderboardParticipant" WHERE "leaderboardId" = ?1"#,
        BOARD,
    )
    .await;
    assert_eq!(stored, Some(0));
    let score = service::adjust(&db, WS, BOARD, CONTACT, 7, "bonus", None)
        .await
        .expect("own contact");
    assert_eq!(score, 7);
}

// --- leaderboard-scoring.test.ts: leaderboardService status transitions -------

#[tokio::test]
async fn publish_requires_an_enabled_rule() {
    let db = board_db(
        "draft",
        json!({ "comments": { "enabled": false, "points": 5 } }),
        json!({}),
    )
    .await;
    let error = service::publish(&db, WS, BOARD).await.expect_err("no rule");
    assert_eq!(error.code(), Some("validation"));
    assert!(error.message().contains("scoring rule"));
}

#[tokio::test]
async fn a_transition_the_status_does_not_allow_is_refused() {
    let db = board_db("draft", enabled_rules(), json!({})).await;
    let error = service::pause(&db, WS, BOARD).await.expect_err("draft");
    assert_eq!(error.code(), Some("leaderboardActionNotAllowed"));
}

#[tokio::test]
async fn finish_queues_automatic_rewards_after_the_late_event_grace() {
    let db = board_db(
        "active",
        enabled_rules(),
        json!({ "flowId": "77", "topN": 3, "autoOnFinish": true }),
    )
    .await;
    let jobs = RecordingEnqueuer::new();
    let row = service::finish(&deps(&db, Some(jobs.clone())), WS, BOARD)
        .await
        .expect("finish");
    assert_eq!(row.status, "finished");
    assert!(row.finished_at.is_some());
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].queue, QueueName::Leaderboard);
    assert_eq!(calls[0].data["type"], "triggerRewards");
    assert_eq!(calls[0].opts.job_id.as_deref(), Some("lb-rewards-6201"));
    assert_eq!(
        calls[0].opts.delay_ms,
        Some(chatbotx_business::leaderboard::LATE_EVENT_GRACE_MS as u64)
    );
}

// --- leaderboard-rewards.test.ts ----------------------------------------------

#[tokio::test]
async fn rewards_refuse_a_leaderboard_that_is_not_finished() {
    let db = board_db(
        "active",
        enabled_rules(),
        json!({ "flowId": "77", "topN": 3 }),
    )
    .await;
    let error = service::trigger_rewards(&deps(&db, None), WS, BOARD, false)
        .await
        .expect_err("active");
    assert_eq!(error.code(), Some("leaderboardNotFinished"));
}

// --- export-leaderboard-participants.test.ts ----------------------------------

fn participant(tx: &Connection, contact: i64, score: i64, banned: bool) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "LeaderboardParticipant"
             ("leaderboardId", "contactId", "workspaceId", "score", "isBanned", "dailyCounts", "breakdown")
           VALUES (?1, ?2, ?3, ?4, ?5, '{}', '{"comments":2,"keywords":0}')"#,
        params![BOARD, contact, WS, score, banned],
    )?;
    Ok(())
}

fn export_job(include_contact_info: bool) -> service::ExportJob {
    service::ExportJob {
        workspace_id: WS,
        leaderboard_id: BOARD,
        file_id: 1,
        output_path: "exports/board.csv".into(),
        include_contact_info,
    }
}

#[tokio::test]
async fn the_export_is_a_bom_csv_that_ranks_only_unbanned_rows() {
    let db = board_db("active", enabled_rules(), json!({})).await;
    setup(&db, |tx| {
        common::contact(tx, 3203, WS)?;
        participant(tx, CONTACT, 90, false)?;
        participant(tx, 3203, 40, false)?;
        Ok(())
    })
    .await;
    setup(&db, |tx| {
        common::contact(tx, 3204, WS)?;
        participant(tx, 3204, 500, true)
    })
    .await;
    let (csv, total) = service::export_csv(&db, &export_job(false))
        .await
        .expect("csv");
    assert_eq!(total, 3);
    assert!(csv.starts_with('\u{feff}'));
    let lines: Vec<&str> = csv.trim_start_matches('\u{feff}').lines().collect();
    assert!(
        lines[0].starts_with("\"Rank\",\"Contact ID\",\"Name\",\"Score\""),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with(&format!("\"1\",\"{CONTACT}\"")),
        "{}",
        lines[1]
    );
    assert!(lines[1].contains("\"comments: 2\""), "{}", lines[1]);
    assert!(lines[2].starts_with("\"2\",\"3203\""), "{}", lines[2]);
    assert!(lines[3].starts_with("\"\",\"3204\""), "{}", lines[3]);
    assert!(lines[3].contains("\"yes\""));
}

#[tokio::test]
async fn the_export_adds_phone_and_email_only_with_contact_info() {
    let db = board_db("active", enabled_rules(), json!({})).await;
    let (with, _) = service::export_csv(&db, &export_job(true))
        .await
        .expect("csv");
    assert!(with.contains("\"Phone\",\"Email\""));
    let (without, _) = service::export_csv(&db, &export_job(false))
        .await
        .expect("csv");
    assert!(!without.contains("\"Phone\""));
}

// Parity `52-leaderboards-export-rewards`: `ContactInbox` has no workspace
// column; the winner's channel identity is found through its contact.
#[tokio::test]
async fn rewards_queue_the_flow_for_a_winner_with_a_conversation() {
    let db = board_db(
        "finished",
        enabled_rules(),
        json!({ "flowId": "77", "topN": 2 }),
    )
    .await;
    setup(&db, |tx| {
        common::inbox(tx, 10, WS, "messenger")?;
        common::contact_inbox(tx, 4201, CONTACT, 10, "messenger", "psid-1")?;
        common::conversation(tx, 5201, WS, CONTACT)?;
        participant(tx, CONTACT, 30, false)
    })
    .await;
    let jobs = RecordingEnqueuer::new();
    let out = service::trigger_rewards(&deps(&db, Some(jobs.clone())), WS, BOARD, false)
        .await
        .expect("rewards");
    assert_eq!(out["sent"], 1, "{out}");
    assert_eq!(out["failed"], json!([]));
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].data["data"]["contactInboxId"], "4201");
}
