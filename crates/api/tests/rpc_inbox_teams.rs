//! The inbox team actions and list procedure under `/rpc` (Q.D11 slice 10),
//! from `packages/business/__tests__/inbox-team-service-members.test.ts`:
//! the TS test mocks the database; here the actions run through
//! `/rpc/inboxTeamActions/<name>` on a migrated database (audit rows written
//! inline) and the stored rows are checked.

mod rpc_common;

use axum::http::StatusCode;
use rpc_common::{AGENT, OUTSIDER, OWNER, OWNER_TOKEN, WS, action, call, fixture, scalar};
use serde_json::{Value, json};

const KEY: &str = "inboxTeamActions";

fn ws() -> String {
    WS.to_string()
}

async fn audits(fx: &rpc_common::Fixture) -> Vec<(String, String)> {
    fx.db()
        .read(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "action", "detail" FROM "AuditLog" ORDER BY "createdAt", "id""#,
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

async fn members(fx: &rpc_common::Fixture, team: i64) -> Vec<i64> {
    fx.db()
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "userId" FROM "InboxTeamMember" WHERE "inboxTeamId" = ?1 ORDER BY "userId""#,
            )?;
            let rows = stmt
                .query_map([team], |r| r.get(0))?
                .collect::<Result<Vec<i64>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

/// Creates "Support" with the agent; its id.
async fn support(fx: &rpc_common::Fixture) -> i64 {
    let (status, out) = action(
        fx,
        KEY,
        "createInboxTeam",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "name": "  Support ", "userIds": [AGENT.to_string()] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    scalar(
        fx,
        r#"SELECT "id" FROM "InboxTeam" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .as_i64()
    .unwrap()
}

#[tokio::test]
async fn create_writes_the_team_and_members_and_audits() {
    let fx = fixture().await;
    let team = support(&fx).await;
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "name" FROM "InboxTeam" WHERE "id" = ?1"#,
            team
        )
        .await,
        json!("Support")
    );
    assert_eq!(members(&fx, team).await, [AGENT]);
    assert_eq!(
        audits(&fx).await,
        [("create".to_owned(), format!("created a new team (#{team})"))]
    );
}

/// "create rejects a non-member even when a duplicate row pads the count".
#[tokio::test]
async fn create_rejects_a_non_member_even_with_a_duplicate_member_id() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "createInboxTeam",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "name": "Support", "userIds": [AGENT.to_string(), AGENT.to_string(), OUTSIDER.to_string()] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!("One or more userIds are not members of this workspace")
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "InboxTeam" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!(0)
    );
    // An empty name is a validation error.
    let (status, _) = action(
        &fx,
        KEY,
        "createInboxTeam",
        OWNER_TOKEN,
        json!({ "workspaceId": ws(), "name": "  ", "userIds": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_audits_only_a_name_change_and_404s_an_unknown_team() {
    let fx = fixture().await;
    let team = support(&fx).await;
    let rename =
        |name: &str, id: i64| json!({ "workspaceId": ws(), "id": id.to_string(), "name": name });
    let (status, out) = action(
        &fx,
        KEY,
        "updateInboxTeam",
        OWNER_TOKEN,
        rename("Sales", team),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["name"], json!("Sales"));
    let (status, _) = action(
        &fx,
        KEY,
        "updateInboxTeam",
        OWNER_TOKEN,
        rename("Sales", team),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, out) = action(&fx, KEY, "updateInboxTeam", OWNER_TOKEN, rename("X", 1)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{out}");
    assert_eq!(out["json"]["message"], json!("Inbox team not found"));
    let details: Vec<String> = audits(&fx).await.into_iter().map(|(_, d)| d).collect();
    assert_eq!(
        details,
        [
            format!("created a new team (#{team})"),
            format!("updated a team (#{team})")
        ]
    );
}

#[tokio::test]
async fn add_and_remove_members_skip_existing_rows_and_audit_names() {
    let fx = fixture().await;
    let team = support(&fx).await;
    let users = |ids: &[i64]| -> Value { ids.iter().map(|id| json!(id.to_string())).collect() };
    let input =
        |ids: &[i64]| json!({ "workspaceId": ws(), "id": team.to_string(), "userIds": users(ids) });

    let (status, out) = action(
        &fx,
        KEY,
        "addInboxTeamMember",
        OWNER_TOKEN,
        input(&[AGENT, OWNER]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(members(&fx, team).await, [OWNER, AGENT]);
    // "addMembers rejects a non-member even when a duplicate row pads the count".
    let (status, _) = action(
        &fx,
        KEY,
        "addInboxTeamMember",
        OWNER_TOKEN,
        input(&[OWNER, OWNER, OUTSIDER]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Nothing new: no audit.
    action(&fx, KEY, "addInboxTeamMember", OWNER_TOKEN, input(&[AGENT])).await;

    let (status, _) = action(
        &fx,
        KEY,
        "deleteTeamMembers",
        OWNER_TOKEN,
        input(&[AGENT, OUTSIDER]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(members(&fx, team).await, [OWNER]);
    // Nothing removed: no audit.
    action(&fx, KEY, "deleteTeamMembers", OWNER_TOKEN, input(&[AGENT])).await;

    let details: Vec<String> = audits(&fx)
        .await
        .into_iter()
        .skip(1)
        .map(|(_, d)| d)
        .collect();
    assert_eq!(
        details,
        [
            format!("added Owner to the team (#{team})"),
            format!("removed Agent from the team (#{team})"),
        ]
    );
}

#[tokio::test]
async fn delete_removes_the_teams_and_404s_when_none_match() {
    let fx = fixture().await;
    let team = support(&fx).await;
    let input = |ids: Vec<String>| json!({ "workspaceId": ws(), "ids": ids });
    let (status, out) = action(
        &fx,
        KEY,
        "deleteInboxTeam",
        OWNER_TOKEN,
        input(vec!["1".into()]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{out}");
    let (status, out) = action(
        &fx,
        KEY,
        "deleteInboxTeam",
        OWNER_TOKEN,
        input(vec![team.to_string(), "1".into()]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "InboxTeam" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!(0)
    );
    assert!(members(&fx, team).await.is_empty());
    assert_eq!(
        audits(&fx).await.last().unwrap().1,
        format!("deleted team (#{team})")
    );
}

#[tokio::test]
async fn list_procedure_returns_teams_with_members_users_and_date_meta() {
    let fx = fixture().await;
    let team = support(&fx).await;
    let (status, out) = call(
        &fx,
        "GET",
        "/rpc/inboxTeamsAPI/listInboxTeamsAuthenticatedAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let data = out["json"]["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], json!(team.to_string()));
    assert_eq!(data[0]["name"], json!("Support"));
    let member = &data[0]["inboxTeamMembers"][0];
    assert_eq!(member["userId"], json!(AGENT.to_string()));
    assert_eq!(member["user"]["name"], json!("Agent"));
    assert_eq!(member["user"]["email"], json!("agent@rpc.test"));
    let meta = out["meta"].as_array().expect("meta");
    assert!(
        meta.contains(&json!([1, "data", 0, "createdAt"])),
        "{meta:?}"
    );
    assert!(meta.contains(&json!([
        1,
        "data",
        0,
        "inboxTeamMembers",
        0,
        "user",
        "updatedAt"
    ])));
}
