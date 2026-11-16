//! Port of `apps/builder/__tests__/minigames-public-api.test.ts`. Adapted: the
//! captured oRPC handlers become `handlers::dispatch` calls over a real
//! migrated database; "the service was called with" becomes the stored rows /
//! the response; the mocked `buildMinigamePlayUrl` is the real play URL on the
//! fixture app URL (`http://app`).

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

fn body(name: &str) -> Value {
    json!({
        "type": "luckyWheel",
        "generalSettings": {
            "name": name,
            "playedAtFrom": "2026-01-01T00:00:00.000Z",
            "playedAtTo": "2027-12-31T23:59:59.000Z",
        },
        "appearance": { "backgroundImage": {}, "prizeDescriptionImage": {}, "startButtonImage": {} },
        "playerSettings": { "resetPolicy": "never" },
        "prizeSettings": {
            "prizes": [{
                "id": "prize-1",
                "name": "Free coffee",
                "icon": { "mode": "url", "url": "https://example.com/c.png" },
                "winRate": 30,
                "quantity": 5,
            }],
            "nonWinning": { "title": "Try again", "loseRate": 70, "loseImage": {} },
        },
        "winningMessageSettings": { "title": "You won {{prize_name}}" },
        "nonWinningMessageSettings": {},
    })
}

fn play_url(id: &str) -> String {
    format!("http://app/minigames?minigameId={id}&token={{{{minigame_play_token}}}}")
}

async fn new_game(f: &Fixture, ws: i64, name: &str) -> Value {
    let (status, created) = call(f, "minigames.create", ws, &[], None, Some(body(name))).await;
    assert_eq!(status, 201, "{created}");
    created
}

fn id_of(game: &Value) -> String {
    game["id"].as_str().expect("id").to_string()
}

#[tokio::test]
async fn registers_the_minigames_public_router_under_the_minigames_scope() {
    assert_eq!(
        scopes_of_module("features/minigames/api/public"),
        vec!["minigames"]
    );
}

#[tokio::test]
async fn get_v1_minigames_lists_minigames_in_the_authenticated_workspace_pinned_to_created_at_desc()
{
    let f = fixture().await;
    let first = id_of(&new_game(&f, WS, "Prize one").await);
    let second = id_of(&new_game(&f, WS, "Prize two").await);
    new_game(&f, WS, "Other").await;
    new_game(&f, OTHER_WS, "Prize foreign").await;
    let (status, listed) = call(
        &f,
        "minigames.list",
        WS,
        &[],
        Some("page=1&perPage=50&name=Prize"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["pageCount"], json!(1));
    let ids: Vec<&str> = listed["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, vec![second.as_str(), first.as_str()]);
    assert_eq!(listed["data"][0]["playUrl"], json!(play_url(&second)));
}

#[tokio::test]
async fn get_v1_minigames_id_gets_a_minigame_scoped_to_the_authenticated_workspace() {
    let f = fixture().await;
    let game = new_game(&f, WS, "Wheel").await;
    let id = id_of(&game);
    let (status, got) = call(&f, "minigames.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{got}");
    assert_eq!(got, game);
    assert_eq!(got["playUrl"], json!(play_url(&id)));
    let (status, _) = call(&f, "minigames.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn get_v1_minigames_id_propagates_the_service_declared_not_found_error() {
    let f = fixture().await;
    let (status, error) = call(&f, "minigames.get", WS, &[("id", "999")], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(error["code"], json!("notFound"));
    assert_eq!(error["message"], json!("Minigame not found"));
    assert_eq!(error["defined"], json!(true));
}

#[tokio::test]
async fn post_v1_minigames_creates_a_minigame_in_the_authenticated_workspace() {
    let f = fixture().await;
    let game = new_game(&f, WS, "Wheel").await;
    let id: i64 = id_of(&game).parse().expect("numeric id");
    assert_eq!(game["name"], json!("Wheel"));
    assert_eq!(game["type"], json!("luckyWheel"));
    assert_eq!(game["enabled"], json!(true));
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "Minigame" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn put_v1_minigames_id_updates_a_minigame_honoring_submitted_prize_quantities_verbatim() {
    let f = fixture().await;
    let id = id_of(&new_game(&f, WS, "Wheel").await);
    let id_num: i64 = id.parse().expect("numeric id");
    // A play took one prize since the create (live quantity 4).
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Minigame" SET "prizeSettings" = json_set("prizeSettings", '$.prizes[0].quantity', 4) WHERE "id" = ?1"#,
                params![id_num],
            )?;
            Ok(())
        })
        .await
        .expect("decrement");
    let mut put = body("Wheel renamed");
    put["prizeSettings"]["prizes"][0]["quantity"] = json!(5);
    let (status, updated) = call(&f, "minigames.update", WS, &[("id", &id)], None, Some(put)).await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["name"], json!("Wheel renamed"));
    assert_eq!(updated["playUrl"], json!(play_url(&id)));
    // The public API sends no baseline: the submitted 5 wins over the live 4.
    assert_eq!(updated["prizeSettings"]["prizes"][0]["quantity"], json!(5));
}

#[tokio::test]
async fn patch_v1_minigames_id_partially_updates_a_minigame_forwarding_only_the_submitted_fields() {
    let f = fixture().await;
    let game = new_game(&f, WS, "Wheel").await;
    let id = id_of(&game);
    let mut general = game["generalSettings"].clone();
    general["name"] = json!("Renamed");
    let (status, updated) = call(
        &f,
        "minigames.updatePartial",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "generalSettings": general })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["name"], json!("Renamed"));
    assert_eq!(updated["generalSettings"]["name"], json!("Renamed"));
    assert_eq!(updated["prizeSettings"], game["prizeSettings"]);
    assert_eq!(updated["playUrl"], json!(play_url(&id)));
}

#[tokio::test]
async fn delete_v1_minigames_id_deletes_the_selected_minigame_in_the_authenticated_workspace() {
    let f = fixture().await;
    let id = id_of(&new_game(&f, WS, "Wheel").await);
    let (status, reply) = call(&f, "minigames.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    assert_eq!(reply, Value::Null);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "id" FROM "Minigame" WHERE "id" = ?1"#,
            id.parse().expect("numeric id")
        )
        .await,
        None
    );
}

#[tokio::test]
async fn delete_v1_minigames_id_rejects_instead_of_silently_succeeding_when_the_minigame_does_not_exist()
 {
    let f = fixture().await;
    let id = id_of(&new_game(&f, OTHER_WS, "Wheel").await);
    let (status, error) = call(&f, "minigames.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(error["message"], json!("Minigame not found"));
}

#[tokio::test]
async fn post_v1_minigames_bulk_delete_deletes_multiple_minigames_in_the_authenticated_workspace() {
    let f = fixture().await;
    let a = id_of(&new_game(&f, WS, "A").await);
    let b = id_of(&new_game(&f, WS, "B").await);
    let keep = id_of(&new_game(&f, WS, "Keep").await);
    let foreign = id_of(&new_game(&f, OTHER_WS, "Foreign").await);
    let (status, _) = call(
        &f,
        "minigames.deleteMany",
        WS,
        &[],
        None,
        Some(json!({ "ids": [a, b, foreign] })),
    )
    .await;
    assert_eq!(status, 204);
    let count = |id: &str| {
        let id: i64 = id.parse().expect("numeric id");
        let f = &f;
        async move { scalar(f, r#"SELECT COUNT(*) FROM "Minigame" WHERE "id" = ?1"#, id).await }
    };
    assert_eq!(count(&a).await, Some(json!(0)));
    assert_eq!(count(&b).await, Some(json!(0)));
    assert_eq!(count(&keep).await, Some(json!(1)));
    assert_eq!(count(&foreign).await, Some(json!(1)));
}

#[tokio::test]
async fn patch_v1_minigames_id_enabled_sets_the_minigame_enabled_state_in_the_authenticated_workspace()
 {
    let f = fixture().await;
    let id = id_of(&new_game(&f, WS, "Wheel").await);
    let (status, updated) = call(
        &f,
        "minigames.setEnabled",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["enabled"], json!(false));
    assert_eq!(updated["playUrl"], json!(play_url(&id)));
}

async fn seed_players(f: &Fixture, game: i64) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName") VALUES (3001, ?1, 'Ada', 'Lovelace')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (3002, ?1, 'Bob')"#,
                params![WS],
            )?;
            for (id, contact, updated) in [(6001_i64, 3001_i64, 2_000_i64), (6002, 3002, 3_000)] {
                tx.execute(
                    r#"INSERT INTO "MinigameContact"
                         ("id", "createdAt", "updatedAt", "minigameId", "contactId", "openedAt", "played", "remaining", "sharesCount")
                       VALUES (?1, 1000, ?2, ?3, ?4, 1000, 1, 0, 0)"#,
                    params![id, updated, game, contact],
                )?;
            }
            for (id, contact, at, winning) in [
                (6101_i64, 3001_i64, 1_500_i64, true),
                (6102, 3001, 1_800, false),
                (6103, 3002, 2_500, false),
            ] {
                tx.execute(
                    r#"INSERT INTO "MinigamePlay" ("id", "createdAt", "updatedAt", "minigameId", "contactId", "isWinning", "prizeName")
                       VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)"#,
                    params![id, at, game, contact, winning, winning.then_some("Free coffee")],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed players");
}

#[tokio::test]
async fn get_v1_minigames_id_plays_lists_play_records_for_the_requested_contact() {
    let f = fixture().await;
    let id = id_of(&new_game(&f, WS, "Wheel").await);
    seed_players(&f, id.parse().expect("numeric id")).await;
    let (status, plays) = call(
        &f,
        "minigames.listPlays",
        WS,
        &[("id", &id)],
        Some("contactId=3001"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{plays}");
    let ids: Vec<&str> = plays["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, vec!["6102", "6101"]);
    assert_eq!(plays["data"][1]["prizeName"], json!("Free coffee"));
    assert_eq!(plays["data"][1]["isWinning"], json!(true));
}

#[tokio::test]
async fn get_v1_minigames_id_players_lists_players_for_the_requested_minigame() {
    let f = fixture().await;
    let id = id_of(&new_game(&f, WS, "Wheel").await);
    seed_players(&f, id.parse().expect("numeric id")).await;
    let (status, players) = call(&f, "minigames.listPlayers", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{players}");
    assert_eq!(players["pageCount"], json!(1));
    let ids: Vec<&str> = players["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().unwrap_or_default())
        .collect();
    // Newest played first (updatedAt desc).
    assert_eq!(ids, vec!["6002", "6001"]);
    assert_eq!(
        players["data"][1]["contact"]["fullName"],
        json!("Ada Lovelace")
    );
}

#[tokio::test]
async fn get_v1_minigames_id_players_forwards_the_optional_name_filter() {
    let f = fixture().await;
    let id = id_of(&new_game(&f, WS, "Wheel").await);
    seed_players(&f, id.parse().expect("numeric id")).await;
    let (status, players) = call(
        &f,
        "minigames.listPlayers",
        WS,
        &[("id", &id)],
        Some("name=ada"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{players}");
    let ids: Vec<&str> = players["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, vec!["6001"]);
}
