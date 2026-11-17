//! `apps/builder/__tests__/saved-replies-public-api.test.ts` and
//! `response-sla-public-api.test.ts` (Q.D13): the TS tests check that each
//! route delegates to its service with the token workspace; here the routes
//! run on a migrated database and the responses and rows are checked.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

async fn seed_files(fx: &handlers_common::Fixture) {
    fx.state
        .db
        .write(|tx| {
            for (id, name, mime) in [
                (31, "a.png", "image/png"),
                (32, "b.jpg", "image/jpeg"),
                (33, "doc.pdf", "application/pdf"),
            ] {
                tx.execute(
                    r#"INSERT INTO "MediaLibraryFile" ("id", "name", "path", "mimeType", "size", "workspaceId")
                       VALUES (?1, ?2, ?3, ?4, 10, ?5)"#,
                    chatbotx_db::rusqlite::params![id, name, format!("ws/{name}"), mime, WS],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn saved_reply_create_get_update_list_and_delete() {
    let fx = fixture().await;
    seed_files(&fx).await;
    // `delegates to savedReplyService.create` + `passes the ordered image ids`.
    let (status, out) = call(
        &fx,
        "savedReplies.create",
        WS,
        &[],
        None,
        Some(json!({ "shortcut": " hi ", "text": " Hello ", "imageFileIds": ["32", "31"] })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(out["shortcut"], "hi");
    assert_eq!(out["text"], "Hello");
    assert_eq!(out["workspaceId"], WS.to_string());
    let images: Vec<&Value> = out["images"].as_array().unwrap().iter().collect();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0]["mediaLibraryFileId"], "32");
    assert_eq!(images[0]["name"], "b.jpg");
    assert!(images[0]["url"].as_str().unwrap().ends_with("ws/b.jpg"));
    let id = out["id"].as_str().unwrap().to_owned();

    // `delegates to savedReplyService.findByIdOrFail`; other workspaces 404.
    let (status, got) = call(&fx, "savedReplies.get", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, got["id"].clone()), (200, json!(id)));
    let (status, got) = call(&fx, "savedReplies.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!((status, got["message"].clone()), (404, json!("Saved reply not found")));

    // `accepts a rename that leaves text and images out`: both are kept.
    let (status, out) = call(
        &fx,
        "savedReplies.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "shortcut": "hey" })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!((out["shortcut"].clone(), out["text"].clone()), (json!("hey"), json!("Hello")));
    assert_eq!(out["images"].as_array().unwrap().len(), 2);

    // `[]` removes the images; empty text then has nothing left.
    let (status, out) = call(
        &fx,
        "savedReplies.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "shortcut": "hey", "text": "", "imageFileIds": [] })),
    )
    .await;
    assert_eq!((status, out["code"].clone()), (422, json!("validation")), "{out}");

    let (status, list) = call(&fx, "savedReplies.list", WS, &[], Some("perPage=1"), None).await;
    assert_eq!(status, 200);
    assert_eq!((list["data"].as_array().unwrap().len(), list["pageCount"].clone()), (1, json!(1)));

    // `delegates to savedReplyService.delete`; the image links cascade.
    let (status, _) = call(&fx, "savedReplies.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(&fx, r#"SELECT COUNT(*) FROM "SavedReplyImage" WHERE "savedReplyId" = ?1"#, id.parse().unwrap()).await,
        Some(json!(0))
    );
    let (status, _) = call(&fx, "savedReplies.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn saved_reply_request_checks() {
    let fx = fixture().await;
    seed_files(&fx).await;
    // `accepts images without text`.
    let (status, out) = call(
        &fx,
        "savedReplies.create",
        WS,
        &[],
        None,
        Some(json!({ "shortcut": "img", "imageFileIds": ["31"] })),
    )
    .await;
    assert_eq!((status, out["text"].clone()), (201, json!("")), "{out}");
    // `rejects a reply with neither text nor images` (refine on `text`).
    let (status, out) = call(&fx, "savedReplies.create", WS, &[], None, Some(json!({ "shortcut": "x" }))).await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(out["data"]["issues"][0]["path"], json!(["text"]));
    assert_eq!(out["data"]["issues"][0]["message"], "A saved reply needs text or at least one image");
    // `rejects more than 10 images`.
    let many: Vec<String> = (0..11).map(|i| i.to_string()).collect();
    let (status, out) = call(
        &fx,
        "savedReplies.create",
        WS,
        &[],
        None,
        Some(json!({ "shortcut": "x", "imageFileIds": many })),
    )
    .await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(out["data"]["issues"][0]["path"], json!(["imageFileIds"]));
    // Only images; unknown files 404.
    let (status, out) = call(
        &fx,
        "savedReplies.create",
        WS,
        &[],
        None,
        Some(json!({ "shortcut": "x", "imageFileIds": ["33"] })),
    )
    .await;
    assert_eq!((status, out["code"].clone()), (422, json!("validation")), "{out}");
    let (status, out) = call(
        &fx,
        "savedReplies.create",
        WS,
        &[],
        None,
        Some(json!({ "shortcut": "x", "imageFileIds": ["99"] })),
    )
    .await;
    assert_eq!((status, out["message"].clone()), (404, json!("MediaLibraryFile 99 not found")));
}

#[tokio::test]
async fn response_sla_get_update_and_delete() {
    let fx = fixture().await;
    // `get returns ... null when none is configured`.
    let (status, out) = call(&fx, "responseSla.get", WS, &[], None, None).await;
    assert_eq!((status, out), (200, Value::Null));
    // `update replaces the policy`: defaults for `enabled` and the threshold.
    let (status, out) = call(
        &fx,
        "responseSla.update",
        WS,
        &[],
        None,
        Some(json!({ "firstResponseTargetSeconds": 600, "nextResponseTargetSeconds": null })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["enabled"], true);
    assert_eq!(out["breachingThresholdPercent"], 25);
    assert_eq!(out["nextResponseTargetSeconds"], Value::Null);
    assert!(out["createdAt"].is_string());
    // `nextResponseTargetSeconds` is nullable, not optional; targets 60..2592000.
    let (status, _) = call(
        &fx,
        "responseSla.update",
        WS,
        &[],
        None,
        Some(json!({ "firstResponseTargetSeconds": 600 })),
    )
    .await;
    assert_eq!(status, 422);
    let (status, _) = call(
        &fx,
        "responseSla.update",
        WS,
        &[],
        None,
        Some(json!({ "firstResponseTargetSeconds": 59, "nextResponseTargetSeconds": null })),
    )
    .await;
    assert_eq!(status, 422);
    let (status, out) = call(&fx, "responseSla.get", WS, &[], None, None).await;
    assert_eq!((status, out["firstResponseTargetSeconds"].clone()), (200, json!(600)));
    // `delete returns 204 and removes the workspace policy`.
    let (status, _) = call(&fx, "responseSla.delete", WS, &[], None, None).await;
    assert_eq!(status, 204);
    let (_, out) = call(&fx, "responseSla.get", WS, &[], None, None).await;
    assert_eq!(out, Value::Null);
}
