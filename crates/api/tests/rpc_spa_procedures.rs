//! The `/rpc` procedures and `/rpc/actions/*` server actions of the SPA
//! (`crate::rpc_procedures`, P4): one test per group against a migrated
//! in-memory database, through the `/rpc` router (session cookie, workspace
//! check, RPC encoding with `Date` meta). Rust-native: the builder tests mock
//! the services, these check the wiring to the ported ones.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::{RpcState, rpc_router_with};
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "rpc-spa-procedures-secret";
const NOW_MS: i64 = 1_800_000_000_000;
const ADMIN_ID: i64 = 101;
const ASSIGNED_ID: i64 = 102;
const WS: i64 = 201;
const TAG_ID: i64 = 301;
const FIELD_ID: i64 = 302;
const CONTACT_ID: i64 = 401;

fn now_ms() -> i64 {
    NOW_MS
}

async fn router() -> (axum::Router, Arc<Db>) {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        for (id, email, token) in [
            (ADMIN_ID, "admin@spa.test", "admintoken"),
            (ASSIGNED_ID, "assigned@spa.test", "assignedtoken"),
        ] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "name", "emailVerified") VALUES (?1, ?2, 'Ann', 1)"#,
                params![id, email],
            )?;
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (?1, ?2, ?1, ?3)"#,
                params![id, token, NOW_MS + 60_000],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'ws', ?2)"#,
            params![WS, ADMIN_ID],
        )?;
        for (id, user, role, permissions) in [
            (501, ADMIN_ID, "owner", r#"{"superAdmin":true}"#),
            (502, ASSIGNED_ID, "agent", r#"{"onlyAssignedContacts":true}"#),
        ] {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions")
                   VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![id, WS, user, role, permissions],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Tag" ("id", "name", "workspaceId", "createdAt", "updatedAt") VALUES (?1, 'vip', ?2, ?3, ?3)"#,
            params![TAG_ID, WS, NOW_MS],
        )?;
        tx.execute(
            r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (?1, 'plan', 'shortText', ?2)"#,
            params![FIELD_ID, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName", "email", "createdAt", "updatedAt")
               VALUES (?1, ?2, 'Ada', 'Lovelace', 'ada@spa.test', ?3, ?3)"#,
            params![CONTACT_ID, WS, NOW_MS],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let db = Arc::new(db);
    let auth = Arc::new(AuthState {
        db: Arc::clone(&db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                broker_url: None,
                builder_url: "http://app".to_string(),
                next_phase: None,
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    let api = Arc::new(ApiState::new(Arc::clone(&db), "http://app"));
    let router = rpc_router_with(RpcState {
        auth,
        api: Some(api),
        realtime_secret: None,
    });
    (router, db)
}

/// POSTs `{ json: input }` to `/rpc/<path>`: `(status, json, meta)`.
async fn post(
    router: &axum::Router,
    path: &str,
    token: &str,
    input: Value,
) -> (StatusCode, Value, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/rpc/{path}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value(token, SECRET)
            ),
        )
        .body(Body::from(json!({ "json": input }).to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    (status, body["json"].clone(), body["meta"].clone())
}

fn ws() -> String {
    WS.to_string()
}

#[tokio::test]
async fn option_lists_return_the_builder_resources_with_date_meta() {
    let (router, _) = router().await;
    let (status, json, meta) = post(
        &router,
        "tagsAPI/privateListWorkspaceTagsAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["pageCount"], 1);
    let tag = &json["data"][0];
    assert_eq!(tag["id"], TAG_ID.to_string());
    assert_eq!(tag["workspaceId"], ws());
    assert_eq!(tag["folderId"], Value::Null);
    assert_eq!(tag["createdAt"], "2027-01-15T08:00:00.000Z");
    assert!(
        meta.as_array()
            .unwrap()
            .contains(&json!([1, "data", 0, "createdAt"]))
    );

    let (status, json, _) = post(
        &router,
        "customFieldsAPI/privateListCustomFieldsAPI",
        "admintoken",
        json!({ "workspaceId": ws(), "name": "pl" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"][0]["name"], "plan");

    let (status, json, _) = post(
        &router,
        "workspaceMembersAPI/listWorkspaceMembersAuthenticatedAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"].as_array().unwrap().len(), 2);
    assert_eq!(json["data"][0]["user"]["name"], "Ann");
    assert_eq!(json["data"][0]["permissions"]["superAdmin"], true);
}

#[tokio::test]
async fn contacts_list_detail_and_tag_actions_follow_the_member_scope() {
    let (router, _) = router().await;
    let (status, json, _) = post(
        &router,
        "contactsAPIs/listContactsByPOSTAuthenticatedAPI",
        "admintoken",
        json!({ "workspaceId": ws(), "keyword": "ada" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["totalCount"], 1);
    assert_eq!(json["data"][0]["fullName"], "Ada Lovelace");
    assert_eq!(json["data"][0]["conversation"], Value::Null);

    let (status, json, _) = post(
        &router,
        "actions/addContactTagAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "ids": [CONTACT_ID.to_string()], "tags": ["vip"] }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["processedContactIds"], json!([CONTACT_ID.to_string()]));

    let (status, json, _) = post(
        &router,
        "contactsAPIs/getContactAuthenticatedAPI",
        "admintoken",
        json!({ "workspaceId": ws(), "contactId": CONTACT_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["email"], "ada@spa.test");
    assert_eq!(json["tags"][0]["name"], "vip");

    // An assigned-only member sees no unassigned contact (and no PII).
    let (status, json, _) = post(
        &router,
        "contactsAPIs/listContactsByPOSTAuthenticatedAPI",
        "assignedtoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["totalCount"], 0);
    let (status, json, _) = post(
        &router,
        "contactsAPIs/getContactAuthenticatedAPI",
        "assignedtoken",
        json!({ "workspaceId": ws(), "contactId": CONTACT_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["message"], "Contact not found");
    let (_, json, _) = post(
        &router,
        "actions/deleteContactAction",
        "assignedtoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "ids": [CONTACT_ID.to_string()] }] }),
    )
    .await;
    assert_eq!(json["skippedContactIds"], json!([]));
    assert_eq!(json["processedContactIds"], json!([]));
}

#[tokio::test]
async fn flows_and_folders_round_trip_through_the_actions() {
    let (router, _) = router().await;
    let (status, folder, _) = post(
        &router,
        "actions/createFolderAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "name": " Sales ", "parentId": null, "folderType": "flow" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{folder}");
    let (_, folders, _) = post(
        &router,
        "foldersAPI/listFoldersAuthenticatedAPI",
        "admintoken",
        json!({ "workspaceId": ws(), "folderType": "flow" }),
    )
    .await;
    assert_eq!(folders["data"][0]["name"], "Sales");
    let folder_id = folders["data"][0]["id"].as_str().unwrap().to_string();

    let (status, flow, _) = post(
        &router,
        "actions/createFlowAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "name": "Welcome", "folderId": null }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{flow}");
    let flow_id = flow["id"].as_str().unwrap().to_string();
    assert_eq!(flow["name"], "Welcome");

    let (status, _, _) = post(
        &router,
        "actions/changeFolderAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "folderType": "flow", "modelIds": [flow_id], "newFolderId": folder_id }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, list, _) = post(
        &router,
        "flowsAPI/privateListFlowsAPI",
        "admintoken",
        json!({ "workspaceId": ws(), "folderId": folder_id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["data"][0]["id"], flow_id);
    assert_eq!(list["data"][0]["flowVersions"][0]["isDraft"], true);

    let (status, _, _) = post(
        &router,
        "actions/updateFlowAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), flow_id, { "name": "Hello" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, list, _) = post(
        &router,
        "flowsAPI/privateListFlowsAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(list["data"][0]["name"], "Hello");

    let (status, _, _) = post(
        &router,
        "actions/deleteFlowAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "ids": [flow_id] }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, list, _) = post(
        &router,
        "flowsAPI/privateListFlowsAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(list["data"], json!([]));
}

#[tokio::test]
async fn the_first_api_channel_creates_the_workspace() {
    let (router, db) = router().await;
    let (status, json, _) = post(
        &router,
        "actions/createApiAction",
        "assignedtoken",
        json!({ "args": [{ "name": "My bot" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let workspace_id: i64 = json["workspaceId"].as_str().unwrap().parse().unwrap();
    assert!(json["token"].as_str().is_some_and(|t| !t.is_empty()));
    let (name, role): (String, String) = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT w."name", m."role" FROM "Workspace" w JOIN "WorkspaceMember" m ON m."workspaceId" = w."id"
                   WHERE w."id" = ?1 AND m."userId" = ?2"#,
                params![workspace_id, ASSIGNED_ID],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!((name.as_str(), role.as_str()), ("My bot", "owner"));

    // Someone else's workspace is "Workspace not found".
    let (status, json, _) = post(
        &router,
        "actions/createApiAction",
        "assignedtoken",
        json!({ "args": [{ "name": "x", "workspaceId": "999" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["message"], "Workspace not found");
}

#[tokio::test]
async fn ai_procedures_and_actions_are_wired() {
    let (router, _) = router().await;
    let agent = json!({
        "name": "Support agent",
        "prompt": "You answer questions.",
        "messages": [{ "role": "user", "content": "Hello" }],
        "models": [{ "provider": "openai", "model": "gpt-4o-mini" }],
        "temperature": 0.4,
        "maxOutputTokens": 500,
        "tools": [],
        "webSearchAuthorizedDomains": [],
        "isDefault": false,
    });
    let (status, out, _) = post(
        &router,
        "actions/createAIAgentAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), agent] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, list, meta) = post(
        &router,
        "aiAgentsAPI/listAIAgentsAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["data"][0]["name"], "Support agent");
    assert!(
        meta.as_array()
            .unwrap()
            .contains(&json!([1, "data", 0, "createdAt"]))
    );
}

#[tokio::test]
async fn email_topic_procedures_and_actions_are_wired() {
    let (router, _) = router().await;
    let (status, topic, _) = post(
        &router,
        "actions/createEmailTopicAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "name": " News " }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{topic}");
    assert_eq!(topic["name"], "News");
    assert_eq!(topic["workspaceId"], ws());
    let id = topic["id"].as_str().unwrap().to_string();

    let (status, again, _) = post(
        &router,
        "actions/createEmailTopicAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "name": "News" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{again}");
    assert_eq!(again["message"], "Name is already taken.");

    let (status, updated, _) = post(
        &router,
        "actions/updateEmailTopicAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), id, { "name": "Updates" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Updates");

    let (status, list, meta) = post(
        &router,
        "emailTopicsAPI/privateListWorkspaceEmailTopicsAPI",
        "admintoken",
        json!({ "workspaceId": ws(), "name": "upd" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["pageCount"], 1);
    assert_eq!(list["data"][0]["id"], id);
    assert!(
        meta.as_array()
            .unwrap()
            .contains(&json!([1, "data", 0, "createdAt"]))
    );

    let (status, _, _) = post(
        &router,
        "actions/deleteEmailTopicAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "ids": [id] }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, list, _) = post(
        &router,
        "emailTopicsAPI/privateListWorkspaceEmailTopicsAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(list["data"], json!([]));
}

/// Port of `create-trigger-action`, `update-trigger-action`,
/// `update-trigger-settings-action` (Q.D5): the trigger actions are wired.
#[tokio::test]
async fn trigger_actions_are_wired() {
    let (router, _) = router().await;
    let (status, trigger, _) = post(
        &router,
        "actions/createTriggerAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "name": "My trigger", "folderId": null }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{trigger}");
    assert_eq!(trigger["name"], "My trigger");
    assert_eq!(trigger["actions"], json!([]));
    let id = trigger["id"].as_str().unwrap().to_string();

    let (status, updated, _) = post(
        &router,
        "actions/updateTriggerAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), id, {
            "actions": [],
            "conditions": [{ "type": "shoppingCartSent", "sourceId": "" }, { "type": "incomingCall" }],
        }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["trigger"]["id"], id);
    assert_eq!(updated["conditions"].as_array().map(Vec::len), Some(2));

    let (status, _, _) = post(
        &router,
        "actions/updateTriggerSettingsAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), id, { "active": false }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, missing, _) = post(
        &router,
        "actions/updateTriggerSettingsAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), "999", { "active": false }] }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["message"], "Trigger not found");

    let (status, _, _) = post(
        &router,
        "actions/deleteTriggersAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "ids": [id] }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// Port of `create-sequence.action`, `delete-sequence.action`,
/// `upsert-sequence-step.action`, `delete-sequence-step.action` and
/// `update-contact-sequence.action` (Q.D5): the sequence actions are wired.
#[tokio::test]
async fn sequence_actions_are_wired() {
    let (router, _) = router().await;
    let call = |path: &'static str, args: Value| {
        let router = router.clone();
        async move {
            let (status, body, _) = post(
                &router,
                path,
                "admintoken",
                json!({ "workspaceId": ws(), "args": args }),
            )
            .await;
            (status, body)
        }
    };
    let (status, created) = call(
        "actions/createSequenceAction",
        json!([ws(), { "name": "Welcome" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["sequenceId"].as_str().unwrap().to_string();
    // maps a validationException(name) to returnValidationErrors
    let (status, taken) = call(
        "actions/createSequenceAction",
        json!([ws(), { "name": "Welcome" }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{taken}");
    assert_eq!(
        taken["data"]["validationErrors"]["name"]["_errors"],
        json!(["Sequence name already exists"])
    );

    // upsert: create path, then the update path with stepId.
    let (status, step) = call(
        "actions/upsertSequenceStepAction",
        json!([ws(), { "sequenceId": id, "order": 0 }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{step}");
    let step_id = step["stepId"].as_str().unwrap().to_string();
    let (status, again) = call(
        "actions/upsertSequenceStepAction",
        json!([ws(), { "sequenceId": id, "stepId": step_id, "order": 0, "delayDays": 2 }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["stepId"], step_id);
    let (status, missing) = call(
        "actions/upsertSequenceStepAction",
        json!([ws(), { "sequenceId": "999", "order": 0 }]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["message"], "Sequence not found");

    // updateContactSequenceAction: the set replace answers the sequences.
    let (status, rows) = call(
        "actions/updateContactSequenceAction",
        json!([ws(), { "contactId": CONTACT_ID.to_string(), "sequences": [id] }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(
        rows,
        json!([{ "sequence": { "id": id, "name": "Welcome" } }])
    );

    let (status, gone) = call(
        "actions/deleteSequenceStepAction",
        json!([ws(), { "stepId": "998", "sequenceId": id }]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
    assert_eq!(gone["message"], "Step not found");
    let (status, deleted) = call(
        "actions/deleteSequenceStepAction",
        json!([ws(), { "stepId": step_id, "sequenceId": id }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted, json!({ "success": true }));

    let (status, _) = call("actions/deleteSequenceAction", json!([ws(), id])).await;
    assert_eq!(status, StatusCode::OK);
    let (status, missing) = call("actions/deleteSequenceAction", json!([ws(), id])).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["message"], "Sequence not found");
}
