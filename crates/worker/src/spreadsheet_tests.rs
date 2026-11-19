//! Ports of the main cases of `apps/worker/__tests__/spreadsheet-handler-*.test.ts`,
//! `spreadsheet-lookup-values.test.ts` and `spreadsheet-write-values.test.ts`
//! (Q.D13a): the Sheets steps on a migrated database over a fake Sheets
//! transport that records the requests. Adapted: the mocked
//! `googleSheetsIntegration` becomes the recorded HTTP calls.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use chatbotx_business::email_marketing::{Http, Reply, Request};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use serde_json::{Value, json};

use super::{SheetRun, execute};
use crate::steps::test_support::Fixture;

/// Header row + two rows; records `(method, url, body)`.
#[derive(Default)]
struct FakeSheets(Mutex<Vec<(String, String, Value)>>);

impl FakeSheets {
    fn writes(&self) -> Vec<(String, String, Value)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _, _)| m != "GET")
            .cloned()
            .collect()
    }
}

impl Http for FakeSheets {
    fn send(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = Result<Reply, String>> + Send + '_>> {
        let body = request.body.clone().unwrap_or(Value::Null);
        self.0
            .lock()
            .unwrap()
            .push((request.method.to_owned(), request.url.clone(), body));
        let reply = if request.method == "GET" && request.url.contains("1%3A1") {
            json!({ "values": [["Name", "Email"]] })
        } else if request.method == "GET" {
            json!({ "values": [["Name", "Email"], ["Ada", "ada@example.com"], ["Bob", "bob@example.com"]] })
        } else {
            json!({})
        };
        Box::pin(async move {
            Ok(Reply {
                status: 200,
                headers: Vec::new(),
                body: reply.to_string(),
            })
        })
    }
}

async fn fixture() -> Fixture {
    let fx = Fixture::new().await;
    fx.sql(|tx| {
        tx.execute_batch(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (20, 2, 'googleSheets');
               INSERT INTO "IntegrationGoogleSheet" ("id", "workspaceId", "integrationId", "auth")
                 VALUES (21, 2, 20, '{"tokens":{"accessToken":"t"}}');
               INSERT INTO "Spreadsheet" ("id", "workspaceId", "name", "url", "spreadsheetId")
                 VALUES (22, 2, 'Leads', 'https://docs.google.com/spreadsheets/d/g-1/edit', 'g-1');
               INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (23, 'Email', 'email', 2);
               INSERT INTO "ContactCustomField" ("contactId", "customFieldId", "value") VALUES (3, 23, 'bob@example.com');"#,
        )?;
        Ok(())
    })
    .await;
    fx
}

async fn rows(fx: &Fixture) -> (Conversation, ContactInbox) {
    let (conversation, contact_inbox) = fx
        .deps
        .db
        .read(|c| {
            Ok((
                chatbotx_db::repo::find_one::<Conversation>(c, r#""id" = 6"#, &[]),
                chatbotx_db::repo::find_one::<ContactInbox>(c, r#""id" = 5"#, &[]),
            ))
        })
        .await
        .expect("read");
    (
        conversation.expect("query").expect("conversation"),
        contact_inbox.expect("query").expect("contact inbox"),
    )
}

async fn run(
    fx: &Fixture,
    http: &FakeSheets,
    step_type: &str,
    step: Value,
) -> Result<(), &'static str> {
    let (conversation, contact_inbox) = rows(fx).await;
    let r = SheetRun {
        deps: &fx.deps,
        http,
        conversation: &conversation,
        contact_inbox: &contact_inbox,
        sent_by_user_id: None,
    };
    execute(&r, step_type, &step).await.map_err(|f| f.message)
}

fn by_email(value: &str) -> Value {
    json!({ "mode": "AND", "conditions": [{ "column": "Email", "operator": "is", "value": value }] })
}

#[tokio::test]
async fn get_row_stores_the_mapped_cells_in_the_contact_fields() {
    let fx = fixture().await;
    let http = FakeSheets::default();
    let step = json!({
        "spreadsheetId": "22", "sheetName": "Leads",
        "lookup": by_email("ada@example.com"),
        "map": [{ "header": "Email", "customFieldId": "23" }],
    });
    run(&fx, &http, "spreadsheetGetRow", step)
        .await
        .expect("run");
    assert_eq!(
        fx.text(r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = 23"#)
            .await
            .as_deref(),
        Some("ada@example.com")
    );
    // A lookup without a match is the step's error.
    let step = json!({ "spreadsheetId": "22", "sheetName": "Leads", "lookup": by_email("nobody"), "map": [] });
    assert_eq!(
        run(&fx, &http, "spreadsheetGetRow", step).await,
        Err("Row not found")
    );
}

#[tokio::test]
async fn send_data_appends_a_header_aligned_row_from_the_contact_fields() {
    let fx = fixture().await;
    let http = FakeSheets::default();
    let step = json!({
        "spreadsheetId": "22", "sheetName": "Leads",
        "map": [{ "header": "Email", "customFieldId": "23" }],
    });
    run(&fx, &http, "spreadsheetSendData", step)
        .await
        .expect("run");
    let writes = http.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].0, "POST");
    assert!(
        writes[0].1.contains("/spreadsheets/g-1/values/"),
        "{}",
        writes[0].1
    );
    assert_eq!(writes[0].2, json!({ "values": [["", "bob@example.com"]] }));
}

#[tokio::test]
async fn update_and_clear_touch_only_the_matching_rows() {
    let fx = fixture().await;
    let http = FakeSheets::default();
    let step = json!({
        "spreadsheetId": "22", "sheetName": "Leads", "version": "v2",
        "lookup": by_email("bob@example.com"),
        "map": [{ "header": "Name", "value": "Robert" }],
    });
    run(&fx, &http, "spreadsheetUpdateRow", step)
        .await
        .expect("update");
    let writes = http.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].0, "PUT");
    // Row index 2 of the values (the header is row 1): `A3`.
    assert!(writes[0].1.contains("Leads%21A3"), "{}", writes[0].1);
    assert_eq!(
        writes[0].2,
        json!({ "values": [["Robert", "bob@example.com"]] })
    );

    let http = FakeSheets::default();
    let step = json!({ "spreadsheetId": "22", "sheetName": "Leads", "lookup": by_email("ada@example.com") });
    run(&fx, &http, "spreadsheetClearRow", step)
        .await
        .expect("clear");
    let writes = http.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].1.ends_with(":clear"), "{}", writes[0].1);

    let step = json!({ "spreadsheetId": "22", "sheetName": "Leads", "lookup": by_email("nobody") });
    assert_eq!(
        run(&fx, &http, "spreadsheetClearRow", step).await,
        Err("No rows found")
    );
}

#[tokio::test]
async fn a_spreadsheet_outside_the_workspace_fails_the_step() {
    let fx = fixture().await;
    let http = FakeSheets::default();
    let step =
        json!({ "spreadsheetId": "999", "sheetName": "Leads", "lookup": by_email("x"), "map": [] });
    assert_eq!(
        run(&fx, &http, "spreadsheetGetRow", step).await,
        Err("Failed to get spreadsheet row")
    );
    assert!(http.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bot_field_mappings_use_the_bot_field_write_and_read_paths() {
    let fx = fixture().await;
    fx.exec(
        r#"INSERT INTO "BotField" ("id", "name", "type", "value", "workspaceId") VALUES (24, 'Last lead', 'shortText', 'old', 2)"#,
    )
    .await;
    let http = FakeSheets::default();
    // Sheet -> bot field (the custom field mapping keeps its own path).
    let step = json!({
        "spreadsheetId": "22", "sheetName": "Leads",
        "lookup": by_email("ada@example.com"),
        "map": [
            { "header": "Name", "customFieldId": "bot_field:24" },
            { "header": "Email", "customFieldId": "23" },
        ],
    });
    run(&fx, &http, "spreadsheetGetRow", step)
        .await
        .expect("get");
    assert_eq!(
        fx.text(r#"SELECT "value" FROM "BotField" WHERE "id" = 24"#)
            .await
            .as_deref(),
        Some("Ada")
    );
    assert_eq!(
        fx.text(r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = 23"#)
            .await
            .as_deref(),
        Some("ada@example.com")
    );
    // Contact -> sheet writes the bot field's value, not a blank.
    let step = json!({
        "spreadsheetId": "22", "sheetName": "Leads",
        "map": [{ "header": "Name", "customFieldId": "bot_field:24" }],
    });
    run(&fx, &http, "spreadsheetSendData", step)
        .await
        .expect("send");
    assert_eq!(http.writes()[0].2, json!({ "values": [["Ada", ""]] }));
}
