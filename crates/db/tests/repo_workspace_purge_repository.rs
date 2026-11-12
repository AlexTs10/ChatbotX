//! Port of `packages/database/__tests__/workspace-purge-repository.test.ts` (P3.3). The
//! mocked `execute` returning a driver timestamp string becomes a real analytics row;
//! SQLite stores unix ms, so "decoding" is reading the stored instant back.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::workspace_purge::find_next_analytics_row_at;
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::{ms, ok, setup};

/// Port of: packages/database/__tests__/workspace-purge-repository.test.ts:5 "workspace purge repository > decodes an analytics timestamp returned by the postgres driver"
#[tokio::test]
async fn workspace_purge_repository_decodes_an_analytics_timestamp_returned_by_the_postgres_driver()
{
    let db = fresh_db().await;
    let s = seed(&db).await;
    let at = ms("2026-10-24T10:00:00.000Z");
    setup(&db, move |tx| {
        for (event, occurred) in [("e1", at + 60_000), ("e2", at)] {
            ins!(tx, "AnalyticsAgentResponseEvent", {
                "eventId" => event, "workspaceId" => s.workspace_id, "conversationId" => 1_i64,
                "contactId" => 1_i64, "adminId" => 1_i64, "occurredAt" => occurred,
                "awaitingSince" => occurred, "measuredFrom" => occurred, "responseSeconds" => 1_i64,
                "isFirstResponse" => false,
            })?;
        }
        Ok(())
    })
    .await;
    let ws = s.workspace_id;
    let occurred_at = ok(repo::read(&db, move |c| {
        find_next_analytics_row_at(c, "AnalyticsAgentResponseEvent", ws)
    })
    .await);
    assert_eq!(occurred_at, Some(at));
    assert_eq!(repo::iso(at), "2026-10-24T10:00:00.000Z");
}
