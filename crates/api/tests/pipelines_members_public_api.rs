//! Port of the `setMember` case of
//! `apps/builder/__tests__/pipelines-public-api.test.ts`. Adapted: the zod
//! schema is checked through `handlers::dispatch` over a real pipeline and
//! workspace member.

mod handlers_common;

use handlers_common::{WS, call, fixture};
use serde_json::json;

const MEMBER: i64 = 4001;

#[tokio::test]
async fn set_member_requires_an_explicit_view_grant_and_defaults_the_rest_to_none() {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (?1, ?2, 1001, 'agent')"#,
                [MEMBER, WS],
            )?;
            Ok(())
        })
        .await
        .expect("member");
    let (status, body) = call(
        &f,
        "pipelines.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "opportunity", "name": "Sales" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().expect("id").to_string();
    let member = MEMBER.to_string();
    let params = [("id", id.as_str()), ("workspaceMemberId", member.as_str())];
    let (status, body) = call(
        &f,
        "pipelines.setMember",
        WS,
        &params,
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["data"]["issues"][0]["path"],
        json!(["canView"]),
        "{body}"
    );
    let (status, body) = call(
        &f,
        "pipelines.setMember",
        WS,
        &params,
        None,
        Some(json!({ "canView": true })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["canEdit"], false);
    assert_eq!(body["canDelete"], false);
    assert_eq!(body["canManageSettings"], false);
    assert_eq!(body["canView"], true);
}
