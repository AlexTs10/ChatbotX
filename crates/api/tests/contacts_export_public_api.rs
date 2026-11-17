//! Ports (P3.6-D2 slice contact export) of
//! `apps/builder/__tests__/contacts-export-public-api.test.ts`,
//! `contacts-export-files-public-api.test.ts`, `export-contacts-action.test.ts`
//! (the `exportContactsRequest` schema) and the `listContactExportFiles` cases
//! of `contact-export-files.test.ts`. Adapted: the captured handlers run
//! through `handlers::dispatch` over a real database and an in-memory store;
//! the mocked service calls become the rows and the CSV they write.

mod handlers_common;

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chatbotx_api::routes::match_route;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{LocalUrlSigner, Uploader};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn with_files() -> (Fixture, Arc<Uploader>) {
    let mut f = fixture().await;
    let files = Arc::new(
        Uploader::in_memory()
            .with_url_signer(LocalUrlSigner::new("secret", "http://app/files").expect("signer")),
    );
    f.state = f.state.clone().with_files(Arc::clone(&files));
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "email", "workspaceId") VALUES (1, 'Jane', 'jane@example.com', ?1), (2, 'Ken', NULL, ?1), (3, 'Other', NULL, ?2)"#,
                params![WS, OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    (f, files)
}

#[tokio::test]
async fn registers_the_contact_export_routes_under_the_contacts_scope() {
    for (method, path, id) in [
        ("POST", "/v1/contacts/export", "contacts.export"),
        (
            "GET",
            "/v1/contacts/export-files/1",
            "contacts.getExportFile",
        ),
        (
            "GET",
            "/v1/contacts/export-files",
            "contacts.listExportFiles",
        ),
    ] {
        let matched = match_route(method, path).expect("route");
        assert_eq!(matched.route.id, id);
        assert_eq!(
            format!("{:?}", matched.route.auth),
            format!(
                "{:?}",
                chatbotx_api::routes::RouteAuth::WorkspaceToken(&["contacts"])
            ),
        );
    }
}

#[tokio::test]
async fn post_v1_contacts_export_starts_the_export_with_no_requester_and_full_pii_visibility() {
    let (f, files) = with_files().await;
    let (status, body) = call(
        &f,
        "contacts.export",
        WS,
        &[],
        None,
        Some(json!({ "fields": ["sys:firstName", "sys:email"], "exportAll": true })),
    )
    .await;
    // Route metadata declares a 202 (async job).
    assert_eq!(status, 202, "{body}");
    let file_id = body["fileId"].as_str().expect("fileId").to_string();
    let user: Option<i64> = f
        .state
        .db
        .read({
            let id: i64 = file_id.parse().expect("id");
            move |c| {
                Ok(c.query_row(
                    r#"SELECT "userId" FROM "File" WHERE "id" = ?1"#,
                    [id],
                    |r| r.get(0),
                )?)
            }
        })
        .await
        .expect("file");
    assert_eq!(user, None);
    let (status, file) = call(
        &f,
        "contacts.getExportFile",
        WS,
        &[("fileId", &file_id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{file}");
    assert_eq!(file["status"], json!("uploaded"));
    assert_eq!(file["totalRecords"], json!(2));
    assert!(
        file["fileName"]
            .as_str()
            .is_some_and(|n| n.starts_with("contacts-"))
    );
    assert!(
        file["downloadUrl"]
            .as_str()
            .is_some_and(|u| u.contains(&format!(
                "workspaces/{WS}/exports/contacts/contact_{file_id}.csv"
            )))
    );
    let path = format!("workspaces/{WS}/exports/contacts/contact_{file_id}.csv");
    let bytes = files.get_object(&path).await.expect("csv");
    // A token sees full PII: the email column is kept.
    assert_eq!(
        String::from_utf8(bytes.to_vec()).expect("utf8"),
        "\u{feff}\"First Name\",\"Email\"\n\"Jane\",\"jane@example.com\"\n\"Ken\",\"\"\n"
    );
}

#[tokio::test]
async fn get_v1_contacts_export_files_scopes_the_lookup_by_workspace_id() {
    let (f, _) = with_files().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType", "status", "meta")
                   VALUES (41, ?1, 'export', 'contacts', 'p/41', 'contacts-2026-01-01.csv', 'text/csv', 'pending', NULL),
                          (42, ?2, 'export', 'contacts', 'p/42', 'theirs.csv', 'text/csv', 'uploaded', NULL)"#,
                params![WS, OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let (status, body) = call(
        &f,
        "contacts.getExportFile",
        WS,
        &[("fileId", "41")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "status": "pending", "fileName": "contacts-2026-01-01.csv", "downloadUrl": null, "totalRecords": null })
    );
    let (status, body) = call(
        &f,
        "contacts.getExportFile",
        WS,
        &[("fileId", "42")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Export file not found"));
}

async fn export_status(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "contacts.export", WS, &[], None, Some(body)).await
}

#[tokio::test]
async fn export_contacts_request_schema_accepts_the_valid_shapes() {
    let (f, _) = with_files().await;
    let fields = json!(["sys:fullName", "sys:email"]);
    for body in [
        json!({ "fields": fields, "exportAll": true }),
        json!({ "fields": fields, "exportAll": true, "filter": { "keyword": "Acme", "contactFilter": { "operator": "and", "conditions": [] } } }),
        json!({ "fields": fields, "contactIds": ["1", "2"] }),
        json!({ "fields": ["cus:123", "tag:9", "sys:sourceUserId"], "exportAll": true }),
        json!({ "fields": fields, "contactIds": ["1234567890"] }),
    ] {
        let (status, out) = export_status(&f, body.clone()).await;
        assert_eq!(status, 202, "{body}: {out}");
    }
}

fn issue_paths(body: &Value) -> Vec<String> {
    body["data"]["issues"]
        .as_array()
        .map(|issues| issues.iter().map(|i| i["path"].to_string()).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn export_contacts_request_schema_rejects_the_invalid_shapes() {
    let (f, _) = with_files().await;
    let fields = json!(["sys:fullName", "sys:email"]);
    for (body, path) in [
        (
            json!({ "fields": fields, "contactIds": [] }),
            "[\"contactIds\"]",
        ),
        (json!({ "fields": fields }), "[\"contactIds\"]"),
        (
            json!({ "fields": fields, "exportAll": false }),
            "[\"contactIds\"]",
        ),
        (
            json!({ "fields": ["sys:email", "123"], "exportAll": true }),
            "[\"fields\",1]",
        ),
        (json!({ "fields": [], "exportAll": true }), "[\"fields\"]"),
        (
            json!({ "fields": fields, "contactIds": ["abc"] }),
            "[\"contactIds\",0]",
        ),
    ] {
        let (status, out) = export_status(&f, body.clone()).await;
        assert_eq!(status, 422, "{body}: {out}");
        assert_eq!(issue_paths(&out), vec![path.to_string()], "{body}: {out}");
    }
    let (_, out) = export_status(&f, json!({ "fields": ["123"], "exportAll": true })).await;
    assert_eq!(
        out["data"]["issues"][0]["message"],
        json!("Use sys:<column>, cus:<customFieldId> or tag:<tagId>")
    );
}

fn cursor(value: Value) -> String {
    STANDARD.encode(value.to_string())
}

#[tokio::test]
async fn get_v1_contacts_export_files_pages_with_an_opaque_cursor() {
    let (f, _) = with_files().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(r#"UPDATE "User" SET "name" = 'Lan' WHERE "id" = (SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1)"#, [WS])?;
            for (id, ws, user) in [(51_i64, WS, true), (52, WS, false), (53, OTHER_WS, false)] {
                tx.execute(
                    r#"INSERT INTO "File" ("id", "createdAt", "workspaceId", "userId", "contextType", "subType", "path", "fileName", "mimeType", "status", "meta")
                       VALUES (?1, 1792490400123, ?2, CASE WHEN ?3 THEN (SELECT "ownerId" FROM "Workspace" WHERE "id" = ?2) END, 'export', 'contacts', ?4, 'c.csv', 'text/csv', 'uploaded', '{"totalRecords":3}')"#,
                    params![id, ws, user, format!("p/{id}")],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    let (status, body) = call(
        &f,
        "contacts.listExportFiles",
        WS,
        &[],
        Some("perPage=1"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"][0]["id"], json!("52"));
    assert_eq!(body["data"][0]["requestedBy"], Value::Null);
    assert_eq!(
        body["data"][0]["createdAt"],
        json!("2026-10-20T10:00:00.123Z")
    );
    assert_eq!(
        body["data"][0]["expiresAt"],
        json!("2026-10-27T10:00:00.123Z")
    );
    let next = body["nextCursor"].as_str().expect("cursor").to_string();
    let decoded: Value =
        serde_json::from_slice(&STANDARD.decode(&next).expect("base64")).expect("json");
    assert_eq!(
        decoded,
        json!({ "createdAt": "2026-10-20T10:00:00.123000Z", "id": "52" })
    );
    let (status, body) = call(
        &f,
        "contacts.listExportFiles",
        WS,
        &[],
        Some(&format!("perPage=1&cursor={}", urlencode(&next))),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"][0]["id"], json!("51"));
    assert_eq!(body["data"][0]["totalRecords"], json!(3));
    assert_eq!(body["data"][0]["requestedBy"]["name"], json!("Lan"));
    assert_eq!(body["nextCursor"], Value::Null);
}

fn urlencode(text: &str) -> String {
    text.replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D")
}

#[tokio::test]
async fn get_v1_contacts_export_files_rejects_a_corrupt_cursor_instead_of_restarting_at_page_1() {
    let (f, _) = with_files().await;
    let created_at = "2026-10-20T10:00:00.123456Z";
    for text in [
        "not-a-cursor".to_string(),
        cursor(json!({ "createdAt": "'; drop", "id": "42" })),
        cursor(json!({ "createdAt": "2026-02-30T10:00:00.000000Z", "id": "42" })),
        cursor(json!({ "createdAt": "0000-01-01T00:00:00.000000Z", "id": "42" })),
        cursor(json!({ "createdAt": created_at, "id": "9223372036854775808" })),
    ] {
        let (status, body) = call(
            &f,
            "contacts.listExportFiles",
            WS,
            &[],
            Some(&format!("cursor={}", urlencode(&text))),
            None,
        )
        .await;
        assert_eq!(
            (status, &body["code"]),
            (400, &json!("invalidCursor")),
            "{text}: {body}"
        );
    }
    let (status, body) = call(
        &f,
        "contacts.listExportFiles",
        WS,
        &[],
        Some(&format!(
            "cursor={}",
            urlencode(&cursor(json!({ "createdAt": created_at, "id": "42" })))
        )),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
}
