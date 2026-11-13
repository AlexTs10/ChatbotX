//! Port of `packages/business/__tests__/pipeline-update-lock.test.ts`.
//! The lock-order case is not-applicable: the template locks
//! (`withTemplateLockedTransaction`, `assertResourceEditable`) are not ported
//! (DECISIONS 2026-11-13) and one SQLite writer serialises the update.
//! Adapted: a pipeline that is gone is a real missing row, not a mock.

mod common;

use chatbotx_business::pipeline::PipelineAccessScope;
use chatbotx_business::pipeline::service::{UpdatePipeline, update};
use common::{WS, pipeline_row, scalar_text, setup, two_workspaces};

#[tokio::test]
async fn a_pipeline_deleted_before_the_lock_is_not_found_and_nothing_is_written() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        pipeline_row(tx, 7, WS, "Old", None)?;
        tx.execute(r#"DELETE FROM "Pipeline" WHERE "id" = 7"#, [])?;
        pipeline_row(tx, 8, WS, "Other", None)
    })
    .await;
    let error = update(
        &db,
        UpdatePipeline {
            workspace_id: WS,
            id: 7,
            scope: PipelineAccessScope::WorkspaceToken,
            name: Some("New".to_string()),
            reference_name: None,
            currency: None,
            settings: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Pipeline not found");
    assert_eq!(
        scalar_text(&db, r#"SELECT "name" FROM "Pipeline" WHERE "id" = ?1"#, 8)
            .await
            .as_deref(),
        Some("Other")
    );
}
