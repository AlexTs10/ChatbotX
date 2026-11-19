//! The page queries and procedures of the automation screens (Q.F2-automation:
//! broadcasts, sequences, automated responses, triggers, coupons, contact
//! scans and imports, flow analytics) through `/rpc/actions/<name>` and
//! `/rpc/<procedure>`. Rust-native: the builder pages call the services in
//! server components, these check the wiring to the ported services.

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_db::rusqlite::params;
use rpc_common::{AGENT_TOKEN, NOW_MS, OUTSIDER_TOKEN, OWNER_TOKEN, WS, call, fixture};
use serde_json::{Value, json};

/// A page query (`serverQuery(name, workspaceId, [workspaceId, input])`).
async fn query(
    fx: &rpc_common::Fixture,
    token: &str,
    name: &str,
    input: Value,
) -> (StatusCode, Value) {
    let ws = WS.to_string();
    let (status, body) = call(
        fx,
        "POST",
        &format!("/rpc/actions/{name}"),
        Some(token),
        json!({ "args": [ws, input], "workspaceId": ws }),
    )
    .await;
    (status, body["json"].clone())
}

/// A procedure (`client.<router>.<name>(input)`).
async fn procedure(
    fx: &rpc_common::Fixture,
    token: &str,
    path: &str,
    input: Value,
) -> (StatusCode, Value) {
    let (status, body) = call(fx, "POST", &format!("/rpc/{path}"), Some(token), input).await;
    (status, body["json"].clone())
}

// ---- broadcasts -----------------------------------------------------------

async fn seed_broadcasts(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            for (id, name, status, schedules_type, deleted) in [
                (601, "Draft one", "draft", "now", None::<i64>),
                (602, "Sent one", "sent", "now", None),
                (603, "Weekly", "recurring", "recurring", None),
                (604, "Gone", "draft", "now", Some(NOW_MS)),
            ] {
                tx.execute(
                    r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "deletedAt", "createdAt", "updatedAt")
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'sendFlow', 'webchat', ?7, ?6, ?6)"#,
                    params![id, name, WS, status, schedules_type, NOW_MS + id, deleted],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "BroadcastRecurrence" ("id", "workspaceId", "broadcastId", "rule", "occurrenceCount")
                   VALUES (1, ?1, 603, '{"frequency":"weekly","interval":1,"timezone":"UTC","byWeekday":["MO"],"time":"09:00","startDate":"2027-01-01","end":{"type":"never"}}', 2)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed broadcasts");
}

#[tokio::test]
async fn broadcast_list_hides_deleted_rows_and_filters_by_kind() {
    let fx = fixture().await;
    seed_broadcasts(&fx).await;

    let (status, page) = query(
        &fx,
        OWNER_TOKEN,
        "listBroadcasts",
        json!({ "page": 1, "perPage": 10 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let mut names: Vec<&str> = page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["Draft one", "Sent one", "Weekly"]);
    assert_eq!(page["pageCount"], 1);

    let (_, repeating) = query(
        &fx,
        OWNER_TOKEN,
        "listBroadcasts",
        json!({ "kind": "repeating" }),
    )
    .await;
    let ids: Vec<&str> = repeating["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["603"]);

    // An unknown status parses as no filter (nuqs).
    let (_, all) = query(
        &fx,
        OWNER_TOKEN,
        "listBroadcasts",
        json!({ "status": "nope" }),
    )
    .await;
    assert_eq!(all["data"].as_array().unwrap().len(), 3);

    // Another workspace's member gets the `/rpc` workspace check.
    let (status, _) = query(&fx, OUTSIDER_TOKEN, "listBroadcasts", json!({})).await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn broadcast_calendar_lists_rows_in_the_window() {
    let fx = fixture().await;
    seed_broadcasts(&fx).await;
    let iso = |ms: i64| chatbotx_core::js::to_iso_string(ms).unwrap();
    let (status, rows) = query(
        &fx,
        OWNER_TOKEN,
        "listBroadcastsForCalendar",
        json!({ "from": iso(NOW_MS + 601), "to": iso(NOW_MS + 602), "status": null, "kind": null, "name": null }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    let ids: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["601", "602"]);

    let (status, _) = query(
        &fx,
        OWNER_TOKEN,
        "listBroadcastsForCalendar",
        json!({ "from": "x" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn broadcast_draft_and_runs_resolve_only_their_kind() {
    let fx = fixture().await;
    seed_broadcasts(&fx).await;

    let (status, draft) = query(
        &fx,
        OWNER_TOKEN,
        "findDraftBroadcast",
        json!({ "broadcastId": "601" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    assert_eq!(draft["id"], "601");
    assert_eq!(draft["targets"], json!([]));
    assert_eq!(draft["recurrence"], Value::Null);
    assert_eq!(draft["integrationWhatsapp"], Value::Null);
    for id in ["602", "604", "999"] {
        let (_, other) = query(
            &fx,
            OWNER_TOKEN,
            "findDraftBroadcast",
            json!({ "broadcastId": id }),
        )
        .await;
        assert_eq!(other, Value::Null, "{id}");
    }

    let (_, series) = query(
        &fx,
        OWNER_TOKEN,
        "findRecurringBroadcast",
        json!({ "broadcastId": "603" }),
    )
    .await;
    assert_eq!(series["id"], "603");
    assert_eq!(series["recurrence"]["occurrenceCount"], 2);
    let (_, one_off) = query(
        &fx,
        OWNER_TOKEN,
        "findRecurringBroadcast",
        json!({ "broadcastId": "601" }),
    )
    .await;
    assert_eq!(one_off, Value::Null);
    let (_, missing) = query(
        &fx,
        OWNER_TOKEN,
        "findRecurringBroadcast",
        json!({ "broadcastId": "nope" }),
    )
    .await;
    assert_eq!(missing, Value::Null);

    let (_, inbox) = query(
        &fx,
        OWNER_TOKEN,
        "findIntegrationWhatsappInboxId",
        json!({ "integrationWhatsappId": "77" }),
    )
    .await;
    assert_eq!(inbox, Value::Null);
}

#[tokio::test]
async fn broadcast_batch_stats_drop_deleted_ids() {
    let fx = fixture().await;
    seed_broadcasts(&fx).await;
    let (status, stats) = procedure(
        &fx,
        AGENT_TOKEN,
        "broadcastAPIs/privateGetBatchBroadcastStatsAPI",
        json!({ "workspaceId": WS.to_string(), "broadcastIds": ["601", "604"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    let keys: Vec<&String> = stats.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["601"]);
}

// ---- sequences ------------------------------------------------------------

async fn seed_sequence(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (701, 'Onboarding', ?1)"#,
                params![WS],
            )?;
            for (id, order) in [(712, 2), (711, 1)] {
                tx.execute(
                    r#"INSERT INTO "SequenceStep" ("id", "order", "delayDays", "sequenceId") VALUES (?1, ?2, 1, 701)"#,
                    params![id, order],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed sequence");
}

#[tokio::test]
async fn get_sequence_returns_its_steps_in_order_or_null() {
    let fx = fixture().await;
    seed_sequence(&fx).await;
    let ws = WS.to_string();
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/actions/getSequence",
        Some(OWNER_TOKEN),
        json!({ "args": [ws, "701"], "workspaceId": ws }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sequence = &body["json"];
    assert_eq!(sequence["name"], "Onboarding");
    let ids: Vec<&str> = sequence["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["711", "712"]);
    assert_eq!(sequence["steps"][0]["flow"], Value::Null);
    assert_eq!(sequence["sequenceSteps"], sequence["steps"]);

    let (_, missing) = call(
        &fx,
        "POST",
        "/rpc/actions/getSequence",
        Some(OWNER_TOKEN),
        json!({ "args": [ws, "999"], "workspaceId": ws }),
    )
    .await;
    assert_eq!(missing["json"], Value::Null);
}

#[tokio::test]
async fn sequence_step_stats_and_contacts_read_the_dispatches() {
    let fx = fixture().await;
    seed_sequence(&fx).await;
    let ws = WS.to_string();
    let (status, stats) = procedure(
        &fx,
        AGENT_TOKEN,
        "sequencesAPI/privateGetSequenceStepStatsAPI",
        json!({ "workspaceId": ws, "sequenceId": "701", "stepId": "711" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(
        stats,
        json!({ "message:sent": 0, "message:delivered": 0, "message:seen": 0, "flow:clicked": 0, "message:failed": 0 })
    );

    let (status, page) = procedure(
        &fx,
        AGENT_TOKEN,
        "sequencesAPI/privateListSequenceStepContactsAPI",
        json!({ "workspaceId": ws, "sequenceId": "701", "stepId": "711", "eventType": "message:seen" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page,
        json!({ "data": [], "total": 0, "page": 1, "pageCount": 0 })
    );

    let (status, _) = procedure(
        &fx,
        AGENT_TOKEN,
        "sequencesAPI/privateListSequenceStepContactsAPI",
        json!({ "workspaceId": ws, "sequenceId": "701", "stepId": "711", "eventType": "nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- coupons --------------------------------------------------------------

#[tokio::test]
async fn coupon_topics_list_with_total_options_and_archive() {
    let fx = fixture().await;
    let ws = WS.to_string();
    for name in ["Beta", "Alpha"] {
        let (status, topic) = procedure(
            &fx,
            OWNER_TOKEN,
            "couponsAPI/createCouponTopicAPI",
            json!({ "workspaceId": ws, "name": format!("  {name} "), "description": null }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{topic}");
        assert_eq!(topic["name"], name);
        assert_eq!(topic["createdById"], rpc_common::OWNER.to_string());
        assert_eq!(topic["deletedAt"], Value::Null);
    }
    let (status, dup) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/createCouponTopicAPI",
        json!({ "workspaceId": ws, "name": "alpha" }),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{dup}");

    let (_, page) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/listCouponTopicsAPI",
        json!({ "workspaceId": ws, "sort": [{ "id": "name", "desc": true }] }),
    )
    .await;
    let names: Vec<&str> = page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Beta", "Alpha"]);
    assert_eq!(page["total"], 2);
    assert_eq!(page["data"][0]["couponCount"], 0);

    let (_, options) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/listCouponTopicOptionsAPI",
        json!({ "workspaceId": ws, "keyword": "alp", "issueableOnly": true }),
    )
    .await;
    assert_eq!(options.as_array().unwrap().len(), 1);
    assert_eq!(options[0]["name"], "Alpha");
    let alpha = options[0]["id"].as_str().unwrap().to_owned();

    let (status, archived) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/archiveCouponTopicAPI",
        json!({ "workspaceId": ws, "topicId": alpha }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{archived}");
    assert_eq!(archived["status"], "archived");
    let (_, archived_page) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/listCouponTopicsAPI",
        json!({ "workspaceId": ws, "archived": true }),
    )
    .await;
    assert_eq!(archived_page["total"], 1);

    let (_, coupons) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/listCouponsAPI",
        json!({ "workspaceId": ws, "issueStatus": "published" }),
    )
    .await;
    assert_eq!(coupons, json!({ "data": [], "pageCount": 0, "total": 0 }));
    let (_, count) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/countCouponExportAPI",
        json!({ "workspaceId": ws }),
    )
    .await;
    assert_eq!(count, json!({ "count": 0 }));
    let (status, _) = procedure(
        &fx,
        OWNER_TOKEN,
        "couponsAPI/listCouponsAPI",
        json!({ "workspaceId": ws, "usageStatus": "nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn contact_coupons_need_contacts_access() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "createdAt", "updatedAt") VALUES (801, ?1, ?2, ?2)"#,
                params![WS, NOW_MS],
            )?;
            Ok(())
        })
        .await
        .expect("seed contact");
    let ws = WS.to_string();
    let (status, rows) = procedure(
        &fx,
        AGENT_TOKEN,
        "couponsAPI/listContactCouponsAPI",
        json!({ "workspaceId": ws, "contactId": "801" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows, json!([]));
}

// ---- keywords and triggers -----------------------------------------------

#[tokio::test]
async fn keyword_and_trigger_pages_are_super_admin_only() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            for (id, kind) in [(901, "inbound"), (902, "outbound")] {
                tx.execute(
                    r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "keywords", "status", "type", "createdAt", "updatedAt")
                       VALUES (?1, ?2, '["hi"]', 1, ?3, ?4, ?4)"#,
                    params![id, WS, kind, NOW_MS],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "Trigger" ("id", "name", "actions", "workspaceId", "createdAt", "updatedAt") VALUES (951, 'On tag', '[]', ?1, ?2, ?2)"#,
                params![WS, NOW_MS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");

    let (status, page) = query(
        &fx,
        OWNER_TOKEN,
        "listAutomatedResponses",
        json!({ "type": "inbound" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let ids: Vec<&str> = page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["901"]);
    let (_, found) = query(
        &fx,
        OWNER_TOKEN,
        "findAutomatedResponse",
        json!({ "id": "902", "type": "outbound" }),
    )
    .await;
    assert_eq!(found["id"], "902");
    let (_, wrong_type) = query(
        &fx,
        OWNER_TOKEN,
        "findAutomatedResponse",
        json!({ "id": "902", "type": "inbound" }),
    )
    .await;
    assert_eq!(wrong_type, Value::Null);
    let (status, _) = query(
        &fx,
        AGENT_TOKEN,
        "listAutomatedResponses",
        json!({ "type": "inbound" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, triggers) = query(
        &fx,
        OWNER_TOKEN,
        "getTriggers",
        json!({ "folderId": null, "name": "" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{triggers}");
    assert_eq!(triggers["data"][0]["id"], "951");
    assert_eq!(triggers["pageCount"], 1);
    let (_, edit) = query(&fx, OWNER_TOKEN, "triggerEditPage", json!({ "id": "951" })).await;
    assert_eq!(edit["trigger"]["name"], "On tag");
    assert_eq!(edit["storeSource"], "chatbotx");
    let (_, missing) = query(&fx, OWNER_TOKEN, "triggerEditPage", json!({ "id": "1" })).await;
    assert_eq!(missing, Value::Null);
    let (status, _) = query(&fx, AGENT_TOKEN, "triggerEditPage", json!({ "id": "951" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- contact scans and imports ---------------------------------------------

async fn seed_scan_inbox(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status")
                   VALUES (7, ?1, 'Page', 'messenger', 'page-1', 'connected')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
                   VALUES (77, '{"tokens":{"accessToken":"t"}}', 'page-1', 'Fb', '[]', '[]', '[]', ?1, 7)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed scan inbox");
}

async fn make_agent_assigned_only(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"UPDATE "WorkspaceMember" SET "permissions" = '{"onlyAssignedContacts":true}' WHERE "userId" = 102"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("assigned-only agent");
}

#[tokio::test]
async fn contact_scan_schedules_once_and_lists_the_run() {
    let fx = fixture().await;
    seed_scan_inbox(&fx).await;
    let past = "2020-01-01T00:00:00.000Z" /* the service checks the real clock */;

    let (status, out) = query(
        &fx,
        OWNER_TOKEN,
        "scheduleContactScanAction",
        json!({ "inboxId": "7", "scanFromAt": past }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let run_id: i64 = out["runId"].as_str().unwrap().parse().unwrap();
    let requested_by = rpc_common::scalar(
        &fx,
        r#"SELECT "requestedByUserId" FROM "CoexistSyncRun" WHERE "id" = ?1"#,
        run_id,
    )
    .await;
    assert_eq!(requested_by, json!(101));

    // A second scan while the first runs: a top-level validation error.
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/actions/scheduleContactScanAction",
        Some(OWNER_TOKEN),
        json!({ "args": [WS.to_string(), { "inboxId": "7", "scanFromAt": past }], "workspaceId": WS.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["json"]["data"]["validationErrors"]["_errors"][0],
        "A scan is already running for this inbox."
    );

    // An unknown inbox is the `inboxId` field error.
    let (_, body) = call(
        &fx,
        "POST",
        "/rpc/actions/scheduleContactScanAction",
        Some(OWNER_TOKEN),
        json!({ "args": [WS.to_string(), { "inboxId": "999", "scanFromAt": past }], "workspaceId": WS.to_string() }),
    )
    .await;
    assert_eq!(
        body["json"]["data"]["validationErrors"]["inboxId"]["_errors"][0],
        "Inbox not found."
    );

    let (status, view) = procedure(
        &fx,
        OWNER_TOKEN,
        "contactScanAPIs/getContactScanStatusAuthenticatedAPI",
        json!({ "workspaceId": WS.to_string(), "inboxId": "7" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["status"], "init");

    let (status, history) = query(
        &fx,
        OWNER_TOKEN,
        "listContactScanHistory",
        json!({ "page": 1 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["pageCount"], 1);
    assert_eq!(history["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn contact_scan_refuses_an_assigned_only_member() {
    let fx = fixture().await;
    seed_scan_inbox(&fx).await;
    make_agent_assigned_only(&fx).await;
    let past = "2020-01-01T00:00:00.000Z" /* the service checks the real clock */;

    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/actions/scheduleContactScanAction",
        Some(AGENT_TOKEN),
        json!({ "args": [WS.to_string(), { "inboxId": "7", "scanFromAt": past }], "workspaceId": WS.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["json"]["data"]["validationErrors"]["_errors"][0],
        "You do not have permission to run an Automatic Customer Scan."
    );
    let (status, _) = procedure(
        &fx,
        AGENT_TOKEN,
        "contactScanAPIs/getContactScanStatusAuthenticatedAPI",
        json!({ "workspaceId": WS.to_string(), "inboxId": "7" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = query(&fx, AGENT_TOKEN, "listContactScanHistory", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The import history only needs contacts access.
    let (status, _) = query(
        &fx,
        AGENT_TOKEN,
        "listImports",
        json!({ "type": "contacts" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn import_history_and_start_check_the_page_gates() {
    let fx = fixture().await;
    seed_scan_inbox(&fx).await;

    let (status, page) = query(
        &fx,
        OWNER_TOKEN,
        "listImports",
        json!({ "type": "contacts" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page, json!({ "data": [], "pageCount": 0 }));
    // The flow history needs `flows`; the agent has contacts only.
    let (status, _) = query(&fx, AGENT_TOKEN, "listImports", json!({ "type": "flow" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = query(&fx, OWNER_TOKEN, "listImports", json!({ "type": "nope" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A missing file is the `fileId` field error.
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/actions/importContactsAction",
        Some(OWNER_TOKEN),
        json!({ "args": [WS.to_string(), { "fileId": "424242", "channel": "messenger", "inboxId": "7", "contactId": "id" }], "workspaceId": WS.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["json"]["data"]["validationErrors"]["fileId"]["_errors"][0].is_string(),
        "{body}"
    );
    // The schema refine: a messenger import needs the contact id column.
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/actions/importContactsAction",
        Some(OWNER_TOKEN),
        json!({ "args": [WS.to_string(), { "fileId": "424242", "channel": "messenger", "inboxId": "7" }], "workspaceId": WS.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["json"]["data"]["validationErrors"]["contactId"].is_object(),
        "{body}"
    );

    let (status, template) =
        query(&fx, OWNER_TOKEN, "getContactsImportTemplate", json!(null)).await;
    assert_eq!(status, StatusCode::OK, "{template}");
    assert_eq!(template["mimeType"], "text/csv");
    let (status, _) = query(
        &fx,
        OUTSIDER_TOKEN,
        "getContactsImportTemplate",
        json!(null),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}

// ---- flow analytics --------------------------------------------------------

#[tokio::test]
async fn flow_analytics_stats_need_the_flows_permission() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Flow')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
                   VALUES (611, ?1, 61, '[{"id":"9","type":"wait","data":{"details":{"steps":[{"id":"s1"}]}}}]', '[]', 1, 1, 9)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed flow");

    let (status, out) = query(
        &fx,
        OWNER_TOKEN,
        "flowAnalyticsStats",
        json!({ "id": "61" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["stats"], json!({}));
    assert_eq!(
        out["smartDelayStats"],
        json!({ "9": { "waiting": 0, "sent": 0 } })
    );
    let (status, _) = query(
        &fx,
        AGENT_TOKEN,
        "flowAnalyticsStats",
        json!({ "id": "61" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, stats) = procedure(
        &fx,
        OWNER_TOKEN,
        "flowsAPI/privateGetFlowStatsAPI",
        json!({ "workspaceId": WS.to_string(), "flowId": "61" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats, json!({}));
    let (status, _) = procedure(
        &fx,
        OWNER_TOKEN,
        "flowsAPI/privateResetFlowStatsAPI",
        json!({ "workspaceId": WS.to_string(), "flowId": "61" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sessions = rpc_common::scalar(
        &fx,
        r#"SELECT COUNT(*) FROM "FlowAnalyticsSession" WHERE "flowId" = ?1 AND "deletedAt" IS NULL"#,
        61,
    )
    .await;
    assert_eq!(sessions, json!(1));
}

/// `page-automated-responses-route-guards.test.ts`: the outbound keywords
/// pages open for a super admin and stay closed for a member with every
/// other permission.
#[tokio::test]
async fn outbound_keywords_need_super_admin_not_full_permissions() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"UPDATE "WorkspaceMember" SET "permissions" = '{"contacts":true,"flows":true,"broadcast":true,"analytics":true,"inbox":true,"emailAndPhone":true,"ecommerce":true}' WHERE "userId" = 102"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("full agent");
    let input = json!({ "type": "outbound" });
    let (status, _) = query(&fx, OWNER_TOKEN, "listAutomatedResponses", input.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = query(&fx, AGENT_TOKEN, "listAutomatedResponses", input).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- broadcast audience (contacts procedures) -------------------------------

#[tokio::test]
async fn broadcast_audience_count_and_preview_need_contacts_access() {
    let fx = fixture().await;
    let ws = WS.to_string();
    let (status, count) = procedure(
        &fx,
        OWNER_TOKEN,
        "contactsAPIs/countContactInboxesAuthenticatedAPI",
        json!({ "workspaceId": ws, "sort": [], "channels": ["webchat"], "countMode": "bounded" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{count}");
    assert_eq!(count["total"], 0);
    let (status, preview) = procedure(
        &fx,
        AGENT_TOKEN,
        "contactsAPIs/listContactInboxesAudiencePreviewAuthenticatedAPI",
        json!({ "workspaceId": ws, "page": 1, "perPage": 20, "channels": ["webchat"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview, json!({ "data": [] }));
    let (status, _) = procedure(
        &fx,
        OWNER_TOKEN,
        "contactsAPIs/listContactInboxesAudiencePreviewAuthenticatedAPI",
        json!({ "workspaceId": ws, "perPage": 51 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = procedure(
        &fx,
        OUTSIDER_TOKEN,
        "contactsAPIs/countContactInboxesAuthenticatedAPI",
        json!({ "workspaceId": ws }),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}
