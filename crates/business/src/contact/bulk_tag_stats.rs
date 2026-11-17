//! `contacts.bulkTagByStats` (P3.6-D2 slice contacts bulk): builder
//! `features/contacts/lib/enqueue-bulk-tag-stats.ts` (`prepare`), the worker
//! job `default/handlers/bulk-tag-contacts.ts` (`run`) and the
//! `getContactIdsPage` queries of `analytics` `broadcast-stats` /
//! `sequence-stats` repositories (`contact_ids_page`).
//!
//! No default-queue worker is ported yet: the API runs the job in the
//! request ([`run`]); [`BulkTagJob::job_data`] keeps the original
//! `bulkTagContacts` payload for the worker port. The comment-automation
//! source finds no contacts (its stats / miss repositories belong to the
//! comment-automation analytics port). The member access scope
//! (`restrictToAssignedUserId`) is carried in the payload, not applied.

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension as _, params, params_from_iter};
use serde_json::{Map, Value};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `DefaultJobAction.bulkTagContacts`.
pub const JOB_NAME: &str = "bulkTagContacts";

/// `loopableItemsCount` (worker-config): the page size of the job.
pub const LOOPABLE_ITEMS_COUNT: usize = 1000;

/// The stats event whose contacts are tagged (`BulkTagSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkTagSource {
    Broadcast {
        broadcast_id: i64,
        event_type: String,
    },
    SequenceStep {
        sequence_id: i64,
        step_id: i64,
        event_type: String,
    },
    CommentAutomation {
        automation_id: i64,
        event_type: String,
    },
}

/// `BulkTagStatsContactsRequest` (the parsed route / action input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkTagRequest {
    pub source: BulkTagSource,
    pub tags: Vec<String>,
    pub excluded_contact_ids: Vec<i64>,
}

/// `JobBulkTagContacts["data"]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkTagJob {
    pub workspace_id: i64,
    pub requested_user_id: i64,
    pub tag_ids: Vec<i64>,
    pub excluded_contact_ids: Vec<i64>,
    pub restrict_to_assigned_user_id: Option<i64>,
    pub source: BulkTagSource,
}

impl BulkTagJob {
    /// The `defaultQueue.add(bulkTagContacts, ...)` payload:
    /// `{ type: "bulkTagContacts", data: { workspaceId, requestedUserId,
    /// tagIds, excludedContactIds, [restrictToAssignedUserId], source, ... } }`
    /// (ids as strings).
    pub fn job_data(&self) -> Value {
        let ids = |ids: &[i64]| {
            Value::Array(ids.iter().map(|id| Value::String(id.to_string())).collect())
        };
        let mut data = Map::new();
        data.insert(
            "workspaceId".into(),
            Value::String(self.workspace_id.to_string()),
        );
        data.insert(
            "requestedUserId".into(),
            Value::String(self.requested_user_id.to_string()),
        );
        data.insert("tagIds".into(), ids(&self.tag_ids));
        data.insert("excludedContactIds".into(), ids(&self.excluded_contact_ids));
        if let Some(user) = self.restrict_to_assigned_user_id {
            data.insert(
                "restrictToAssignedUserId".into(),
                Value::String(user.to_string()),
            );
        }
        match &self.source {
            BulkTagSource::Broadcast {
                broadcast_id,
                event_type,
            } => {
                data.insert("source".into(), Value::from("broadcast"));
                data.insert(
                    "broadcastId".into(),
                    Value::String(broadcast_id.to_string()),
                );
                data.insert("eventType".into(), Value::String(event_type.clone()));
            }
            BulkTagSource::SequenceStep {
                sequence_id,
                step_id,
                event_type,
            } => {
                data.insert("source".into(), Value::from("sequenceStep"));
                data.insert("sequenceId".into(), Value::String(sequence_id.to_string()));
                data.insert("stepId".into(), Value::String(step_id.to_string()));
                data.insert("eventType".into(), Value::String(event_type.clone()));
            }
            BulkTagSource::CommentAutomation {
                automation_id,
                event_type,
            } => {
                data.insert("source".into(), Value::from("commentAutomation"));
                data.insert(
                    "automationId".into(),
                    Value::String(automation_id.to_string()),
                );
                data.insert("eventType".into(), Value::String(event_type.clone()));
            }
        }
        let mut job = Map::new();
        job.insert("type".into(), Value::from(JOB_NAME));
        job.insert("data".into(), Value::Object(data));
        Value::Object(job)
    }
}

/// One `getContactIdsPage` row: the cursor id and the contact id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactIdRow {
    pub id: i64,
    pub contact_id: i64,
}

/// The route pre-check: a broadcast (`broadcastService.findByIdOrName`, 404
/// "Broadcast not found") or a sequence (`sequenceService.assertOwned`, 404
/// "Sequence not found") of another workspace; a comment automation is not
/// checked (scoped in the job).
pub async fn assert_source_owned(db: &Db, workspace_id: i64, source: &BulkTagSource) -> Result<()> {
    let source = source.clone();
    tx::read(db, move |conn| match source {
        BulkTagSource::Broadcast { broadcast_id, .. } => {
            repo::broadcast::find_by_id_or_name(conn, workspace_id, &broadcast_id.to_string())?
                .map(|_| ())
                .ok_or_else(|| BusinessError::not_found("Broadcast not found"))
        }
        BulkTagSource::SequenceStep { sequence_id, .. } => conn
            .query_row(
                r#"SELECT 1 FROM "Sequence" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![sequence_id, workspace_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(repo_err)?
            .ok_or_else(|| BusinessError::not_found("Sequence not found")),
        BulkTagSource::CommentAutomation { .. } => Ok(()),
    })
    .await
}

/// `enqueueBulkTagStatsContacts` without the enqueue: upserts the tags by
/// name; `None` when no tag name is usable (nothing to queue).
pub async fn prepare(
    db: &Db,
    workspace_id: i64,
    requested_user_id: i64,
    request: &BulkTagRequest,
    restrict_to_assigned_user_id: Option<i64>,
) -> Result<Option<BulkTagJob>> {
    let tags = crate::tag::upsert_by_names(db, workspace_id, &request.tags).await?;
    if tags.is_empty() {
        return Ok(None);
    }
    Ok(Some(BulkTagJob {
        workspace_id,
        requested_user_id,
        tag_ids: tags.into_iter().map(|(id, _)| id).collect(),
        excluded_contact_ids: request.excluded_contact_ids.clone(),
        restrict_to_assigned_user_id,
        source: request.source.clone(),
    }))
}

/// The `buildEventFilter` condition of the broadcast / sequence stats
/// repositories (`message:sent` = delivered or failed; unknown = delivered).
pub(crate) fn event_condition(event_type: &str) -> &'static str {
    match event_type {
        "message:sent" => r#"("deliveredAt" IS NOT NULL OR "failedAt" IS NOT NULL)"#,
        "message:seen" => r#""seenAt" IS NOT NULL"#,
        "message:failed" => r#""failedAt" IS NOT NULL"#,
        "flow:clicked" => r#""clickedAt" IS NOT NULL"#,
        _ => r#""deliveredAt" IS NOT NULL"#,
    }
}

/// `getContactIdsPage` of the job's source: broadcast rows by `contactId`
/// (cursor = contact id), sequence-step dispatches by `id` (cursor =
/// dispatch id), both without the excluded contacts.
pub async fn contact_ids_page(
    db: &Db,
    job: &BulkTagJob,
    cursor: Option<i64>,
    limit: usize,
) -> Result<Vec<ContactIdRow>> {
    let (id_column, table, scope, mut binds, event_type) = match &job.source {
        BulkTagSource::Broadcast {
            broadcast_id,
            event_type,
        } => (
            r#""contactId""#,
            r#""ContactOnBroadcast""#,
            r#""broadcastId" = ?"#.to_string(),
            vec![Sql::Integer(*broadcast_id)],
            event_type.clone(),
        ),
        BulkTagSource::SequenceStep {
            sequence_id,
            step_id,
            event_type,
        } => (
            r#""id""#,
            r#""SequenceDispatch""#,
            r#""workspaceId" = ? AND "sequenceId" = ? AND "stepId" = ?"#.to_string(),
            vec![
                Sql::Integer(job.workspace_id),
                Sql::Integer(*sequence_id),
                Sql::Integer(*step_id),
            ],
            event_type.clone(),
        ),
        // The comment-automation stats / miss repositories are not ported.
        BulkTagSource::CommentAutomation { .. } => return Ok(Vec::new()),
    };
    let mut filter = format!("{scope} AND {}", event_condition(&event_type));
    if let Some(cursor) = cursor {
        filter.push_str(&format!(" AND {id_column} > ?"));
        binds.push(Sql::Integer(cursor));
    }
    if !job.excluded_contact_ids.is_empty() {
        let marks = vec!["?"; job.excluded_contact_ids.len()].join(", ");
        filter.push_str(&format!(r#" AND "contactId" NOT IN ({marks})"#));
        binds.extend(job.excluded_contact_ids.iter().copied().map(Sql::Integer));
    }
    binds.push(Sql::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));
    let sql = format!(
        r#"SELECT {id_column}, "contactId" FROM {table} WHERE {filter} ORDER BY {id_column} LIMIT ?"#
    );
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let rows = stmt
            .query_map(params_from_iter(binds.iter()), |row| {
                Ok(ContactIdRow {
                    id: row.get(0)?,
                    contact_id: row.get(1)?,
                })
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await
}

/// `handleBulkTagContacts`: pages through the source (`chunkById`) and
/// attaches the tags to each page's unique contacts. Returns the newly linked
/// `(contactId, tagId)` pairs (`tagApplied` events).
pub async fn run(db: &Db, job: &BulkTagJob) -> Result<Vec<(i64, i64)>> {
    let mut linked = Vec::new();
    let mut cursor = None;
    loop {
        let page = contact_ids_page(db, job, cursor, LOOPABLE_ITEMS_COUNT).await?;
        let Some(last) = page.last() else {
            break;
        };
        cursor = Some(last.id);
        let mut contact_ids = Vec::new();
        for row in &page {
            if !contact_ids.contains(&row.contact_id) {
                contact_ids.push(row.contact_id);
            }
        }
        let outcome =
            crate::tag::bulk_attach_to_contacts(db, job.workspace_id, &contact_ids, &job.tag_ids)
                .await?;
        linked.extend(outcome.newly_linked);
        if page.len() < LOOPABLE_ITEMS_COUNT {
            break;
        }
    }
    Ok(linked)
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}
