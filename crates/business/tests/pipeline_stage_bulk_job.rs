//! Port of `apps/worker/__tests__/pipeline-stage-bulk-handler.test.ts`
//! (Q.P6-U5). Adapted: the mocked `pipelineStageBulkService` becomes the
//! real `process_queued_stage_bulk` / `release_stage_bulk_claim` over a
//! migrated database ("forwards its claim id" = only the claim owner drains).

mod common;

use chatbotx_business::pipeline::mutation::PipelineActor;
use chatbotx_business::pipeline::structure::{process_queued_stage_bulk, release_stage_bulk_claim};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, contact, pipeline_row, setup, two_workspaces};

const PIPELINE: i64 = 501;
const STAGE: i64 = 511;
const TARGET: i64 = 512;
const CLAIM: &str = "pipeline-stage-bulk-511-1";

async fn seeded() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        pipeline_row(tx, PIPELINE, WS, "Sales", None)?;
        tx.execute(
            r#"INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position", "bulkJobId")
               VALUES (?1, ?2, ?3, 'Old', 0, ?4), (?5, ?2, ?3, 'New', 1, NULL)"#,
            params![STAGE, WS, PIPELINE, CLAIM, TARGET],
        )?;
        contact(tx, 81, WS)?;
        tx.execute(
            r#"INSERT INTO "PipelineItem" ("id", "workspaceId", "pipelineId", "stageId", "contactId", "number", "title", "status", "priority", "fieldValues", "stageEnteredAt")
               VALUES (521, ?1, ?2, ?3, 81, 1, 'Deal', 'open', 'medium', '{}', 0)"#,
            params![WS, PIPELINE, STAGE],
        )?;
        Ok(())
    })
    .await;
    db
}

fn actor() -> PipelineActor {
    PipelineActor {
        kind: "user".into(),
        user_id: None,
        source_id: None,
    }
}

/// `(stage ids in position order, the item's stage)`.
async fn state(db: &Db) -> (Vec<i64>, Option<i64>, Option<String>) {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "id" FROM "PipelineStage" WHERE "pipelineId" = ?1 ORDER BY "position""#,
        )?;
        let stages = stmt
            .query_map([PIPELINE], |r| r.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        let item = conn
            .query_row(
                r#"SELECT "stageId" FROM "PipelineItem" WHERE "id" = 521"#,
                [],
                |r| r.get(0),
            )
            .ok();
        let claim = conn
            .query_row(
                r#"SELECT "bulkJobId" FROM "PipelineStage" WHERE "id" = ?1"#,
                [STAGE],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        Ok((stages, item, claim))
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn the_job_drains_the_stage_it_owns_and_removes_it() {
    let db = seeded().await;
    process_queued_stage_bulk(
        &db,
        WS,
        PIPELINE,
        STAGE,
        CLAIM.into(),
        Some(TARGET),
        actor(),
    )
    .await
    .unwrap();
    assert_eq!(state(&db).await, (vec![TARGET], Some(TARGET), None));
}

#[tokio::test]
async fn a_job_that_lost_its_claim_changes_nothing() {
    let db = seeded().await;
    process_queued_stage_bulk(
        &db,
        WS,
        PIPELINE,
        STAGE,
        "other-claim".into(),
        Some(TARGET),
        actor(),
    )
    .await
    .unwrap();
    assert_eq!(
        state(&db).await,
        (vec![STAGE, TARGET], Some(STAGE), Some(CLAIM.into()))
    );
}

#[tokio::test]
async fn a_vanished_destination_gives_the_stage_back_instead_of_deleting_items() {
    let db = seeded().await;
    process_queued_stage_bulk(&db, WS, PIPELINE, STAGE, CLAIM.into(), Some(999), actor())
        .await
        .unwrap();
    assert_eq!(state(&db).await, (vec![STAGE, TARGET], Some(STAGE), None));
}

#[tokio::test]
async fn a_blocked_owner_releases_the_jobs_own_claim_instead_of_leaving_the_stage_busy() {
    let db = seeded().await;
    release_stage_bulk_claim(&db, WS, PIPELINE, STAGE, "newer-claim".into())
        .await
        .unwrap();
    assert_eq!(state(&db).await.2, Some(CLAIM.into()));
    release_stage_bulk_claim(&db, WS, PIPELINE, STAGE, CLAIM.into())
        .await
        .unwrap();
    assert_eq!(state(&db).await, (vec![STAGE, TARGET], Some(STAGE), None));
}
