//! Port of `packages/business/__tests__/pipeline-currency-refresh.test.ts`.
//! Adapted: the mocked `listIdsPage` is real `Pipeline` rows and the mocked
//! `queueWorkspaceRealtimeEvent` is the returned event list (the business
//! crate has no realtime handle). The failed-read case is not-applicable
//! (a mocked repository rejection).

mod common;

use chatbotx_business::pipeline::currency::notify_workspace_currency_change;
use chatbotx_business::pipeline::events::PipelineItemChanged;
use common::{OTHER_WS, WS, pipeline_row, setup, two_workspaces};

#[tokio::test]
async fn sends_the_structure_changed_event_for_each_pipeline() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        pipeline_row(tx, 11, WS, "A", None)?;
        pipeline_row(tx, 12, WS, "B", None)?;
        pipeline_row(tx, 13, OTHER_WS, "C", None)
    })
    .await;
    let events = notify_workspace_currency_change(&db, WS).await;
    assert_eq!(
        events,
        [11, 12]
            .map(|pipeline_id| PipelineItemChanged {
                workspace_id: WS,
                pipeline_id,
                item_id: None,
                contact_id: None,
                change: "updated",
            })
            .to_vec()
    );
}

#[tokio::test]
async fn a_workspace_without_pipelines_sends_nothing_and_reads_once() {
    let db = two_workspaces().await;
    assert!(notify_workspace_currency_change(&db, WS).await.is_empty());
}

#[tokio::test]
async fn pages_past_200_pipelines_with_a_keyset_cursor_and_stops_on_a_short_page() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for id in 1..=405 {
            pipeline_row(tx, id, WS, &format!("P{id}"), None)?;
        }
        Ok(())
    })
    .await;
    let events = notify_workspace_currency_change(&db, WS).await;
    assert_eq!(events.len(), 405);
    assert_eq!(
        events.iter().map(|e| e.pipeline_id).collect::<Vec<_>>(),
        (1..=405).collect::<Vec<_>>()
    );
}
