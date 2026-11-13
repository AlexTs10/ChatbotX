//! Port of `packages/business/__tests__/flow-version-after-publish-if-live.test.ts`
//! (P3.6-D2). Adapted: the mocked effects (welcome-message sync, quick-reply
//! cleanup) are not ported; `after_publish_if_live` returns whether they run.

mod common;

use chatbotx_business::flow_version;
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};

async fn flow_with_current(current: i64) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name", "currentVersionId") VALUES (1, ?1, 'Flow', ?2)"#,
            params![WS, current],
        )?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn runs_the_effects_while_the_version_is_live() {
    let db = flow_with_current(2).await;
    assert!(
        flow_version::after_publish_if_live(&db, WS, 1, 2)
            .await
            .expect("read")
    );
}

#[tokio::test]
async fn a_superseded_version_cancels_nothing_of_the_newer_one() {
    let db = flow_with_current(3).await;
    assert!(
        !flow_version::after_publish_if_live(&db, WS, 1, 2)
            .await
            .expect("read")
    );
}
