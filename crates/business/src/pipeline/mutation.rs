//! `pipeline/mutation-engine.ts`: item diffs, activity rows and effects.
//! Values are the stored x 10,000 integers; dates are unix ms.

use std::collections::{BTreeMap, HashMap};

use chatbotx_db::repo::models::{PipelineItem, PipelineStage};
use chatbotx_db::repo::pipeline_activity::PipelineActivityInsert;
use serde_json::Value;

use super::currency::PipelineView;
use super::events::PipelineItemChanged;

/// `PipelineActor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineActor {
    /// `user` / `workspaceToken` / `flow` / `system` ...
    pub kind: String,
    pub user_id: Option<i64>,
    pub source_id: Option<String>,
}

/// `PipelineItemSnapshot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSnapshot {
    pub stage_id: i64,
    pub status: String,
    pub priority: String,
    pub value: Option<i64>,
    pub due_date: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub field_values: BTreeMap<String, String>,
}

impl ItemSnapshot {
    /// The snapshot of a stored item (`fieldValues` string entries only).
    pub fn of(item: &PipelineItem) -> ItemSnapshot {
        let _ = item;
        todo!("P3.6-D1p")
    }
}

/// `Change<T>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change<T> {
    pub from: T,
    pub to: T,
}

/// `PipelineItemDiff`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineItemDiff {
    pub stage: Option<Change<i64>>,
    pub status: Option<Change<String>>,
    pub priority: Option<Change<String>>,
    pub value: Option<Change<Option<i64>>>,
    pub due_date: Option<Change<Option<i64>>>,
    pub title: Option<Change<String>>,
    pub description: bool,
    pub field_values: bool,
}

/// `diffPipelineItem`.
pub fn diff_pipeline_item(before: &ItemSnapshot, after: &ItemSnapshot) -> PipelineItemDiff {
    let _ = (before, after);
    todo!("P3.6-D1p")
}

/// `isEmptyDiff`.
pub fn is_empty_diff(diff: &PipelineItemDiff) -> bool {
    let _ = diff;
    todo!("P3.6-D1p")
}

/// `ActivityContext`.
#[derive(Debug, Clone, Copy)]
pub struct ActivityContext<'a> {
    pub item: &'a PipelineItem,
    pub actor: &'a PipelineActor,
    pub now: i64,
    pub from_pipeline_id: Option<i64>,
}

/// `activityRowFor`: the common columns; the caller sets the kind's columns.
pub fn activity_row_for(kind: &str, context: &ActivityContext<'_>) -> PipelineActivityInsert {
    let _ = (kind, context);
    todo!("P3.6-D1p")
}

/// `activityRowsFromDiff`: one row per changed attribute (no description or
/// field bodies).
pub fn activity_rows_from_diff(
    diff: &PipelineItemDiff,
    context: &ActivityContext<'_>,
) -> Vec<PipelineActivityInsert> {
    let _ = (diff, context);
    todo!("P3.6-D1p")
}

/// `PipelineEffect` (the `assignContact` effect is not ported: no
/// conversation service).
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineEffect {
    /// `emitPipelineItemEvent` (flow triggers); `metadata` is
    /// `PipelineItemEventMetadata` (money in major units).
    Event {
        event_type: &'static str,
        workspace_id: i64,
        contact_id: i64,
        metadata: Value,
    },
    Realtime(PipelineItemChanged),
}

/// `PipelineEventSubject`.
#[derive(Debug, Clone, Copy)]
pub struct EventSubject<'a> {
    pub pipeline: &'a PipelineView,
    pub stage: &'a PipelineStage,
    pub item: &'a PipelineItem,
    pub actor: &'a PipelineActor,
    pub stage_names: &'a HashMap<i64, String>,
}

/// `pipelineEventMetadata` + `extra` keys.
pub fn pipeline_event_metadata(subject: &EventSubject<'_>, extra: Value) -> Value {
    let _ = (subject, extra);
    todo!("P3.6-D1p")
}

/// `effectsFromDiff`.
pub fn effects_from_diff(
    diff: &PipelineItemDiff,
    subject: &EventSubject<'_>,
) -> Vec<PipelineEffect> {
    let _ = (diff, subject);
    todo!("P3.6-D1p")
}

/// `createdEffects`.
pub fn created_effects(subject: &EventSubject<'_>) -> Vec<PipelineEffect> {
    let _ = subject;
    todo!("P3.6-D1p")
}

/// `realtimeEffect`: an item effect carries its id and contact.
pub fn realtime_effect(
    workspace_id: i64,
    pipeline_id: i64,
    item: Option<(i64, i64)>,
    change: &'static str,
) -> PipelineEffect {
    let _ = (workspace_id, pipeline_id, item, change);
    todo!("P3.6-D1p")
}

/// The realtime events of `effects`, in order (`dispatchPipelineEffects`
/// without the event bus).
pub fn realtime_events(effects: &[PipelineEffect]) -> Vec<PipelineItemChanged> {
    let _ = effects;
    todo!("P3.6-D1p")
}
