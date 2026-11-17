//! `apps/builder/__tests__/inbox-teams-public-api.test.ts` (Q.D11 slice 10):
//! the TS test checks that each `/v1/teams` route delegates to
//! `inboxTeamService`; here the routes run on a migrated database and the
//! responses and rows are checked.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const OWNER: i64 = 1001;
const AGENT: i64 = 1003;

async fn seeded() -> handlers_common::Fixture {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "name") VALUES (?1, 'agent@example.test', 'Agent')"#,
                [AGENT],
            )?;
            for (id, user) in [(1, OWNER), (2, AGENT)] {
                tx.execute(
                    r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (?1, ?2, ?3, 'agent', '{}')"#,
                    [id, WS, user],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    fx
}

fn ids(users: &[i64]) -> Value {
    users.iter().map(|u| json!(u.to_string())).collect()
}

async fn create(fx: &handlers_common::Fixture, name: &str, users: &[i64]) -> String {
    let (status, out) = call(
        fx,
        "inboxTeams.create",
        WS,
        &[],
        None,
        Some(json!({ "name": name, "userIds": ids(users) })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(out["name"], json!(name.trim()));
    assert_eq!(out["workspaceId"], json!(WS.to_string()));
    out["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn create_get_update_and_delete_a_team() {
    let fx = seeded().await;
    let id = create(&fx, " Support ", &[AGENT]).await;
    let (status, got) = call(&fx, "inboxTeams.get", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, got["name"].clone()), (200, json!("Support")));
    // Another workspace's token does not see it.
    let (status, _) = call(&fx, "inboxTeams.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);

    let (status, out) = call(
        &fx,
        "inboxTeams.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(
        (status, out["name"].clone()),
        (200, json!("Renamed")),
        "{out}"
    );

    let (status, out) = call(&fx, "inboxTeams.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, out), (204, Value::Null));
    let (status, out) = call(&fx, "inboxTeams.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404, "{out}");
    assert_eq!(out["message"], json!("Inbox team not found"));
}

#[tokio::test]
async fn add_and_remove_members_return_the_team() {
    let fx = seeded().await;
    let id = create(&fx, "Support", &[]).await;
    let team: i64 = id.parse().unwrap();
    const MEMBERS: &str = r#"SELECT COUNT(*) FROM "InboxTeamMember" WHERE "inboxTeamId" = ?1"#;
    let (status, out) = call(
        &fx,
        "inboxTeams.addMembers",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "userIds": ids(&[OWNER, AGENT]) })),
    )
    .await;
    assert_eq!((status, out["id"].clone()), (200, json!(id)), "{out}");
    assert_eq!(scalar(&fx, MEMBERS, team).await, Some(json!(2)));
    let (status, out) = call(
        &fx,
        "inboxTeams.removeMembers",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "userIds": ids(&[AGENT]) })),
    )
    .await;
    assert_eq!((status, out["id"].clone()), (200, json!(id)), "{out}");
    assert_eq!(scalar(&fx, MEMBERS, team).await, Some(json!(1)));
    // A user who is not a workspace member is refused.
    let (status, out) = call(
        &fx,
        "inboxTeams.addMembers",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "userIds": ids(&[1002]) })),
    )
    .await;
    assert_eq!(status, 400, "{out}");
    assert_eq!(out["code"], json!("invalidTeamMember"));
}

#[tokio::test]
async fn list_pages_in_memory_with_members() {
    let fx = seeded().await;
    let first = create(&fx, "A", &[AGENT]).await;
    create(&fx, "B", &[]).await;
    create(&fx, "C", &[]).await;
    let (status, out) = call(
        &fx,
        "inboxTeams.list",
        WS,
        &[],
        Some("page=1&perPage=2"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["pageCount"], json!(2));
    let data = out["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["id"], json!(first));
    assert_eq!(
        data[0]["inboxTeamMembers"][0]["user"]["name"],
        json!("Agent")
    );
    let (_, page2) = call(
        &fx,
        "inboxTeams.list",
        WS,
        &[],
        Some("page=2&perPage=2"),
        None,
    )
    .await;
    assert_eq!(page2["data"][0]["name"], json!("C"));
    let (_, other) = call(&fx, "inboxTeams.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [], "pageCount": 1 }));
}
