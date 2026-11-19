//! `apps/builder/__tests__/spreadsheets-public-api.test.ts` (Q.D13): the
//! routes run on a migrated database; the Google Sheets API is a fake
//! transport (Q.D13a: the `listSheetNames` check, `listWorksheets`,
//! `listWorksheetHeaders`).

mod handlers_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chatbotx_business::email_marketing::{Http, Reply, Request};
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

/// Google Sheets: `bad-sheet` is unreadable, a values read is the header
/// row, any other read one tab.
struct FakeSheets;

impl Http for FakeSheets {
    fn send(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = Result<Reply, String>> + Send + '_>> {
        let (status, body) = if request.url.contains("bad-sheet") {
            (
                404,
                json!({ "error": { "message": "Requested entity was not found." } }),
            )
        } else if request.url.contains("/values/") {
            (200, json!({ "values": [["Name", "Email"]] }))
        } else {
            (
                200,
                json!({ "sheets": [{ "properties": { "title": "Leads" } }] }),
            )
        };
        Box::pin(async move {
            Ok(Reply {
                status,
                headers: Vec::new(),
                body: body.to_string(),
            })
        })
    }
}

const URL: &str = "https://docs.google.com/spreadsheets/d/sheet-1/edit";

#[tokio::test]
async fn spreadsheet_create_needs_the_integration_then_crud_runs() {
    let mut fx = fixture().await;
    fx.state = fx.state.clone().with_provider_http(Arc::new(FakeSheets));
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
                r#"INSERT INTO "IntegrationGoogleSheet" ("workspaceId", "integrationId", "auth") VALUES (?1, 77, '{"tokens":{"accessToken":"t"}}')"#,
                chatbotx_db::rusqlite::params![WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    // Google cannot read the sheet: the URL error.
    let (status, out) = call(
        &fx,
        "spreadsheets.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Sales", "url": "https://docs.google.com/spreadsheets/d/bad-sheet/edit" })),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (
            422,
            json!("URL must be a valid, public or shareable Google Sheets link.")
        ),
        "{out}"
    );

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

    let (status, tabs) = call(
        &fx,
        "spreadsheets.listWorksheets",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!((status, tabs), (200, json!({ "data": ["Leads"] })));
    let (status, headers) = call(
        &fx,
        "spreadsheets.listWorksheetHeaders",
        WS,
        &[("id", &id), ("worksheetName", "Leads")],
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, headers),
        (200, json!({ "data": ["Name", "Email"] }))
    );
    let (status, _) = call(
        &fx,
        "spreadsheets.listWorksheets",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);

    let (_, list) = call(&fx, "spreadsheets.list", WS, &[], Some("name=REN"), None).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
    let (_, list) = call(&fx, "spreadsheets.list", WS, &[], Some("name=zzz"), None).await;
    assert_eq!(list["data"], json!([]));

    let (status, _) = call(&fx, "spreadsheets.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&fx, "spreadsheets.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}
