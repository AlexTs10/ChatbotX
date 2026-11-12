//! Shared fixtures of the P3.3 pipeline slice (on top of `common/mod.rs` and
//! `common/repo.rs`; `repo3.rs` .. `repo6.rs` are locked).
//!
//! Include after `repo_common`:
//! ```ignore
//! #[path = "common/repo7.rs"]
//! mod repo7;
//! ```

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use chatbotx_db::repo::models::PipelineItem;
use chatbotx_db::repo::pipeline::next_item_number;
use chatbotx_db::repo::pipeline_item::{PipelineItemInsert, insert_idempotent};
use chatbotx_db::repo::{self, Json, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};

use crate::common::{TestDb, fresh_db};

/// Money x 10,000 (`value: 10` in the TS is `money(10)` here).
pub const fn money(major: i64) -> i64 {
    major * 10_000
}

/// One workspace with a user, two contacts, a pipeline `Sales` / `sales` (USD) and the
/// stages `Lead` (0) and `Won` (1) (the TS `createFixture`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipeFx {
    pub user_id: i64,
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub stage_ids: [i64; 2],
    pub contact_ids: [i64; 2],
}

impl PipeFx {
    /// The ids of fixture `n` (1, 2, ...).
    pub const fn ids(n: i64) -> PipeFx {
        PipeFx {
            user_id: 1000 + n,
            workspace_id: 2000 + n,
            pipeline_id: 5000 + n,
            stage_ids: [6000 + 10 * n + 1, 6000 + 10 * n + 2],
            contact_ids: [4000 + 10 * n + 1, 4000 + 10 * n + 2],
        }
    }
}

/// Inserts one `User` (`pipeline-<id>@example.test`).
pub fn put_user(tx: &Connection, id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
        params![id, format!("pipeline-{id}@example.test")],
    )?;
    Ok(())
}

/// Inserts one `Pipeline` (type `opportunity`, settings `{}`).
pub fn put_pipeline(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    reference_name: &str,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "Pipeline" ("id", "workspaceId", "name", "type", "referenceName", "currency", "settings")
           VALUES (?1, ?2, ?3, 'opportunity', ?3, 'USD', '{}')"#,
        params![id, workspace_id, reference_name],
    )?;
    Ok(())
}

/// Inserts one `PipelineStage`.
pub fn put_stage(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    pipeline_id: i64,
    name: &str,
    position: i64,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position")
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, workspace_id, pipeline_id, name, position],
    )?;
    Ok(())
}

/// Inserts the rows of fixture `fx`.
pub fn put_fixture(tx: &Connection, fx: PipeFx) -> Result<(), DbError> {
    put_user(tx, fx.user_id)?;
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?3)"#,
        params![
            fx.workspace_id,
            format!("pipeline-{}", fx.workspace_id),
            fx.user_id
        ],
    )?;
    for contact in fx.contact_ids {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
            params![contact, fx.workspace_id],
        )?;
    }
    put_pipeline(tx, fx.pipeline_id, fx.workspace_id, "sales")?;
    put_stage(
        tx,
        fx.stage_ids[0],
        fx.workspace_id,
        fx.pipeline_id,
        "Lead",
        0,
    )?;
    put_stage(
        tx,
        fx.stage_ids[1],
        fx.workspace_id,
        fx.pipeline_id,
        "Won",
        1,
    )?;
    Ok(())
}

/// A fresh file with fixtures 1 (`own`) and 2 (`other`).
pub async fn pipeline_db() -> (TestDb, PipeFx, PipeFx) {
    let db = fresh_db().await;
    let (own, other) = (PipeFx::ids(1), PipeFx::ids(2));
    db.write(move |tx| {
        put_fixture(tx, own)?;
        put_fixture(tx, other)
    })
    .await
    .expect("pipeline fixture");
    (db, own, other)
}

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

/// The TS `insertItem` defaults: stage 0, contact 0, `Item <n>`, open, medium, `{}`.
pub fn item_values(fx: PipeFx) -> PipelineItemInsert {
    let n = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    PipelineItemInsert {
        workspace_id: fx.workspace_id,
        pipeline_id: fx.pipeline_id,
        stage_id: fx.stage_ids[0],
        contact_id: fx.contact_ids[0],
        title: format!("Item {n}"),
        status: "open".to_owned(),
        priority: "medium".to_owned(),
        field_values: Json(serde_json::json!({})),
        stage_entered_at: now_ms(),
        ..PipelineItemInsert::default()
    }
}

/// The TS `insertItem`: `nextItemNumber` + `insertIdempotent` in one write job, with the
/// overrides applied by `edit`.
pub async fn insert_item(
    db: &Db,
    fx: PipeFx,
    edit: impl FnOnce(&mut PipelineItemInsert) + Send + 'static,
) -> PipelineItem {
    let mut values = item_values(fx);
    edit(&mut values);
    repo::write(db, move |tx| {
        values.number = next_item_number(tx, values.workspace_id)?;
        Ok(insert_idempotent(tx, &values)?.row)
    })
    .await
    .expect("insert item")
}

/// Titles of `items`.
pub fn titles(items: &[PipelineItem]) -> Vec<String> {
    items.iter().map(|i| i.title.clone()).collect()
}
