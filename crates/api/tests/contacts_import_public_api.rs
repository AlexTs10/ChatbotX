//! Port of `apps/builder/__tests__/contacts-import-public-api.test.ts`
//! (P3.6-D2 slice contact imports, part two). Adapted: the captured
//! handlers run through `handlers::dispatch` over a real database; the
//! mocked `importService.list` / `find` become seeded `Import` rows. Plus two
//! own cases for `contacts.import` (parity `10-import`: start + run, input
//! errors).

mod handlers_common;

use std::sync::Arc;

use chatbotx_api::routes::match_route;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{PutOptions, Uploader};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            for (file, ws) in [(1_i64, WS), (2, OTHER_WS), (3, WS)] {
                tx.execute(
                    r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType")
                       VALUES (?1, ?2, 'import', 'contacts', ?3, 'people.csv', 'text/csv')"#,
                    params![file, ws, format!("p/{file}.csv")],
                )?;
            }
            for (id, ws, file, kind, status) in [
                (11_i64, WS, 1_i64, "contacts", "completed"),
                (12, OTHER_WS, 2, "contacts", "completed"),
                (13, WS, 3, "products", "completed"),
            ] {
                tx.execute(
                    r#"INSERT INTO "Import" ("id", "workspaceId", "fileId", "type", "format", "status", "meta") VALUES (?1, ?2, ?3, ?4, 'csv', ?5, '{}')"#,
                    params![id, ws, file, kind, status],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
}

#[tokio::test]
async fn registers_the_contact_import_public_router_under_the_contacts_scope() {
    for (method, path) in [
        ("GET", "/v1/contacts/imports"),
        ("GET", "/v1/contacts/imports/1"),
        ("POST", "/v1/contacts/import"),
    ] {
        let matched = match_route(method, path).expect("route");
        assert_eq!(
            format!("{:?}", matched.route.auth),
            format!(
                "{:?}",
                chatbotx_api::routes::RouteAuth::WorkspaceToken(&["contacts"])
            ),
            "{path}"
        );
    }
}

#[tokio::test]
async fn get_v1_contacts_imports_lists_only_contact_import_jobs_in_the_token_workspace() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.listImports",
        WS,
        &[],
        Some("page=1&perPage=25&status=completed&keyword=people"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec!["11"]);
    assert_eq!(body["pageCount"], json!(1));
    assert_eq!(body["data"][0]["type"], json!("contacts"));
}

#[tokio::test]
async fn get_v1_contacts_imports_id_returns_the_workspace_scoped_contact_import_job() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "contacts.getImport", WS, &[("id", "11")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("11"));
    assert_eq!(body["type"], json!("contacts"));
}

#[tokio::test]
async fn get_v1_contacts_imports_id_returns_not_found_when_the_import_is_absent_or_outside_contacts_scope()
 {
    let f = fixture().await;
    seed(&f).await;
    for id in ["999", "12", "13"] {
        let (status, body) = call(&f, "contacts.getImport", WS, &[("id", id)], None, None).await;
        assert_eq!(status, 404, "{id}: {body}");
        assert_eq!(body["message"], json!("Import not found"));
    }
}

/// A fixture with a file store, a messenger inbox (701) and an uploaded
/// contacts CSV (file 21).
async fn uploaded(csv: &str) -> Fixture {
    let mut f = fixture().await;
    let files = Arc::new(Uploader::in_memory());
    files
        .put_object(
            "p/21.csv",
            csv.as_bytes().to_vec(),
            PutOptions::default(),
            None,
        )
        .await
        .expect("put");
    f.state = f.state.clone().with_files(files);
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (701, ?1, 'M', 'messenger', 'src-701')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType")
                   VALUES (21, ?1, 'import', 'contacts', 'p/21.csv', 'contacts.csv', 'text/csv')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

#[tokio::test]
async fn post_v1_contacts_import_starts_and_runs_the_import() {
    let f = uploaded("uid,mail\nimp-1,ONE@Parity.Test\n,\nimp-1,again@parity.test\n").await;
    let (status, body) = call(
        &f,
        "contacts.import",
        WS,
        &[],
        None,
        Some(json!({ "fileId": "21", "channel": "messenger", "inboxId": "701", "contactId": "uid", "email": "mail" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id = body["importId"].as_str().expect("importId").to_string();
    let (status, job) = call(&f, "contacts.getImport", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{job}");
    assert_eq!(job["status"], json!("completed"));
    assert_eq!(
        (
            &job["totalCount"],
            &job["successCount"],
            &job["failedCount"]
        ),
        (&json!(3), &json!(1), &json!(2))
    );
    assert_eq!(
        job["errorSample"],
        json!([{ "row": 3, "reason": "Invalid row" }])
    );
    assert_eq!(
        job["errorMessage"],
        json!("2 rows failed; showing the first 1")
    );
}

fn issue_paths(body: &Value) -> Vec<String> {
    body["data"]["issues"]
        .as_array()
        .map(|issues| issues.iter().map(|i| i["path"].to_string()).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn post_v1_contacts_import_input_and_start_errors() {
    let f = uploaded("uid\nx1\n").await;
    let base =
        json!({ "fileId": "21", "channel": "messenger", "inboxId": "701", "contactId": "uid" });
    let with = |patch: Value| {
        let mut body = base.clone();
        for (k, v) in patch.as_object().expect("patch") {
            if v.is_null() {
                body.as_object_mut().expect("body").remove(k);
            } else {
                body[k] = v.clone();
            }
        }
        body
    };
    for (patch, path) in [
        (json!({ "contactId": null }), "[\"contactId\"]"),
        (
            json!({ "channel": "whatsapp", "contactId": null }),
            "[\"phoneNumber\"]",
        ),
        (json!({ "countryCode": "84" }), "[\"countryCode\"]"),
        (json!({ "channel": "fax" }), "[\"channel\"]"),
        (json!({ "fileId": "abc" }), "[\"fileId\"]"),
    ] {
        let (status, body) = call(&f, "contacts.import", WS, &[], None, Some(with(patch))).await;
        assert_eq!(status, 422, "{body}");
        assert_eq!(issue_paths(&body), vec![path.to_string()], "{body}");
    }
    let many: Vec<Value> = (0..11)
        .map(|i| json!({ "column": format!("c{i}"), "customFieldId": "1" }))
        .collect();
    let (status, body) = call(
        &f,
        "contacts.import",
        WS,
        &[],
        None,
        Some(with(json!({ "fieldMapping": many }))),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(issue_paths(&body), vec!["[\"fieldMapping\"]".to_string()]);
    let (status, body) = call(
        &f,
        "contacts.import",
        WS,
        &[],
        None,
        Some(with(json!({ "fileId": "1" }))),
    )
    .await;
    assert_eq!(
        (status, &body["code"]),
        (404, &json!("contactImportFileNotFound")),
        "{body}"
    );
    let (status, body) = call(
        &f,
        "contacts.import",
        WS,
        &[],
        None,
        Some(with(json!({ "inboxId": "1" }))),
    )
    .await;
    assert_eq!(
        (status, &body["code"]),
        (404, &json!("contactImportInboxNotFound")),
        "{body}"
    );
}
