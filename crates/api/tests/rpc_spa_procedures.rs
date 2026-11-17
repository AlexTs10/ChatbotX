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
async fn inbox_screen_lists_and_acts_on_conversations() {
    let (router, db) = router().await;
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (601, 'Site', 'webchat', 's', ?1)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (602, ?1, 'Support')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "InboxTeamMember" ("id", "inboxTeamId", "userId") VALUES (603, 602, ?1)"#,
            params![ADMIN_ID],
        )?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "lastActivityAt") VALUES (604, ?1, ?2, ?3)"#,
            params![WS, CONTACT_ID, NOW_MS],
        )?;
        Ok(())
    })
    .await
    .expect("seed inbox");

    let (status, inboxes, _) = post(
        &router,
        "inboxesAPI/listAllInboxesAuthenticatedAPI",
        "admintoken",
        json!({ "workspaceId": ws(), "includes": ["integration"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inboxes}");
    assert_eq!(inboxes["data"][0]["id"], "601");
    assert_eq!(inboxes["data"][0]["integrationWebchat"], Value::Null);

    let (status, teams, _) = post(
        &router,
        "inboxTeamsAPI/listInboxTeamsAuthenticatedAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{teams}");
    assert_eq!(
        teams["data"][0]["inboxTeamMembers"][0]["user"]["id"],
        ADMIN_ID.to_string()
    );

    // The builder list has no assigned-only filter: the agent sees it too.
    let list = |token: &'static str| {
        let router = router.clone();
        async move {
            post(
                &router,
                "conversationsAPI/listConversationsByPOSTAuthenticatedAPI",
                token,
                json!({ "workspaceId": ws() }),
            )
            .await
        }
    };
    let (status, page, _) = list("assignedtoken").await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"][0]["id"], "604");

    let (status, _, _) = post(
        &router,
        "actions/assignConversationAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "contactIds": [CONTACT_ID.to_string()], "assignedId": "t_602" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, page, _) = list("admintoken").await;
    assert_eq!(page["data"][0]["assignedInboxTeamId"], "602");

    let (status, read, _) = post(
        &router,
        "actions/readConversationAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), "604"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(read["agentLastReadAt"].is_string(), "{read}");

    let (status, _, _) = post(
        &router,
        "actions/archiveConversationAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), { "ids": ["604"] }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, page, _) = list("admintoken").await;
    assert_eq!(page["data"], json!([]));
}

#[tokio::test]
async fn contact_notes_keep_the_author_and_the_member_scope() {
    let (router, _) = router().await;
    let note = |token: &'static str, name: &'static str, input: Value| {
        let router = router.clone();
        async move {
            post(
                &router,
                &format!("actions/{name}"),
                token,
                json!({ "workspaceId": ws(), "args": [ws(), CONTACT_ID.to_string(), input] }),
            )
            .await
        }
    };
    let (status, created, _) = note(
        "admintoken",
        "createContactNoteAction",
        json!({ "text": " hi " }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["text"], "hi");
    assert_eq!(created["createdById"], ADMIN_ID.to_string());

    // An assigned-only agent without the conversation: 404, no write.
    let (status, error, _) = note(
        "assignedtoken",
        "createContactNoteAction",
        json!({ "text": "x" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["message"], "Contact not found");

    let id = created["id"].as_str().unwrap().to_string();
    let (status, edited, _) = note(
        "admintoken",
        "editContactNoteAction",
        json!({ "contactNoteId": id, "text": "edited" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(edited["text"], "edited");
    for _ in 0..2 {
        // The second delete is a no-op, not an error.
        let (status, _, _) = note(
            "admintoken",
            "deleteContactNoteAction",
            json!({ "contactNoteId": id }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}

#[tokio::test]
async fn tag_actions_create_validate_and_delete() {
    let (router, _) = router().await;
    let act = |name: &'static str, args: Value| {
        let router = router.clone();
        async move {
            post(
                &router,
                &format!("actions/{name}"),
                "admintoken",
                json!({ "workspaceId": ws(), "args": args }),
            )
            .await
        }
    };
    let (status, tag, _) = act("createTagAction", json!([ws(), { "name": "promo" }])).await;
    assert_eq!(status, StatusCode::OK, "{tag}");
    assert_eq!(tag["name"], "promo");

    // A duplicate name is a form error on `name` (next-safe-action shape).
    let (status, error, _) = act("createTagAction", json!([ws(), { "name": "promo" }])).await;
    assert_ne!(status, StatusCode::OK);
    assert!(
        error["data"]["validationErrors"]["name"]["_errors"][0].is_string(),
        "{error}"
    );

    let id = tag["id"].as_str().unwrap().to_string();
    let (status, _, _) = act("deleteTagAction", json!([ws(), { "ids": [id.clone()] }])).await;
    assert_eq!(status, StatusCode::OK);
    let (_, json, _) = post(
        &router,
        "tagsAPI/privateListWorkspaceTagsAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert!(
        json["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["id"] != id),
        "{json}"
    );
}
