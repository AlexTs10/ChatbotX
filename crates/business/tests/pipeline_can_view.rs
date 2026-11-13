//! Port of `apps/builder/__tests__/pipeline-can-view.test.ts`
//! (`canViewPipeline`). Adapted: the mocked session lookup is the `scope`
//! argument (`None` = not a member) and the mocked `pipelineService.get` is a
//! real pipeline row. "rethrows unexpected failures" is not-applicable (a
//! mocked database failure).

mod common;

use chatbotx_business::pipeline::PipelineAccessScope;
use chatbotx_business::pipeline::service::can_view_pipeline;
use common::{OTHER_WS, WS, pipeline_row, setup, two_workspaces};

const PLAIN_MEMBER: PipelineAccessScope = PipelineAccessScope::Member {
    workspace_member_id: 77,
    is_workspace_admin: false,
    can_view_contact_email_and_phone: true,
};

#[tokio::test]
async fn is_true_when_the_service_returns_the_pipeline() {
    let db = two_workspaces().await;
    setup(&db, |tx| pipeline_row(tx, 2, WS, "Sales", None)).await;
    assert!(
        can_view_pipeline(&db, WS, 2, Some(PipelineAccessScope::Support))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn is_false_when_the_viewer_is_not_a_member() {
    let db = two_workspaces().await;
    setup(&db, |tx| pipeline_row(tx, 2, WS, "Sales", None)).await;
    assert!(!can_view_pipeline(&db, WS, 2, None).await.unwrap());
}

#[tokio::test]
async fn is_false_for_a_missing_or_forbidden_pipeline_service_not_found() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        pipeline_row(tx, 2, WS, "Sales", None)?;
        pipeline_row(tx, 3, OTHER_WS, "Theirs", None)
    })
    .await;
    // Missing, another workspace's, and one the plain member has no grant on.
    assert!(
        !can_view_pipeline(&db, WS, 99, Some(PipelineAccessScope::Support))
            .await
            .unwrap()
    );
    assert!(
        !can_view_pipeline(&db, WS, 3, Some(PipelineAccessScope::Support))
            .await
            .unwrap()
    );
    assert!(
        !can_view_pipeline(&db, WS, 2, Some(PLAIN_MEMBER))
            .await
            .unwrap()
    );
}
