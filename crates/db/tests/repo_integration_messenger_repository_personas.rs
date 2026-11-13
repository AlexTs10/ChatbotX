//! Port of `packages/database/__tests__/integration-messenger-repository-personas.test.ts`
//! (P3.2b). The TS checks the mocked select / where / orderBy; here the query runs on a
//! migrated temp file (adapted): pages of two workspaces, created out of id order.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::Json;
use chatbotx_db::repo::integration_messenger::{self as im, MessengerPersonas};
use repo10::{OTHER_WS, WS, coexist_db, exec, read};
use serde_json::json;

#[tokio::test]
async fn integration_messenger_repository_list_personas_by_workspace_id_selects_only_name_and_personas_scoped_to_workspace_id_ordered_by_created_at()
 {
    let db = coexist_db().await;
    // Page 61 ("Page") exists; add 66 (older) in WS and 67 in OTHER_WS.
    db.write(|tx| {
        repo3::integration_messenger(tx, 66, WS, 3066, r#"[{"id":"p1","name":"Ada"}]"#)?;
        repo3::integration_messenger(tx, 67, OTHER_WS, 3067, "[]")?;
        Ok(())
    })
    .await
    .expect("pages");
    exec(
        &db,
        r#"UPDATE "IntegrationMessenger" SET "createdAt" = 1000, "name" = 'Support Page' WHERE "id" = 66;
           UPDATE "IntegrationMessenger" SET "createdAt" = 2000 WHERE "id" = 61"#,
    )
    .await;
    let rows = read(&db, |c| im::list_personas_by_workspace_id(c, WS)).await;
    assert_eq!(
        rows,
        vec![
            MessengerPersonas {
                name: "Support Page".to_owned(),
                personas: Json(json!([{ "id": "p1", "name": "Ada" }])),
            },
            MessengerPersonas {
                name: "Page".to_owned(),
                personas: Json(json!([])),
            },
        ]
    );
}
