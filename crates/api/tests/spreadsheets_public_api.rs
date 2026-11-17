//! `apps/builder/__tests__/spreadsheets-public-api.test.ts` (Q.D13): the
//! routes run on a migrated database. The worksheet reads wait for the
//! Google Sheets client (Q.I4).

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

const URL: &str = "https://docs.google.com/spreadsheets/d/sheet-1/edit";

#[tokio::test]
async fn spreadsheet_create_needs_the_integration_then_crud_runs() {
    let fx = fixture().await;
    let body = json!({ "name": "Sales", "url": URL });
    // No Google Sheets integration: a `validation` error on `url`.
    let (status, out) = call(
        &fx,
        "spreadsheets.create",
        WS,
        &[],
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (422, json!("You need to setup google sheets first.")),
        "{out}"
    );
    // A non-Google URL fails the request schema.
    let (status, _) = call(
        &fx,
        "spreadsheets.create",
        WS,
        &[],
        None,
        Some(
            json!({ "name": "Sales", "url": "https://evil.com/docs.google.com/spreadsheets/d/x/" }),
        ),
    )
    .await;
    assert_eq!(status, 422);

    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (77, ?1, 'googleSheets')"#,
                chatbotx_db::rusqlite::params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationGoogleSheet" ("workspaceId", "integrationId", "auth") VALUES (?1, 77, '{}')"#,
                chatbotx_db::rusqlite::params![WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let (status, out) = call(&fx, "spreadsheets.create", WS, &[], None, Some(body)).await;
    assert_eq!(status, 201, "{out}");
    let id = out["id"].as_str().unwrap().to_owned();

    let (status, got) = call(&fx, "spreadsheets.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{got}");
    assert_eq!(
        (got["spreadsheetId"].clone(), got.get("workspaceId")),
        (json!("sheet-1"), None)
    );
    let (status, _) = call(
        &fx,
        "spreadsheets.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);

    let (status, out) = call(
        &fx,
        "spreadsheets.update",
        WS,
        &[("id", &id)],
        None,
        Some(
            json!({ "name": "Renamed", "url": "https://docs.google.com/spreadsheets/d/sheet-2/" }),
        ),
    )
    .await;
    assert_eq!(
        (status, out["name"].clone(), out["spreadsheetId"].clone()),
        (200, json!("Renamed"), json!("sheet-2")),
        "{out}"
    );

    let (_, list) = call(&fx, "spreadsheets.list", WS, &[], Some("name=REN"), None).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
    let (_, list) = call(&fx, "spreadsheets.list", WS, &[], Some("name=zzz"), None).await;
    assert_eq!(list["data"], json!([]));

    let (status, _) = call(&fx, "spreadsheets.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&fx, "spreadsheets.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}
