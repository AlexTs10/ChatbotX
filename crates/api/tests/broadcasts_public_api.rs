//! Port of `apps/builder/__tests__/broadcasts-public-api.test.ts` (Q.D5).
//! Adapted: the mocked `broadcastService` becomes `handlers::dispatch` over
//! real rows; "called with X" becomes the answer and the stored row.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const FUTURE: &str = "2030-01-07T09:00:00.000Z";

/// Flow 61 "Promo flow" (WS), flow 69 (`OTHER_WS`); WhatsApp inbox 71 with
/// integration 72 "Page" and approved template 73 (`Hi {{1}}`).
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Promo flow'), (69, ?2, 'Far')"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (71, 'Page', 'whatsapp', 's71', ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (72, '{}', 'p', 'w', 'b', 'Page', ?1, 71)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "WhatsappMessageTemplate" ("id", "name", "integrationWhatsappId", "sourceId", "language", "category", "status", "components")
                   VALUES (73, 'promo', 72, 't', 'en', 'MARKETING', 'APPROVED', '[{"type":"BODY","text":"Hi {{1}}","example":{"body_text":[["Ann"]]}}]')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

fn body(overrides: Value) -> Value {
    let mut out = json!({
        "channel": "omnichannel",
        "subaction": "allContacts",
        "flowId": "61",
        "schedulesType": "future",
        "schedulesAt": FUTURE,
        "contactFilter": { "operator": "and", "conditions": [] },
    });
    for (k, v) in overrides.as_object().expect("object") {
        if v.is_null() {
            out.as_object_mut().expect("object").remove(k);
        } else {
            out[k] = v.clone();
        }
    }
    out
}

async fn create(f: &Fixture, overrides: Value) -> Value {
    let (status, out) = call(f, "broadcasts.create", WS, &[], None, Some(body(overrides))).await;
    assert_eq!(status, 201, "{out}");
    out
}

fn issue_paths(out: &Value) -> Vec<String> {
    out["data"]["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            i["path"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_owned))
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

#[test]
fn registers_the_broadcasts_public_router_under_the_broadcasts_scope() {
    for route in chatbotx_api::handlers::broadcasts::ROUTES {
        assert_eq!(
            route_by_id(route).expect("route").auth,
            RouteAuth::WorkspaceToken(&["broadcasts"]),
            "{route}"
        );
    }
}

// describe("POST /v1/broadcasts")

#[tokio::test]
async fn sources_workspace_id_from_context_and_treats_the_token_caller_as_fully_privileged() {
    let f = seeded().await;
    // An email condition survives (fully privileged token caller).
    let filter = json!({ "operator": "and", "conditions": [{ "field": "email", "operator": "isNotEmpty" }] });
    let out = create(&f, json!({ "contactFilter": filter })).await;
    assert_eq!(out["name"], "Promo flow");
    assert_eq!(out["status"], "scheduled");
    assert_eq!(out["flowId"], "61");
    assert!(out.get("workspaceId").is_none());
    let id: i64 = out["id"].as_str().expect("id").parse().expect("id");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "Broadcast" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
    assert_eq!(out["contactFilter"]["conditions"][0]["field"], "email");
}

#[tokio::test]
async fn forwards_save_as_draft_true_unchanged() {
    let f = seeded().await;
    let out = create(
        &f,
        json!({ "saveAsDraft": true, "schedulesAt": "2020-01-01T00:00:00.000Z" }),
    )
    .await;
    assert_eq!(out["status"], "draft");
}

#[tokio::test]
async fn create_validates_the_payload_and_truncates_the_schedule() {
    let f = seeded().await;
    let (status, out) = call(&f, "broadcasts.create", WS, &[], None, Some(json!({}))).await;
    assert_eq!(status, 422);
    let paths = issue_paths(&out);
    for path in ["channel", "subaction", "schedulesType", "schedulesAt"] {
        assert!(paths.contains(&path.to_owned()), "{path} in {paths:?}");
    }
    for (overrides, path) in [
        (json!({ "flowId": null }), "flowId"),
        (json!({ "templateId": "123" }), "flowId"),
        (json!({ "schedulesType": "recurring" }), "recurrence"),
        (
            json!({ "schedulesAt": "2020-01-01T00:00:00.000Z" }),
            "schedulesAt",
        ),
        (
            json!({ "audienceRangeStart": 5, "audienceRangeEnd": 2 }),
            "audienceRange",
        ),
        (json!({ "sendRatePerMinute": 1001 }), "sendRatePerMinute"),
        (json!({ "flowId": "abc" }), "flowId"),
    ] {
        let (status, out) = call(
            &f,
            "broadcasts.create",
            WS,
            &[],
            None,
            Some(body(overrides.clone())),
        )
        .await;
        assert_eq!(status, 422, "{overrides}");
        assert!(
            issue_paths(&out).contains(&path.to_owned()),
            "{overrides}: {out}"
        );
    }
    let (status, out) = call(
        &f,
        "broadcasts.create",
        WS,
        &[],
        None,
        Some(body(json!({ "flowId": "69" }))),
    )
    .await;
    assert_eq!(
        (status, out["code"].clone(), out["message"].clone()),
        (422, json!("validation"), json!("Flow not found"))
    );
    let (status, out) = call(
        &f,
        "broadcasts.create",
        WS,
        &[],
        None,
        Some(body(json!({ "channel": "webchat" }))),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (422, json!("Unsupported broadcast channel"))
    );
    let out = create(
        &f,
        json!({ "schedulesAt": "2030-01-07T09:00:42.123Z", "sendRatePerMinute": 30 }),
    )
    .await;
    assert_eq!(out["schedulesAt"], FUTURE);
    assert_eq!(out["sendRatePerMinute"], 30);
}

// describe("templateParams on create and updateDraft")

fn template_body(overrides: Value) -> Value {
    let mut out = body(
        json!({ "channel": "whatsapp", "subaction": "whatsappTemplateMessage", "flowId": null }),
    );
    for (k, v) in overrides.as_object().expect("object") {
        out[k] = v.clone();
    }
    out
}

#[tokio::test]
async fn create_stores_the_template_data_built_from_template_params() {
    let f = seeded().await;
    let (status, out) = call(
        &f,
        "broadcasts.create",
        WS,
        &[],
        None,
        Some(template_body(json!({ "targets": [{ "inboxId": "71", "templateId": "73", "templateParams": { "body.1": "Ann" } }] }))),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    let id: i64 = out["id"].as_str().expect("id").parse().expect("id");
    let stored = scalar(
        &f,
        r#"SELECT "templateData" FROM "BroadcastTarget" WHERE "broadcastId" = ?1"#,
        id,
    )
    .await
    .expect("target");
    let stored: Value = serde_json::from_str(stored.as_str().expect("text")).expect("json");
    assert_eq!(stored["body"][0]["text"], "Ann", "{stored}");
    assert!(stored.get("templateParams").is_none());
    assert_eq!(out["name"], "Page - promo");
}

#[tokio::test]
async fn update_draft_resolves_per_target_template_params() {
    let f = seeded().await;
    let draft = create(&f, json!({ "saveAsDraft": true })).await;
    let id = draft["id"].as_str().expect("id").to_owned();
    let (status, out) = call(
        &f,
        "broadcasts.updateDraft",
        WS,
        &[("id", &id)],
        None,
        Some(template_body(json!({ "saveAsDraft": true, "targets": [{ "inboxId": "71", "templateId": "73", "templateParams": { "body.9": "x" } }] }))),
    )
    .await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(out["code"], "validation");
    assert!(
        out["message"]
            .as_str()
            .expect("message")
            .starts_with("Template parameters do not match the template")
    );
}

#[tokio::test]
async fn a_template_outside_the_workspace_stops_the_create_declared_404() {
    let f = seeded().await;
    let (status, out) = call(
        &f,
        "broadcasts.create",
        WS,
        &[],
        None,
        Some(template_body(json!({ "templateId": "999", "templateParams": { "body.1": "Ann" }, "targets": [{ "inboxId": "71", "templateId": "999" }] }))),
    )
    .await;
    assert_eq!(
        (status, out["code"].clone(), out["defined"].clone()),
        (404, json!("notFound"), json!(true)),
        "{out}"
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "Broadcast" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(json!(0))
    );
}

// describe("PUT /v1/broadcasts/{id}")

#[tokio::test]
async fn renames_the_broadcast_then_re_fetches_it_from_context_s_workspace() {
    let f = seeded().await;
    let draft = create(&f, json!({ "saveAsDraft": true })).await;
    let id = draft["id"].as_str().expect("id").to_owned();
    let (status, out) = call(
        &f,
        "broadcasts.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "  New name  " })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        (
            out["id"].clone(),
            out["name"].clone(),
            out["recurrence"].clone()
        ),
        (json!(id), json!("New name"), Value::Null)
    );
    let (status, _) = call(
        &f,
        "broadcasts.update",
        OTHER_WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "x" })),
    )
    .await;
    assert_eq!(status, 404);
}

// describe("PUT /v1/broadcasts/{id}/draft")

#[tokio::test]
async fn delegates_to_update_draft_with_the_token_caller_treated_as_fully_privileged() {
    let f = seeded().await;
    let draft = create(&f, json!({ "saveAsDraft": true })).await;
    let id = draft["id"].as_str().expect("id").to_owned();
    let (status, out) = call(
        &f,
        "broadcasts.updateDraft",
        WS,
        &[("id", &id)],
        None,
        Some(body(json!({ "saveAsDraft": true }))),
    )
    .await;
    assert_eq!(
        (status, out.clone()),
        (200, json!({ "id": id, "status": "draft" }))
    );
    let (status, out) = call(
        &f,
        "broadcasts.updateDraft",
        WS,
        &[("id", &id)],
        None,
        Some(body(json!({ "sendRatePerMinute": 10 }))),
    )
    .await;
    assert_eq!(
        (status, out),
        (200, json!({ "id": id, "status": "scheduled" }))
    );
    let (status, out) = call(
        &f,
        "broadcasts.updateDraft",
        WS,
        &[("id", &id)],
        None,
        Some(body(json!({ "saveAsDraft": true }))),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (400, json!("Broadcast is not a draft"))
    );
}

// describe("POST /v1/broadcasts/{id}/schedule")

async fn draft_id(f: &Fixture) -> String {
    create(f, json!({ "saveAsDraft": true })).await["id"]
        .as_str()
        .expect("id")
        .to_owned()
}

#[tokio::test]
async fn resolves_now_to_the_current_minute_truncated_time() {
    let f = seeded().await;
    let id = draft_id(&f).await;
    let (status, out) = call(
        &f,
        "broadcasts.schedule",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "schedulesType": "now", "schedulesAt": null })),
    )
    .await;
    assert_eq!((status, out), (200, json!({ "id": id })));
    let at = scalar(
        &f,
        r#"SELECT "schedulesAt" FROM "Broadcast" WHERE "id" = ?1"#,
        id.parse().expect("id"),
    )
    .await
    .and_then(|v| v.as_i64())
    .expect("schedulesAt");
    assert_eq!(at % 60_000, 0);
    assert!(at <= chatbotx_db::repo::now_ms());
}

#[tokio::test]
async fn passes_a_future_time_through_unchanged_minute_truncated() {
    let f = seeded().await;
    let id = draft_id(&f).await;
    let (status, _) = call(
        &f,
        "broadcasts.schedule",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "schedulesType": "future", "schedulesAt": "2030-01-01T09:30:20.000Z" })),
    )
    .await;
    assert_eq!(status, 200);
    let (_, out) = call(&f, "broadcasts.get", WS, &[("idOrName", &id)], None, None).await;
    assert_eq!(out["schedulesAt"], "2030-01-01T09:30:00.000Z");
}

#[tokio::test]
async fn forwards_the_optional_send_rate() {
    let f = seeded().await;
    let id = draft_id(&f).await;
    let (status, _) = call(
        &f,
        "broadcasts.schedule",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "schedulesType": "now", "schedulesAt": null, "sendRatePerMinute": 120 })),
    )
    .await;
    assert_eq!(status, 200);
    let (_, out) = call(&f, "broadcasts.get", WS, &[("idOrName", &id)], None, None).await;
    assert_eq!(out["sendRatePerMinute"], 120);
    let (status, out) = call(
        &f,
        "broadcasts.schedule",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "schedulesType": "now", "schedulesAt": null })),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (400, json!("Broadcast is not a draft"))
    );
}

// describe("POST /v1/broadcasts/{id}/move-to-draft") / stop / resume

#[tokio::test]
async fn delegates_to_move_to_draft_scoped_to_context_s_workspace() {
    let f = seeded().await;
    let id = create(&f, json!({})).await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let (status, _) = call(
        &f,
        "broadcasts.moveToDraft",
        OTHER_WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400);
    let (status, out) = call(
        &f,
        "broadcasts.moveToDraft",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!((status, out), (200, json!({ "id": id })));
    let (_, out) = call(&f, "broadcasts.get", WS, &[("idOrName", &id)], None, None).await;
    assert_eq!(
        (out["status"].clone(), out["contactCount"].clone()),
        (json!("draft"), Value::Null)
    );
}

#[tokio::test]
async fn delegates_to_stop_sending_scoped_to_context_s_workspace() {
    let f = seeded().await;
    let id = create(&f, json!({})).await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    // Only a sending broadcast stops.
    let (status, _) = call(
        &f,
        "broadcasts.stop",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400);
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Broadcast" SET "status" = 'sending', "contactCount" = 3"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("sending");
    let (status, out) = call(
        &f,
        "broadcasts.stop",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!((status, out), (200, json!({ "id": id })));
    let (status, out) = call(
        &f,
        "broadcasts.resume",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "sendRatePerMinute": 120 })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["id"], json!(id));
    let (_, out) = call(&f, "broadcasts.get", WS, &[("idOrName", &id)], None, None).await;
    assert_eq!(
        (out["status"].clone(), out["sendRatePerMinute"].clone()),
        (json!("sending"), json!(120))
    );
}

#[tokio::test]
async fn declares_the_optional_nullable_send_rate_input() {
    let f = seeded().await;
    let (status, out) = call(
        &f,
        "broadcasts.resume",
        WS,
        &[("id", "1")],
        None,
        Some(json!({ "sendRatePerMinute": 1001 })),
    )
    .await;
    assert_eq!(
        (status, issue_paths(&out)),
        (422, vec!["sendRatePerMinute".to_owned()])
    );
    let (status, _) = call(
        &f,
        "broadcasts.resume",
        WS,
        &[("id", "1")],
        None,
        Some(json!({ "sendRatePerMinute": null })),
    )
    .await;
    // Passes the schema; the broadcast does not exist.
    assert_eq!(status, 400);
}

// describe("POST /v1/broadcasts/{id}/resend") / duplicate / clone

#[tokio::test]
async fn delegates_to_resend_with_pruning_treating_the_token_caller_as_fully_privileged() {
    let f = seeded().await;
    let id = create(&f, json!({})).await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let (status, out) = call(
        &f,
        "broadcasts.resend",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (400, json!("Broadcast is not sent"))
    );
    f.state
        .db
        .write(move |tx| {
            tx.execute(r#"UPDATE "Broadcast" SET "status" = 'sent'"#, [])?;
            Ok(())
        })
        .await
        .expect("sent");
    let (status, out) = call(
        &f,
        "broadcasts.resend",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(
        (
            out["name"].clone(),
            out["status"].clone(),
            out["schedulesType"].clone()
        ),
        (
            json!("Promo flow (Resend)"),
            json!("scheduled"),
            json!("now")
        )
    );
    assert_ne!(out["id"], json!(id));
}

#[tokio::test]
async fn delegates_to_the_same_clone_broadcast_call_as_broadcasts_duplicate() {
    let f = seeded().await;
    let id = create(&f, json!({ "sendRatePerMinute": 20 })).await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let (status, first) = call(
        &f,
        "broadcasts.duplicate",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 201, "{first}");
    assert_eq!(
        (
            first["name"].clone(),
            first["status"].clone(),
            first["sendRatePerMinute"].clone()
        ),
        (json!("Promo flow (Copy 1)"), json!("draft"), json!(20))
    );
    let (_, second) = call(
        &f,
        "broadcasts.clone",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(second["name"], "Promo flow (Copy 2)");
    let copy = first["id"].as_str().expect("id").to_owned();
    let (_, third) = call(
        &f,
        "broadcasts.duplicate",
        WS,
        &[("id", &copy)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(third["name"], "Promo flow (Copy 3)");
    let (status, _) = call(
        &f,
        "broadcasts.clone",
        OTHER_WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 404);
}

// describe("DELETE /v1/broadcasts/{id}")

#[tokio::test]
async fn delegates_to_soft_delete_broadcasts_with_a_single_id_array_scoped_to_context_s_workspace()
{
    let f = seeded().await;
    let id = draft_id(&f).await;
    let (status, _) = call(
        &f,
        "broadcasts.delete",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(&f, "broadcasts.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&f, "broadcasts.get", WS, &[("idOrName", &id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn _404s_instead_of_a_bare_204_when_the_id_is_nonexistent_foreign_or_already_deleted() {
    let f = seeded().await;
    let (status, out) = call(&f, "broadcasts.delete", WS, &[("id", "1")], None, None).await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Broadcast not found or cannot be deleted"))
    );
}

// The list, get, audience preview and recurrence preview routes.

#[tokio::test]
async fn lists_newest_first_and_filters() {
    let f = seeded().await;
    let draft = draft_id(&f).await;
    let scheduled = create(&f, json!({ "schedulesAt": "2030-02-01T00:00:00.000Z" })).await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let ids = |out: &Value| -> Vec<String> {
        out["data"]
            .as_array()
            .expect("data")
            .iter()
            .map(|b| b["id"].as_str().expect("id").to_owned())
            .collect()
    };
    let (status, out) = call(&f, "broadcasts.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(ids(&out), vec![scheduled.clone(), draft.clone()]);
    assert_eq!(out["pageCount"], 1);
    let (_, out) = call(&f, "broadcasts.list", WS, &[], Some("status=draft"), None).await;
    assert_eq!(ids(&out), vec![draft.clone()]);
    let (_, out) = call(
        &f,
        "broadcasts.list",
        WS,
        &[],
        Some("scheduledFrom=2030-01-15T00:00:00.000Z"),
        None,
    )
    .await;
    assert_eq!(ids(&out), vec![scheduled]);
    let (status, _) = call(&f, "broadcasts.list", WS, &[], Some("status=paused"), None).await;
    assert_eq!(status, 422);
    let (status, out) = call(
        &f,
        "broadcasts.get",
        WS,
        &[("idOrName", "Promo flow")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert!(out["targets"].is_array() && out["recurrence"].is_null());
}

#[tokio::test]
async fn previews_the_audience_and_the_recurrence() {
    let f = seeded().await;
    let (status, out) = call(
        &f,
        "broadcasts.previewAudience",
        WS,
        &[],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(
        (status, out["total"].clone(), out["data"].clone()),
        (200, json!(0), json!([]))
    );
    let (status, out) = call(
        &f,
        "broadcasts.previewAudience",
        WS,
        &[],
        None,
        Some(json!({ "perPage": 51 })),
    )
    .await;
    assert_eq!(
        (status, issue_paths(&out)),
        (422, vec!["perPage".to_owned()])
    );
    let rule = json!({
        "frequency": "daily",
        "interval": 1,
        "timeOfDay": "09:00",
        "timezone": "UTC",
        "startsOn": "2030-01-01",
        "end": { "kind": "never" },
    });
    let (status, out) = call(
        &f,
        "broadcasts.previewRecurrence",
        WS,
        &[],
        None,
        Some(json!({ "recurrence": rule })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["upcomingRuns"].as_array().expect("runs").len(), 3);
    assert_eq!(out["upcomingRuns"][0], "2030-01-01T09:00:00.000Z");
    assert_eq!(out["timezone"], "UTC");
}
