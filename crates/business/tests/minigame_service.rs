//! Ports of `packages/business/__tests__/minigame-service-update.test.ts`,
//! `minigame-contact-service-scoping.test.ts` and the `deriveRemaining`
//! describe of `minigame-referral-bonus.test.ts`. Adapted: the mocked
//! `select ... for update` / `update ... set` become a real `Minigame` row;
//! "set was called with" becomes the stored JSON. The play-runtime tests
//! (referral grants and binding, outcome messages, leaderboard awards) move to
//! the minigame play unit (test map).

mod common;

use std::collections::HashMap;

use chatbotx_business::minigame::contact::{self, ListPlayers, derive_remaining};
use chatbotx_business::minigame::{self, MinigameData, MinigamePatch};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};

const GAME: i64 = 5001;

fn minigame_row(tx: &Connection, id: i64, ws: i64, name: &str) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Minigame"
             ("id", "workspaceId", "name", "type", "generalSettings", "appearance", "playerSettings",
              "prizeSettings", "winningMessageSettings", "nonWinningMessageSettings")
           VALUES (?1, ?2, ?3, 'jackpot', ?4, '{"theme":"light"}', '{"drawsPerPerson":1}',
                   '{"prizes":[{"id":"p1","quantity":3}],"nonWinning":{"loseRate":25}}',
                   '{"enabled":false}', '{"enabled":false}')"#,
        params![id, ws, name, json!({ "name": name }).to_string()],
    )?;
    Ok(id)
}

async fn game_db() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        minigame_row(tx, GAME, WS, "Current name")?;
        Ok(())
    })
    .await;
    db
}

fn data(name: &str, prize_settings: Value) -> MinigameData {
    MinigameData {
        r#type: "jackpot".into(),
        general_settings: json!({ "name": name }),
        appearance: json!({}),
        player_settings: json!({}),
        prize_settings,
        winning_message_settings: json!({}),
        non_winning_message_settings: json!({}),
    }
}

async fn stored(db: &Db, column: &'static str) -> Value {
    let sql: &'static str = match column {
        "name" => r#"SELECT "name" FROM "Minigame" WHERE "id" = ?1"#,
        "generalSettings" => r#"SELECT "generalSettings" FROM "Minigame" WHERE "id" = ?1"#,
        "prizeSettings" => r#"SELECT "prizeSettings" FROM "Minigame" WHERE "id" = ?1"#,
        _ => unreachable!("column"),
    };
    let text = common::scalar_text(db, sql, GAME)
        .await
        .expect("stored row");
    if column == "name" {
        Value::String(text)
    } else {
        serde_json::from_str(&text).expect("stored JSON")
    }
}

fn baseline(p1: f64) -> Option<HashMap<String, Option<f64>>> {
    Some(HashMap::from([("p1".to_string(), Some(p1))]))
}

// --- MinigameService.update — prize quantity reconciliation ------------------------

#[tokio::test]
async fn update_preserves_the_db_live_quantity_when_the_admin_did_not_touch_that_prize() {
    let db = game_db().await;
    minigame::update(
        &db,
        WS,
        GAME,
        data(
            "Test minigame",
            json!({ "prizes": [{ "id": "p1", "quantity": 10 }], "nonWinning": { "loseRate": 25 } }),
        ),
        baseline(10.0),
    )
    .await
    .expect("update");
    assert_eq!(
        stored(&db, "prizeSettings").await["prizes"][0]["quantity"],
        json!(3)
    );
}

#[tokio::test]
async fn update_honors_the_admin_new_quantity_when_they_changed_it() {
    let db = game_db().await;
    minigame::update(
        &db,
        WS,
        GAME,
        data(
            "Test minigame",
            json!({ "prizes": [{ "id": "p1", "quantity": 50 }], "nonWinning": { "loseRate": 25 } }),
        ),
        baseline(10.0),
    )
    .await
    .expect("update");
    assert_eq!(
        stored(&db, "prizeSettings").await["prizes"][0]["quantity"],
        json!(50)
    );
}

#[tokio::test]
async fn update_throws_not_found_when_the_row_does_not_exist_in_this_workspace() {
    let db = game_db().await;
    let error = minigame::update(
        &db,
        OTHER_WS,
        GAME,
        data(
            "Test minigame",
            json!({ "prizes": [], "nonWinning": { "loseRate": 100 } }),
        ),
        Some(HashMap::new()),
    )
    .await
    .expect_err("foreign row");
    assert_eq!(error.message(), "Minigame not found");
    assert_eq!(error.status(), Some(404));
}

#[tokio::test]
async fn update_honors_submitted_quantities_verbatim_when_original_prize_quantities_is_null() {
    let db = game_db().await;
    minigame::update(
        &db,
        WS,
        GAME,
        data(
            "Test minigame",
            json!({ "prizes": [{ "id": "p1" }], "nonWinning": { "loseRate": 25 } }),
        ),
        None,
    )
    .await
    .expect("update");
    let prizes = stored(&db, "prizeSettings").await;
    assert_eq!(prizes["prizes"][0].get("quantity"), None);
}

#[tokio::test]
async fn update_maps_a_unique_name_violation_to_a_409_chatbotx_exception() {
    let db = game_db().await;
    setup(&db, |tx| {
        minigame_row(tx, 5002, WS, "Taken")?;
        Ok(())
    })
    .await;
    let error = minigame::update(
        &db,
        WS,
        GAME,
        data(
            "Taken",
            json!({ "prizes": [], "nonWinning": { "loseRate": 100 } }),
        ),
        Some(HashMap::new()),
    )
    .await
    .expect_err("taken name");
    assert_eq!(error.code(), Some("nameAlreadyExists"));
    assert_eq!(error.status(), Some(409));
    assert_eq!(error.message(), "Minigame name already exists");
}

// --- MinigameService.updatePartial — partial merge over the current row ------------

#[tokio::test]
async fn update_partial_omitting_prize_settings_preserves_the_db_live_play_decremented_quantity_verbatim()
 {
    let db = game_db().await;
    minigame::update_partial(
        &db,
        WS,
        GAME,
        MinigamePatch {
            general_settings: Some(json!({ "name": "Renamed" })),
            ..MinigamePatch::default()
        },
    )
    .await
    .expect("updatePartial");
    assert_eq!(
        stored(&db, "generalSettings").await["name"],
        json!("Renamed")
    );
    assert_eq!(stored(&db, "name").await, json!("Renamed"));
    assert_eq!(
        stored(&db, "prizeSettings").await,
        json!({ "prizes": [{ "id": "p1", "quantity": 3 }], "nonWinning": { "loseRate": 25 } })
    );
}

#[tokio::test]
async fn update_partial_sending_prize_settings_replaces_it_wholesale_not_deep_merged() {
    let db = game_db().await;
    minigame::update_partial(
        &db,
        WS,
        GAME,
        MinigamePatch {
            prize_settings: Some(
                json!({ "prizes": [{ "id": "p2", "quantity": 5 }], "nonWinning": { "loseRate": 50 } }),
            ),
            ..MinigamePatch::default()
        },
    )
    .await
    .expect("updatePartial");
    assert_eq!(
        stored(&db, "prizeSettings").await["prizes"],
        json!([{ "id": "p2", "quantity": 5 }])
    );
}

#[tokio::test]
async fn update_partial_throws_not_found_when_the_row_does_not_exist_in_this_workspace() {
    let db = game_db().await;
    let error = minigame::update_partial(
        &db,
        WS,
        9999,
        MinigamePatch {
            general_settings: Some(json!({ "name": "Renamed" })),
            ..MinigamePatch::default()
        },
    )
    .await
    .expect_err("missing row");
    assert_eq!(error.message(), "Minigame not found");
}

#[tokio::test]
async fn update_partial_maps_a_unique_name_violation_to_a_409_chatbotx_exception() {
    let db = game_db().await;
    setup(&db, |tx| {
        minigame_row(tx, 5002, WS, "Taken")?;
        Ok(())
    })
    .await;
    let error = minigame::update_partial(
        &db,
        WS,
        GAME,
        MinigamePatch {
            general_settings: Some(json!({ "name": "Taken" })),
            ..MinigamePatch::default()
        },
    )
    .await
    .expect_err("taken name");
    assert_eq!(error.code(), Some("nameAlreadyExists"));
    assert_eq!(error.status(), Some(409));
}

// --- MinigameContactService — indirect tenancy guard -------------------------------

async fn foreign_game_db() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        minigame_row(tx, GAME, OTHER_WS, "Foreign")?;
        common::contact(tx, 3001, OTHER_WS)?;
        tx.execute(
            r#"INSERT INTO "MinigameContact" ("id", "minigameId", "contactId", "openedAt", "played", "remaining")
               VALUES (6001, ?1, 3001, 1000, 1, 0)"#,
            params![GAME],
        )?;
        tx.execute(
            r#"INSERT INTO "MinigamePlay" ("id", "minigameId", "contactId", "isWinning")
               VALUES (6101, ?1, 3001, 1)"#,
            params![GAME],
        )?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn list_plays_rejects_a_minigame_id_belonging_to_another_workspace_before_reading_any_play_rows()
 {
    let db = foreign_game_db().await;
    let error = contact::list_plays(&db, WS, GAME, 3001)
        .await
        .expect_err("foreign minigame");
    assert_eq!(error.message(), "Minigame not found");
    assert_eq!(error.status(), Some(404));
    // The owner workspace reads the same rows.
    let plays = contact::list_plays(&db, OTHER_WS, GAME, 3001)
        .await
        .expect("listPlays");
    assert_eq!(plays.len(), 1);
}

#[tokio::test]
async fn list_players_rejects_a_minigame_id_belonging_to_another_workspace_before_reading_any_player_rows()
 {
    let db = foreign_game_db().await;
    let error = contact::list(
        &db,
        ListPlayers {
            workspace_id: WS,
            minigame_id: GAME,
            ..ListPlayers::default()
        },
    )
    .await
    .expect_err("foreign minigame");
    assert_eq!(error.message(), "Minigame not found");
    let page = contact::list(
        &db,
        ListPlayers {
            workspace_id: OTHER_WS,
            minigame_id: GAME,
            ..ListPlayers::default()
        },
    )
    .await
    .expect("list");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.page_count, 1);
}

// --- deriveRemaining -------------------------------------------------------------

fn never_policy(draws: i64, max_shares: i64) -> Value {
    json!({ "drawsPerPerson": draws, "maxSharesPerPerson": max_shares, "resetPolicy": "never" })
}

#[test]
fn derive_remaining_adds_earned_bonus_draws_on_top_of_the_configured_allowance() {
    assert_eq!(derive_remaining(&never_policy(1, 3), 1, 2), 2);
}

#[test]
fn derive_remaining_clamps_the_bonus_at_the_cap() {
    assert_eq!(derive_remaining(&never_policy(1, 3), 0, 5), 4);
}

#[test]
fn derive_remaining_lowering_the_cap_claws_back_uncredited_bonus_draws() {
    assert_eq!(derive_remaining(&never_policy(1, 1), 0, 3), 2);
}

#[test]
fn derive_remaining_treats_legacy_player_settings_with_no_max_shares_per_person_as_no_bonus() {
    let legacy = json!({ "drawsPerPerson": 2, "resetPolicy": "never" });
    assert_eq!(derive_remaining(&legacy, 0, 4), 2);
}

#[test]
fn derive_remaining_never_goes_negative() {
    assert_eq!(derive_remaining(&never_policy(1, 2), 10, 1), 0);
}
