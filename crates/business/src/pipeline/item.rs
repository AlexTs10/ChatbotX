//! `pipeline/item.service.ts`, `item-base.service.ts`, `item-fields.ts` and
//! the item reads of `item-query.service.ts` (`getItem`, `getItemView`).
//! `value` is the stored x 10,000 integer; dates are unix ms.

use std::collections::BTreeMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::PipelineItem;
use chatbotx_db::repo::pipeline_item_content::AttachmentWithFile;
use chatbotx_db::repo::pipeline_reference::ContactSummary;

use super::access::PipelineAccessScope;
use super::assignees::Assignees;
use super::events::PipelineItemChanged;
use super::mutation::PipelineActor;
use crate::errors::Result;

/// The item write fields (`PipelineItemWriteFields`); `Some(None)` clears.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemFields {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub value: Option<Option<i64>>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub due_date: Option<Option<i64>>,
    pub stage_id: Option<i64>,
    pub field_values: Option<BTreeMap<String, Option<String>>>,
    pub field_values_timezone: Option<String>,
    pub assignee_member_ids: Option<Vec<i64>>,
    pub assignee_inbox_team_ids: Option<Vec<i64>>,
}

/// `CreatePipelineItemInput`.
#[derive(Debug, Clone)]
pub struct CreateItem {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub contact_id: i64,
    pub scope: PipelineAccessScope,
    pub actor: PipelineActor,
    pub fields: ItemFields,
    pub media_library_file_ids: Vec<i64>,
    pub idempotency_key: Option<String>,
}

/// `UpdatePipelineItemInput` (`move` = a `stage_id`-only update).
#[derive(Debug, Clone)]
pub struct UpdateItem {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub item_id: i64,
    pub scope: PipelineAccessScope,
    pub actor: PipelineActor,
    pub fields: ItemFields,
}

/// The `transfer` input.
#[derive(Debug, Clone)]
pub struct TransferItem {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub item_id: i64,
    pub destination_pipeline_id: i64,
    pub destination_stage_id: Option<i64>,
    pub scope: PipelineAccessScope,
    pub actor: PipelineActor,
}

/// A write result with the realtime events it queued.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemMutation<T> {
    pub value: T,
    pub events: Vec<PipelineItemChanged>,
}

/// `PipelineItemView`.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    pub item: PipelineItem,
    pub contact: Option<ContactSummary>,
    pub assignees: Assignees,
    pub comment_count: i64,
}

/// `getItem`: the view and its attachments.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDetail {
    pub view: ItemView,
    pub attachments: Vec<AttachmentWithFile>,
}

/// `pipelineItemService.create`.
pub async fn create(db: &Db, input: CreateItem) -> Result<ItemMutation<PipelineItem>> {
    let _ = (db, input);
    todo!("P3.6-D1p")
}

/// `pipelineItemService.update` / `move`.
pub async fn update(db: &Db, input: UpdateItem) -> Result<ItemMutation<PipelineItem>> {
    let _ = (db, input);
    todo!("P3.6-D1p")
}

/// `pipelineItemService.transfer`.
pub async fn transfer(db: &Db, input: TransferItem) -> Result<ItemMutation<PipelineItem>> {
    let _ = (db, input);
    todo!("P3.6-D1p")
}

/// `pipelineItemService.delete` (the activity history stays).
pub async fn delete(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    item_id: i64,
    scope: PipelineAccessScope,
    actor: PipelineActor,
) -> Result<ItemMutation<()>> {
    let _ = (db, workspace_id, pipeline_id, item_id, scope, actor);
    todo!("P3.6-D1p")
}

/// `pipelineItemQueryService.getItem` (without `conversationId`).
pub async fn get_item(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    item_id: i64,
    scope: PipelineAccessScope,
) -> Result<ItemDetail> {
    let _ = (db, workspace_id, pipeline_id, item_id, scope);
    todo!("P3.6-D1p")
}
