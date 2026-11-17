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

#[tokio::test]
async fn media_library_folders_round_trip_through_the_actions() {
    let (router, _) = router().await;
    let act = |name: &'static str, input: Value| {
        let router = router.clone();
        async move {
            post(
                &router,
                &format!("actions/{name}"),
                "admintoken",
                json!({ "workspaceId": ws(), "args": [ws(), input] }),
            )
            .await
        }
    };
    let (status, folder, _) =
        act("createMediaLibraryFolderAction", json!({ "name": "Logos" })).await;
    assert_eq!(status, StatusCode::OK, "{folder}");
    let id = folder["id"].as_str().unwrap().to_string();
    let (status, _, _) = act(
        "renameMediaLibraryFolderAction",
        json!({ "folderId": id, "name": "Brand" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, json, _) = post(
        &router,
        "mediaLibraryAPI/listMediaLibraryFolders",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data"][0]["name"], "Brand");
    assert_eq!(json["data"][0]["fileCount"], 0);

    let (status, json, _) = post(
        &router,
        "mediaLibraryAPI/listMediaLibraryFiles",
        "admintoken",
        json!({ "workspaceId": ws(), "folderId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data"], json!([]));

    // A bare id input (`zodBigintAsString()`).
    let (status, _, _) = act("deleteMediaLibraryFolderAction", json!(id)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn flow_step_option_lists_answer_for_a_member() {
    let (router, _) = router().await;
    for (path, data) in [
        ("pipelinesAPI/privateListPipelineOptionsAPI", "/data"),
        ("refLinksAPI/listRefLinkOptionsAuthenticatedAPI", "/data"),
        (
            "personasAPIs/listMessengerPersonasAuthenticatedAPI",
            "/data",
        ),
        (
            "userPersistentMenusAPI/listUserPersistentMenusAuthenticatedAPI",
            "/data",
        ),
        ("placesNearMeAPI/listPlaceCategoryOptionsAPI", "/data"),
        ("questionnairesAPI/listQuestionnairesForFlowAPI", ""),
    ] {
        let (status, json, _) =
            post(&router, path, "admintoken", json!({ "workspaceId": ws() })).await;
        assert_eq!(status, StatusCode::OK, "{path}: {json}");
        assert!(json.pointer(data).unwrap().is_array(), "{path}: {json}");
    }
    let (status, json, _) = post(
        &router,
        "pipelinesAPI/privateGetPipelineAPI",
        "assignedtoken",
        json!({ "workspaceId": ws(), "pipelineId": "999" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
}

#[tokio::test]
async fn settings_integration_queries_hide_secrets_and_disconnect() {
    let (router, db) = router().await;
    let query = |name: &str, args: Value| {
        let router = router.clone();
        let name = name.to_string();
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
    let (status, has, _) = query("hasAIIntegration", json!([ws()])).await;
    assert_eq!((status, has), (StatusCode::OK, json!(false)));
    let (_, none, _) = query("findIntegrationClaude", json!([{ "workspaceId": ws() }])).await;
    assert_eq!(none, Value::Null);
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (601, ?1, 'claude')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationClaude" ("id", "workspaceId", "integrationId", "auth", "model", "maxOutputTokens")
               VALUES (602, ?1, 601, '{"authType":"secretText","secretText":"sk-secret"}', 'claude-sonnet-4-6', 1000)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status")
               VALUES (603, 'bot', 'telegram', '42', ?1, 'connected')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationTelegram" ("id", "auth", "botId", "name", "workspaceId", "inboxId")
               VALUES (604, '{"authType":"secretText","secretText":"42:bot-token"}', '42', 'bot', ?1, 603)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await
    .expect("seed integrations");
    let (_, has, _) = query("hasAIIntegration", json!([ws()])).await;
    assert_eq!(has, json!(true));
    let (_, claude, _) = query("findIntegrationClaude", json!([{ "workspaceId": ws() }])).await;
    assert_eq!(claude["id"], "602");
    assert_eq!(claude["auth"], json!({ "authType": "secretText" }));
    let (_, telegrams, _) = query(
        "listIntegrationTelegrams",
        json!([{ "where": { "workspaceId": ws() } }]),
    )
    .await;
    assert_eq!(telegrams["data"][0]["botId"], "42");
    assert_eq!(
        telegrams["data"][0]["auth"],
        json!({ "authType": "secretText" })
    );
    // No `Connection` row: the legacy disconnect deletes the integration.
    let (status, _, _) = query("disconnectClaudeAction", json!([ws()])).await;
    assert_eq!(status, StatusCode::OK);
    let (_, none, _) = query("findIntegrationClaude", json!([{ "workspaceId": ws() }])).await;
    assert_eq!(none, Value::Null);
}

/// `actions/<name>` as the admin in [`WS`].
async fn act(router: &axum::Router, token: &str, name: &str, args: Value) -> (StatusCode, Value) {
    let (status, json, _) = post(
        router,
        &format!("actions/{name}"),
        token,
        json!({ "workspaceId": ws(), "args": args }),
    )
    .await;
    (status, json)
}

#[tokio::test]
async fn api_channel_update_rotate_and_delete() {
    let (router, db) = router().await;
    let (status, created) = act(
        &router,
        "admintoken",
        "createApiAction",
        json!([{ "name": "Bot", "workspaceId": ws() }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let row = |db: Arc<Db>| async move {
        db.read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "id", "name", "tokenHash" FROM "IntegrationApi" WHERE "workspaceId" = ?1"#,
                params![WS],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )?)
        })
        .await
    };
    let (id, _, hash) = row(Arc::clone(&db)).await.unwrap();
    let id_arg = id.to_string();

    let (status, out) = act(
        &router,
        "admintoken",
        "updateApiAction",
        json!([ws(), id_arg, { "name": "Renamed" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(row(Arc::clone(&db)).await.unwrap().1, "Renamed");

    let (status, out) = act(
        &router,
        "admintoken",
        "rotateApiTokenAction",
        json!([ws(), id_arg]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(out["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_ne!(row(Arc::clone(&db)).await.unwrap().2, hash);

    let (status, out) = act(
        &router,
        "admintoken",
        "deleteApiAction",
        json!([ws(), id_arg]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(row(Arc::clone(&db)).await.is_err());
    let (status, _) = act(
        &router,
        "admintoken",
        "rotateApiTokenAction",
        json!([ws(), id_arg]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ecommerce_pages_need_the_permission_and_list_categories() {
    let (router, _) = router().await;
    let (status, out) = act(
        &router,
        "admintoken",
        "createProductCategoryAction",
        json!([ws(), { "name": "Drinks" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, page) = act(
        &router,
        "admintoken",
        "productCategoriesPage",
        json!([ws(), {}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"][0]["name"], "Drinks");
    assert_eq!(page["parent"], Value::Null);
    let (status, page) = act(&router, "admintoken", "productsPage", json!([ws(), {}])).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["categories"][0]["name"], "Drinks");
    // An agent without `ecommerce` gets the layout's 404.
    let (status, _) = act(
        &router,
        "assignedtoken",
        "productCategoriesPage",
        json!([ws(), {}]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pipelines_and_questionnaires_round_trip() {
    let (router, _) = router().await;
    let (status, out) = act(
        &router,
        "admintoken",
        "createPipelineAction",
        json!([ws(), { "type": "opportunity", "name": "Sales" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, list, _) = post(
        &router,
        "pipelinesAPI/privateListPipelinesAPI",
        "admintoken",
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list[0]["pipeline"]["name"], "Sales");
    assert_eq!(list[0]["itemsCount"], 0);

    let (status, out) = act(
        &router,
        "admintoken",
        "createQuestionnaireAction",
        json!([ws(), { "name": " Survey " }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let id = out["id"].as_str().unwrap().to_string();
    let (status, out) = act(
        &router,
        "admintoken",
        "renameQuestionnaireAction",
        json!([ws(), id, { "name": "Poll" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, list) = act(
        &router,
        "admintoken",
        "listQuestionnaires",
        json!([{ "page": 1, "perPage": 10, "sort": [{ "id": "name", "desc": false }] }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["data"][0]["name"], "Poll");
    let (status, out) = act(
        &router,
        "admintoken",
        "deleteQuestionnairesAction",
        json!([ws(), { "ids": [id] }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (_, list) = act(&router, "admintoken", "listQuestionnaires", json!([{}])).await;
    assert_eq!(list["data"], json!([]));
}

#[tokio::test]
async fn dashboard_analytics_procedures_run_the_public_routes() {
    let (router, _) = router().await;
    let range = json!({
        "workspaceId": ws(),
        "from": "2027-01-01T00:00:00.000Z",
        "to": "2027-01-31T23:59:59.999Z",
        "timezone": "UTC",
    });
    let (status, json, _) = post(
        &router,
        "analyticsRoutes/contactsCountAnalyticsAPI",
        "admintoken",
        range.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // `InboxContactStat` totals: the seed has no inbox.
    assert_eq!(json["data"]["count"], 0);
    let (status, json, _) = post(
        &router,
        "analyticsRoutes/contactCountsPerDayAnalyticsAPI",
        "assignedtoken",
        range.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["data"].as_array().unwrap().len() >= 28);
    let mut by_dimension = range.clone();
    by_dimension["dimension"] = json!("channel");
    by_dimension["inboxIds"] = json!(["9"]);
    let (status, json, _) = post(
        &router,
        "analyticsRoutes/contactsByDimensionAnalyticsAPI",
        "admintoken",
        by_dimension,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data"], json!([]));
    let mut bad = range;
    bad["granularity"] = json!("week");
    let (status, _, _) = post(
        &router,
        "analyticsRoutes/botMessagesByResultAnalyticsAPI",
        "admintoken",
        bad,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn spa_stub_path_runs_the_workspace_actions_with_their_gates() {
    let (router, db) = router().await;
    let (status, out) = act(
        &router,
        "admintoken",
        "updateWorkspaceBasicAction",
        json!([ws(), { "name": "Renamed" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let name: String = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#,
                [WS],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(name, "Renamed");
    // An agent is no workspace super admin.
    let (status, out) = act(
        &router,
        "assignedtoken",
        "scheduleWorkspaceDeletionAction",
        json!([ws()]),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{out}");
    // A bad input is the next-safe-action validation error.
    let (status, out) = act(
        &router,
        "admintoken",
        "updateWorkspaceBasicAction",
        json!([ws(), { "name": "" }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(out["data"]["validationErrors"]["name"].is_object(), "{out}");
}

#[tokio::test]
async fn template_locked_ids_answer_the_lock_badges() {
    let (router, db) = router().await;
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "Template" ("id", "workspaceId", "tenantId", "name", "selection", "payload",
                 "categoryCounts", "formatVersion", "shareToken", "defaultPermissions")
               VALUES (601, ?1, (SELECT "tenantId" FROM "Workspace" WHERE "id" = ?1), 'T', '{}', '{}',
                 '{}', 1, 'tok', '{}')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "TemplateInstallation" ("id", "workspaceId", "templateId", "templateName",
                 "sourceWorkspaceId", "formatVersion", "status", "permissions", "sourceTemplateId")
               VALUES (602, ?1, 601, 'T', ?1, 1, 'completed', '{}', 601)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "TemplateInstalledResource" ("id", "installationId", "workspaceId", "category",
                 "resourceKind", "resourceId", "sourceResourceId", "allowEdit", "allowDelete")
               VALUES (603, 602, ?1, 'flows', 'flow', 77, 's', 0, 1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await
    .expect("seed template");
    let (status, json, _) = post(
        &router,
        "templatesAPI/listLockedIdsAPI",
        "assignedtoken",
        json!({ "workspaceId": ws(), "resourceKind": "flow", "resourceIds": ["77", "78"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json, json!({ "lockedIds": ["77"], "undeletableIds": [] }));
    let (status, _, _) = post(
        &router,
        "templatesAPI/listLockedIdsAPI",
        "assignedtoken",
        json!({ "workspaceId": ws(), "resourceKind": "nope", "resourceIds": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn questionnaire_editor_loads_through_the_public_route() {
    let (router, _) = router().await;
    let (status, out) = act(
        &router,
        "admintoken",
        "createQuestionnaireAction",
        json!([ws(), { "name": "Survey" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let id = out["id"].as_str().unwrap().to_string();
    let (status, found) = act(
        &router,
        "assignedtoken",
        "getQuestionnaire",
        json!([{ "workspaceId": ws(), "id": id }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["name"], "Survey");
    assert_eq!(found["questions"], json!([]));
    let (status, out) = act(
        &router,
        "admintoken",
        "updateQuestionnaireAction",
        json!([ws(), id, { "enableScore": "yes" }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(out["data"]["validationErrors"].is_object(), "{out}");
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

async fn keyword_rule(db: &Db, id: i64, kind: &'static str) {
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "keywords", "status", "text", "type")
               VALUES (?1, ?2, '["x"]', 0, 'old', ?3)"#,
            params![id, WS, kind],
        )?;
        Ok(())
    })
    .await
    .unwrap();
}

async fn keyword_text(db: &Db, sql: &'static str, id: i64) -> Option<String> {
    db.read(move |conn| {
        Ok(conn
            .query_row(sql, [id], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn create_automated_response_delegates_with_the_resolved_workspace_and_type() {
    let (router, db) = router().await;
    let (status, out, _) = post(
        &router,
        "actions/createAutomatedResponseAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), "outbound", {
            "text": "hello", "flowId": null, "folderId": null, "keywords": [{ "value": "hi" }],
        }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let row = keyword_text(
        &db,
        r#"SELECT json_array("workspaceId", "type", "text", "keywords") FROM "AutomatedResponse" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert_eq!(
        row.as_deref(),
        Some(r#"[201,"outbound","hello","[\"hi\"]"]"#)
    );
}

#[tokio::test]
async fn create_automated_response_maps_a_field_scoped_validation_exception_to_a_form_field_error()
{
    let (router, db) = router().await;
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (901, ?1, 'f')"#,
            [WS],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let (status, out, _) = post(
        &router,
        "actions/createAutomatedResponseAction",
        "admintoken",
        json!({ "workspaceId": ws(), "args": [ws(), "inbound", {
            "text": "hello", "flowId": "901", "keywords": [{ "value": "hi" }],
        }] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["data"]["validationErrors"],
        json!({
            "_errors": ["Validation Exception"],
            "flowId": { "_errors": ["A keyword replies with either text or a flow, not both"] },
        })
    );
}

#[tokio::test]
async fn update_automated_response_passes_the_callers_type_through_to_the_service() {
    let (router, db) = router().await;
    keyword_rule(&db, 911, "outbound").await;
    let update = |kind: &str| {
        json!({ "workspaceId": ws(), "args": [ws(), "911", kind, {
            "text": "hi", "flowId": null, "folderId": null, "keywords": [{ "value": "x" }],
        }] })
    };
    let (status, _, _) = post(
        &router,
        "actions/updateAutomatedResponseAction",
        "admintoken",
        update("inbound"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, out, _) = post(
        &router,
        "actions/updateAutomatedResponseAction",
        "admintoken",
        update("outbound"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        keyword_text(
            &db,
            r#"SELECT "text" FROM "AutomatedResponse" WHERE "id" = ?1"#,
            911
        )
        .await
        .as_deref(),
        Some("hi")
    );
}

#[tokio::test]
async fn enable_automated_response_scopes_both_find_or_fail_and_set_status_by_the_callers_type() {
    let (router, db) = router().await;
    keyword_rule(&db, 912, "outbound").await;
    let enable = |kind: &str| json!({ "workspaceId": ws(), "args": [ws(), "912", kind, { "status": true }] });
    let (status, _, _) = post(
        &router,
        "actions/enableAutomatedResponseAction",
        "admintoken",
        enable("inbound"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, out, _) = post(
        &router,
        "actions/enableAutomatedResponseAction",
        "admintoken",
        enable("outbound"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        keyword_text(
            &db,
            r#"SELECT CAST("status" AS TEXT) FROM "AutomatedResponse" WHERE "id" = ?1"#,
            912
        )
        .await
        .as_deref(),
        Some("1")
    );
}

#[tokio::test]
async fn delete_automated_response_action_scopes_the_bulk_delete_by_the_callers_type() {
    let (router, db) = router().await;
    keyword_rule(&db, 913, "outbound").await;
    keyword_rule(&db, 914, "outbound").await;
    let delete = |kind: &str| json!({ "workspaceId": ws(), "args": [ws(), kind, { "ids": ["913", "914"] }] });
    let count = || {
        keyword_text(
            &db,
            r#"SELECT CAST(count(*) AS TEXT) FROM "AutomatedResponse" WHERE "workspaceId" = ?1"#,
            WS,
        )
    };
    let (status, _, _) = post(
        &router,
        "actions/deleteAutomatedResponseAction",
        "admintoken",
        delete("inbound"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count().await.as_deref(), Some("2"));
    let (status, _, _) = post(
        &router,
        "actions/deleteAutomatedResponseAction",
        "admintoken",
        delete("outbound"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count().await.as_deref(), Some("0"));
}
