//! The workspace procedures and server actions under `/rpc` (Q.D11).
//!
//! Ported from `apps/builder/__tests__/workspace-settings-actions.test.ts`
//! and `update-workspace-advanced-schema.test.ts` (the TS tests mock the
//! service and call the action body; here the actions run through
//! `/rpc/workspaceActions/<name>` against a migrated in-memory database and
//! the stored row is checked). The list, gate and deletion tests are
//! Rust-native (no TS test).

mod rpc_common;

use axum::http::StatusCode;
use rpc_common::{AGENT_TOKEN, OUTSIDER_TOKEN, OWNER_TOKEN, WS, action, call, fixture, scalar};
use serde_json::{Value, json};

fn ws() -> String {
    WS.to_string()
}

fn advanced_input() -> Value {
    json!({
        "workspaceId": ws(),
        "defaultReply": "11",
        "targetCountry": "VN",
        "language": "vi",
        "timezone": "Asia/Ho_Chi_Minh",
        "brandColor": "#016DFF",
        "developmentMode": false,
    })
}

async fn with_flow(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (11, ?1, 'F')"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("flow");
}

async fn exec(fx: &rpc_common::Fixture, sql: &'static str) {
    fx.db()
        .write(move |tx| {
            tx.execute(sql, [WS])?;
            Ok(())
        })
        .await
        .expect("exec");
}

// --- workspace-settings-actions.test.ts ---

#[tokio::test]
async fn advanced_settings_go_through_update_settings() {
    // Adapted: the flow id is a numeric id of the workspace ("flow-1" in TS).
    let fx = fixture().await;
    with_flow(&fx).await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceAdvanced",
        OWNER_TOKEN,
        advanced_input(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out, json!({ "json": null, "meta": [[3]] }));
    for (sql, expected) in [
        (
            r#"SELECT "defaultReply" FROM "Workspace" WHERE "id" = ?1"#,
            json!("11"),
        ),
        (
            r#"SELECT "targetCountry" FROM "Workspace" WHERE "id" = ?1"#,
            json!("VN"),
        ),
        (
            r#"SELECT "language" FROM "Workspace" WHERE "id" = ?1"#,
            json!("vi"),
        ),
        (
            r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            json!("Asia/Ho_Chi_Minh"),
        ),
    ] {
        assert_eq!(scalar(&fx, sql, WS).await, expected, "{sql}");
    }
}

#[tokio::test]
async fn an_empty_default_reply_from_the_form_clears_it() {
    let fx = fixture().await;
    exec(
        &fx,
        r#"UPDATE "Workspace" SET "defaultReply" = '5' WHERE "id" = ?1"#,
    )
    .await;
    let mut input = advanced_input();
    input["defaultReply"] = json!("");
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceAdvanced",
        OWNER_TOKEN,
        input,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "defaultReply" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        Value::Null
    );
}

#[tokio::test]
async fn the_bot_reply_delay_goes_through_update_settings() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateSmartResponseDelay",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "smartResponseDelaySeconds": 30 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "smartResponseDelaySeconds" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        json!(30)
    );
}

// --- update-workspace-advanced-schema.test.ts ---

#[tokio::test]
async fn accepts_each_default_reply_frequency() {
    let fx = fixture().await;
    for frequency in ["allTime", "oncePerHour", "oncePerDay"] {
        let mut input = advanced_input();
        input["defaultReply"] = Value::Null;
        input["defaultReplyFrequency"] = json!(frequency);
        let (status, out) = action(
            &fx,
            "workspaceActions",
            "updateWorkspaceAdvanced",
            OWNER_TOKEN,
            input,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{frequency}: {out}");
        assert_eq!(
            scalar(
                &fx,
                r#"SELECT "defaultReplyFrequency" FROM "Workspace" WHERE "id" = ?1"#,
                WS
            )
            .await,
            json!(frequency)
        );
    }
}

#[tokio::test]
async fn rejects_an_invalid_frequency_string() {
    let fx = fixture().await;
    let mut input = advanced_input();
    input["defaultReply"] = Value::Null;
    input["defaultReplyFrequency"] = json!("everyMinute");
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceAdvanced",
        OWNER_TOKEN,
        input,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(out["json"]["message"], json!("Input validation failed"));
    assert!(out["json"]["data"]["validationErrors"]["defaultReplyFrequency"]["_errors"].is_array());
}

#[tokio::test]
async fn a_missing_frequency_keeps_the_stored_value() {
    let fx = fixture().await;
    exec(
        &fx,
        r#"UPDATE "Workspace" SET "defaultReplyFrequency" = 'oncePerDay' WHERE "id" = ?1"#,
    )
    .await;
    let mut input = advanced_input();
    input["defaultReply"] = Value::Null;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceAdvanced",
        OWNER_TOKEN,
        input,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "defaultReplyFrequency" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        json!("oncePerDay")
    );
}

// --- Rust-native: procedures ---

#[tokio::test]
async fn list_my_workspaces_returns_the_members_workspaces_with_date_meta() {
    let fx = fixture().await;
    let (status, out) = call(
        &fx,
        "GET",
        "/rpc/workspacesAPI/listMyWorkspacesAPI",
        Some(OWNER_TOKEN),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let list = out["json"]["workspaces"].as_array().expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], json!(ws()));
    assert_eq!(list[0]["name"], json!("Acme"));
    assert!(list[0].get("token").is_none());
    assert!(out["meta"].as_array().expect("meta").contains(&json!([
        1,
        "workspaces",
        0,
        "createdAt"
    ])));

    let (_, none) = call(
        &fx,
        "GET",
        "/rpc/workspacesAPI/listMyWorkspacesAPI",
        Some(OUTSIDER_TOKEN),
        json!({}),
    )
    .await;
    assert_eq!(none["json"]["workspaces"], json!([]));
}

#[tokio::test]
async fn list_deleted_workspaces_pages_and_validates() {
    let fx = fixture().await;
    let (status, out) = call(
        &fx,
        "GET",
        "/rpc/workspacesAPI/listDeletedWorkspacesAPI",
        Some(OWNER_TOKEN),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], json!({ "data": [], "pageCount": 0 }));
    let (status, _) = call(
        &fx,
        "GET",
        "/rpc/workspacesAPI/listDeletedWorkspacesAPI",
        Some(OWNER_TOKEN),
        json!({ "perPage": 101 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// --- Rust-native: action gates ---

#[tokio::test]
async fn an_action_needs_a_session_and_a_known_name() {
    let fx = fixture().await;
    let (status, _) = call(
        &fx,
        "POST",
        "/rpc/workspaceActions/updateWorkspaceBasic",
        None,
        json!({ "workspaceId": ws(), "name": "X" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, out) = action(&fx, "workspaceActions", "nope", OWNER_TOKEN, json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(out["json"]["message"], json!("Action not ported yet"));
}

#[tokio::test]
async fn a_non_member_gets_the_default_server_error() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceBasic",
        OUTSIDER_TOKEN,
        json!({ "workspaceId": ws(), "name": "X" }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        out["json"]["message"],
        json!("Something went wrong while executing the operation.")
    );
}

#[tokio::test]
async fn a_scheduled_workspace_refuses_workspace_actions_but_not_basic_edits() {
    let fx = fixture().await;
    exec(
        &fx,
        r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
    )
    .await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateSmartResponseDelay",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "smartResponseDelaySeconds": 30 }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(out["json"]["code"], json!("workspaceScheduledDeletion"));
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceBasic",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "name": "Renamed" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(&fx, r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#, WS).await,
        json!("Renamed")
    );
}

#[tokio::test]
async fn workspace_status_needs_both_times_and_a_super_admin() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceStatus",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "isActive": true, "startTime": "09:00", "endTime": null }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(out["json"]["data"]["validationErrors"]["endTime"]["_errors"].is_array());

    let input =
        json!({ "workspaceId": ws(), "isActive": false, "startTime": "09:00", "endTime": "17:30" });
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceStatus",
        AGENT_TOKEN,
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!("You need to be a super admin to change workspace active hours")
    );
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "updateWorkspaceStatus",
        OWNER_TOKEN,
        input,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "isActive" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        json!(0)
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "endTime" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        json!("17:30")
    );
}

#[tokio::test]
async fn schedule_and_cancel_deletion_need_a_super_admin_and_keep_the_log() {
    let fx = fixture().await;
    let input = json!({ "workspaceId": ws() });
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "scheduleWorkspaceDeletion",
        AGENT_TOKEN,
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!("You need to be a workspace super admin to do this.")
    );

    let (status, out) = action(
        &fx,
        "workspaceActions",
        "scheduleWorkspaceDeletion",
        OWNER_TOKEN,
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_ne!(
        scalar(
            &fx,
            r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        Value::Null
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "status" FROM "ResourceDeletionLog" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!("pending")
    );

    let (status, out) = action(
        &fx,
        "workspaceActions",
        "cancelWorkspaceDeletion",
        OWNER_TOKEN,
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        Value::Null
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "status" FROM "ResourceDeletionLog" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!("canceled")
    );

    exec(
        &fx,
        r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1, "purgeStartedAt" = 1 WHERE "id" = ?1"#,
    )
    .await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "cancelWorkspaceDeletion",
        OWNER_TOKEN,
        input,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!("Workspace deletion has already started.")
    );
}

// --- create-workspace-token-action.test.ts / delete-workspace-token-action.test.ts ---

fn token_input(all_scopes: bool, scopes: Value) -> Value {
    json!({
        "workspaceId": ws(),
        "name": "My token",
        "permission": "full",
        "allScopes": all_scopes,
        "scopes": scopes,
    })
}

async fn token_row(fx: &rpc_common::Fixture) -> (String, String, String, Value) {
    fx.db()
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "name", "tokenHash", "tokenPrefix", "scopes" FROM "WorkspaceApiToken" WHERE "workspaceId" = ?1"#,
                [WS],
                |r| {
                    let scopes: Option<String> = r.get(3)?;
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        scopes.map_or(Value::Null, |s| serde_json::from_str(&s).unwrap()),
                    ))
                },
            )?)
        })
        .await
        .expect("token row")
}

#[tokio::test]
async fn create_token_mints_a_cbx_ws_token_hashes_it_and_never_returns_the_hash() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "createWorkspaceToken",
        OWNER_TOKEN,
        token_input(true, json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let token = out["json"]["token"].as_str().expect("token").to_string();
    assert!(token.starts_with("cbx_ws_"), "{token}");
    let (name, hash, prefix, _) = token_row(&fx).await;
    assert_eq!(name, "My token");
    assert_ne!(hash, token);
    assert_eq!(prefix, token[..12]);
    assert!(!out.to_string().contains(&hash));
}

#[tokio::test]
async fn create_token_with_all_scopes_persists_null_scopes() {
    let fx = fixture().await;
    action(
        &fx,
        "workspaceActions",
        "createWorkspaceToken",
        OWNER_TOKEN,
        token_input(true, json!(["contacts"])),
    )
    .await;
    assert_eq!(token_row(&fx).await.3, Value::Null);
}

#[tokio::test]
async fn create_token_persists_the_explicit_scopes_array() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "createWorkspaceToken",
        OWNER_TOKEN,
        token_input(false, json!(["contacts", "inbox"])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(token_row(&fx).await.3, json!(["contacts", "inbox"]));
}

#[tokio::test]
async fn create_token_rejects_a_non_super_admin_member() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "createWorkspaceToken",
        AGENT_TOKEN,
        token_input(true, json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(
        out["json"]["message"]
            .as_str()
            .unwrap()
            .contains("super admin")
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "WorkspaceApiToken" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!(0)
    );
}

#[tokio::test]
async fn create_token_propagates_the_token_cap_error() {
    let fx = fixture().await;
    for _ in 0..10 {
        let (status, _) = action(
            &fx,
            "workspaceActions",
            "createWorkspaceToken",
            OWNER_TOKEN,
            token_input(true, json!([])),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "createWorkspaceToken",
        OWNER_TOKEN,
        token_input(true, json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(out["json"]["code"], json!("workspaceApiTokenLimitReached"));
    assert!(out["json"]["message"].as_str().unwrap().contains("maximum"));
}

#[tokio::test]
async fn create_token_needs_a_scope_unless_all_scopes() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "createWorkspaceToken",
        OWNER_TOKEN,
        token_input(false, json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["data"]["validationErrors"]["scopes"]["_errors"],
        json!(["Select at least one scope, or choose All scopes"])
    );
}

async fn one_token(fx: &rpc_common::Fixture) -> String {
    action(
        fx,
        "workspaceActions",
        "createWorkspaceToken",
        OWNER_TOKEN,
        token_input(true, json!([])),
    )
    .await;
    scalar(
        fx,
        r#"SELECT CAST("id" AS TEXT) FROM "WorkspaceApiToken" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .as_str()
    .unwrap()
    .to_string()
}

#[tokio::test]
async fn delete_token_deletes_the_token_scoped_to_the_workspace() {
    let fx = fixture().await;
    let id = one_token(&fx).await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "deleteWorkspaceToken",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "id": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "WorkspaceApiToken" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!(0)
    );
}

#[tokio::test]
async fn delete_token_returns_a_validation_error_when_no_row_was_deleted() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "deleteWorkspaceToken",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "id": "999" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["data"]["validationErrors"],
        json!({ "_errors": ["Token no longer exists"] })
    );
}

#[tokio::test]
async fn delete_token_rejects_a_non_super_admin_member() {
    let fx = fixture().await;
    let id = one_token(&fx).await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "deleteWorkspaceToken",
        AGENT_TOKEN,
        json!({ "workspaceId": ws(), "id": id }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(
        out["json"]["message"]
            .as_str()
            .unwrap()
            .contains("super admin")
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "WorkspaceApiToken" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!(1)
    );
}

#[tokio::test]
async fn token_create_and_delete_audit_without_the_hash_or_plaintext() {
    let fx = fixture().await;
    let (_, out) = action(
        &fx,
        "workspaceActions",
        "createWorkspaceToken",
        OWNER_TOKEN,
        token_input(false, json!(["contacts", "inbox"])),
    )
    .await;
    let token = out["json"]["token"].as_str().unwrap().to_string();
    let id = scalar(
        &fx,
        r#"SELECT CAST("id" AS TEXT) FROM "WorkspaceApiToken" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    action(
        &fx,
        "workspaceActions",
        "deleteWorkspaceToken",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "id": id }),
    )
    .await;
    let rows = fx
        .db()
        .read(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "action", "detail" FROM "AuditLog" ORDER BY "createdAt", "id""#,
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "create".to_string(),
                "created workspace API token \"My token\" (full, scopes: contacts,inbox)"
                    .to_string()
            ),
            (
                "delete".to_string(),
                format!("deleted workspace API token \"{}\"", id.as_str().unwrap())
            ),
        ]
    );
    assert!(!rows.iter().any(|(_, d)| d.contains(&token)));
}

// --- slice 4: `deleteErrorLogAction` ---

#[tokio::test]
async fn delete_error_log_removes_the_given_ids_of_the_workspace_only() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (202, 'Other', 103)"#,
                [],
            )?;
            for (id, ws) in [(1, WS), (2, WS), (3, 202)] {
                tx.execute(
                    r#"INSERT INTO "ErrorLog" ("id", "workspaceId", "action", "detail") VALUES (?1, ?2, 'smtp', 'x')"#,
                    chatbotx_db::rusqlite::params![id, ws],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    let (status, body) = action(
        &fx,
        "workspaceActions",
        "deleteErrorLog",
        OWNER_TOKEN,
        json!({ "workspaceId": WS.to_string(), "ids": ["1", "3"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "json": null, "meta": [[3]] }));
    let count = |id| {
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "ErrorLog" WHERE "id" = ?1"#,
            id,
        )
    };
    assert_eq!(count(1).await, json!(0));
    assert_eq!(count(2).await, json!(1));
    assert_eq!(count(3).await, json!(1));
}

#[tokio::test]
async fn delete_error_log_validates_the_ids() {
    let fx = fixture().await;
    let (status, body) = action(
        &fx,
        "workspaceActions",
        "deleteErrorLog",
        OWNER_TOKEN,
        json!({ "workspaceId": WS.to_string(), "ids": ["x"] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["json"]["data"]["validationErrors"]["ids"]["0"]["_errors"],
        json!(["Invalid string: must match pattern /^\\d+$/"])
    );
}

// --- mcp-oauth-revoke-action.test.ts (Q.A1) ---

async fn with_grant(fx: &rpc_common::Fixture, user: i64) {
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"INSERT OR IGNORE INTO "OAuthClient" ("clientId", "redirectUris", "name", "icon")
                   VALUES ('claude', '[]', 'Claude', 'http://insecure.example/icon.png')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "OAuthConsent" ("clientId", "userId", "scopes") VALUES ('claude', ?1, '["openid"]')"#,
                [user],
            )?;
            Ok(())
        })
        .await
        .expect("grant");
}

#[tokio::test]
async fn revoke_mcp_oauth_grant_revokes_only_the_current_users_grant() {
    let fx = fixture().await;
    with_grant(&fx, rpc_common::OWNER).await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "listMcpOAuthGrants",
        OWNER_TOKEN,
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"][0]["clientId"], "claude");
    assert_eq!(out["json"][0]["clientName"], "Claude");
    // A non-https icon is dropped.
    assert_eq!(out["json"][0]["clientIcon"], Value::Null);

    // Another member cannot revoke the owner's grant.
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "revokeMcpOAuthGrant",
        AGENT_TOKEN,
        json!({ "workspaceId": ws(), "clientId": "claude" }),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{out}");

    let (status, out) = action(
        &fx,
        "workspaceActions",
        "revokeMcpOAuthGrant",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "clientId": "claude" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT count(*) FROM "OAuthConsent" WHERE "userId" = ?1"#,
            rpc_common::OWNER
        )
        .await,
        json!(0)
    );
}

#[tokio::test]
async fn revoke_mcp_oauth_grant_reports_an_absent_grant() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "revokeMcpOAuthGrant",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "clientId": "missing" }),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{out}");
    assert!(
        out.to_string().contains("OAuth grant no longer exists"),
        "{out}"
    );
}

#[tokio::test]
async fn revoke_mcp_oauth_grant_rejects_an_empty_client_id() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        "workspaceActions",
        "revokeMcpOAuthGrant",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "clientId": "" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
}
