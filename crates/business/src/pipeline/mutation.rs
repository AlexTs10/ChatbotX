//! `pipeline/mutation-engine.ts`: item diffs, activity rows and effects.
//! Values are the stored x 10,000 integers; dates are unix ms.

use std::collections::{BTreeMap, HashMap};

use chatbotx_db::repo::models::{PipelineItem, PipelineStage};
use chatbotx_db::repo::pipeline_activity::PipelineActivityInsert;
use serde_json::{Map, Value, json};

use super::currency::PipelineView;
use super::events::PipelineItemChanged;
use super::service::stored_settings;

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
        let field_values = match &item.field_values.0 {
            Value::Object(map) => map
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect(),
            _ => BTreeMap::new(),
        };
        ItemSnapshot {
            stage_id: item.stage_id,
            status: item.status.clone(),
            priority: item.priority.clone(),
            value: item.value,
            due_date: item.due_date,
            title: item.title.clone(),
            description: item.description.clone(),
            field_values,
        }
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
    fn change<T: PartialEq + Clone>(from: &T, to: &T) -> Option<Change<T>> {
        (from != to).then(|| Change {
            from: from.clone(),
            to: to.clone(),
        })
    }
    PipelineItemDiff {
        stage: change(&before.stage_id, &after.stage_id),
        status: change(&before.status, &after.status),
        priority: change(&before.priority, &after.priority),
        value: change(&before.value, &after.value),
        due_date: change(&before.due_date, &after.due_date),
        title: change(&before.title, &after.title),
        description: before.description != after.description,
        field_values: before.field_values != after.field_values,
    }
}

/// `isEmptyDiff`.
pub fn is_empty_diff(diff: &PipelineItemDiff) -> bool {
    *diff == PipelineItemDiff::default()
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
    let item = context.item;
    PipelineActivityInsert {
        workspace_id: item.workspace_id,
        pipeline_id: item.pipeline_id,
        item_id: item.id,
        item_number: item.number,
        item_title: item.title.clone(),
        contact_id: item.contact_id,
        kind: kind.to_string(),
        actor_kind: context.actor.kind.clone(),
        actor_user_id: context.actor.user_id,
        actor_source_id: context.actor.source_id.clone(),
        from_pipeline_id: context.from_pipeline_id,
        occurred_at: context.now,
        ..PipelineActivityInsert::default()
    }
}

/// `activityRowsFromDiff`: one row per changed attribute (no description or
/// field bodies).
pub fn activity_rows_from_diff(
    diff: &PipelineItemDiff,
    context: &ActivityContext<'_>,
) -> Vec<PipelineActivityInsert> {
    let mut rows = Vec::new();
    let mut push = |kind: &str, set: &dyn Fn(&mut PipelineActivityInsert)| {
        let mut row = activity_row_for(kind, context);
        set(&mut row);
        rows.push(row);
    };
    if let Some(stage) = &diff.stage {
        push("stageChanged", &|row| {
            row.from_stage_id = Some(stage.from);
            row.to_stage_id = Some(stage.to);
        });
    }
    if let Some(status) = &diff.status {
        push("statusChanged", &|row| {
            row.from_status = Some(status.from.clone());
            row.to_status = Some(status.to.clone());
            row.to_value = context.item.value;
        });
    }
    if let Some(value) = &diff.value {
        push("valueChanged", &|row| {
            row.from_value = value.from;
            row.to_value = value.to;
        });
    }
    if let Some(priority) = &diff.priority {
        push("priorityChanged", &|row| {
            row.from_priority = Some(priority.from.clone());
            row.to_priority = Some(priority.to.clone());
        });
    }
    if let Some(due) = &diff.due_date {
        push("dueDateChanged", &|row| {
            row.from_due_date = due.from;
            row.to_due_date = due.to;
        });
    }
    if let Some(title) = &diff.title {
        push("titleChanged", &|row| {
            row.from_title = Some(title.from.clone());
            row.to_title = Some(title.to.clone());
        });
    }
    if diff.description {
        push("descriptionChanged", &|_| {});
    }
    if diff.field_values {
        push("fieldValuesChanged", &|_| {});
    }
    rows
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
    let EventSubject {
        pipeline,
        stage,
        item,
        actor,
        ..
    } = subject;
    let settings = stored_settings(&pipeline.pipeline);
    let label = match item.status.as_str() {
        "won" => settings.status_labels.won,
        "lost" => settings.status_labels.lost,
        _ => settings.status_labels.open,
    };
    let mut out = json!({
        "sourceId": pipeline.pipeline.id.to_string(),
        "pipelineId": pipeline.pipeline.id.to_string(),
        "pipelineName": pipeline.pipeline.name,
        "pipelineReferenceName": pipeline.pipeline.reference_name,
        "itemId": item.id.to_string(),
        "itemNumber": item.number,
        "title": item.title,
        "contactId": item.contact_id.to_string(),
        "stageId": stage.id.to_string(),
        "stageName": stage.name,
        "status": item.status,
        "statusLabel": label,
        "priority": item.priority,
        "value": item.value.map(major_units),
        "currency": pipeline.currency,
        "dueDate": item.due_date.map(chatbotx_db::repo::iso),
        "actorKind": actor.kind,
        "actorUserId": actor.user_id.map(|id| id.to_string()),
    });
    if let (Value::Object(base), Value::Object(extra)) = (&mut out, extra) {
        base.extend(extra);
    }
    out
}

/// `effectsFromDiff`.
pub fn effects_from_diff(
    diff: &PipelineItemDiff,
    subject: &EventSubject<'_>,
) -> Vec<PipelineEffect> {
    let event = |event_type: &'static str, extra: Value| PipelineEffect::Event {
        event_type,
        workspace_id: subject.item.workspace_id,
        contact_id: subject.item.contact_id,
        metadata: pipeline_event_metadata(subject, extra),
    };
    let mut effects = Vec::new();
    if let Some(stage) = &diff.stage {
        effects.push(event(
            "pipelineItemMovedToStage",
            json!({
                "fromStageId": stage.from.to_string(),
                "fromStageName": subject.stage_names.get(&stage.from),
            }),
        ));
    }
    if let Some(value) = &diff.value {
        effects.push(event(
            "pipelineItemValueChanged",
            json!({ "previousValue": value.from.map(major_units) }),
        ));
    }
    if let Some(status) = &diff.status {
        effects.push(event(
            "pipelineItemStatusChanged",
            json!({ "previousStatus": status.from }),
        ));
    }
    if let Some(priority) = &diff.priority {
        effects.push(event(
            "pipelineItemPriorityChanged",
            json!({ "previousPriority": priority.from }),
        ));
    }
    if !is_empty_diff(diff) {
        let change = if diff.stage.is_some() {
            "moved"
        } else {
            "updated"
        };
        effects.push(item_realtime(subject.item, change));
    }
    effects
}

/// `createdEffects`.
pub fn created_effects(subject: &EventSubject<'_>) -> Vec<PipelineEffect> {
    vec![
        PipelineEffect::Event {
            event_type: "pipelineItemCreated",
            workspace_id: subject.item.workspace_id,
            contact_id: subject.item.contact_id,
            metadata: pipeline_event_metadata(subject, Value::Object(Map::new())),
        },
        item_realtime(subject.item, "created"),
    ]
}

/// `realtimeEffect`: an item effect carries its id and contact.
pub fn realtime_effect(
    workspace_id: i64,
    pipeline_id: i64,
    item: Option<(i64, i64)>,
    change: &'static str,
) -> PipelineEffect {
    PipelineEffect::Realtime(PipelineItemChanged {
        workspace_id,
        pipeline_id,
        item_id: item.map(|(id, _)| id),
        contact_id: item.map(|(_, contact)| contact),
        change,
    })
}

/// The realtime events of `effects`, in order (`dispatchPipelineEffects`
/// without the event bus).
pub fn realtime_events(effects: &[PipelineEffect]) -> Vec<PipelineItemChanged> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            PipelineEffect::Realtime(event) => Some(event.clone()),
            PipelineEffect::Event { .. } => None,
        })
        .collect()
}

/// The stored x 10,000 value in major units (the TS `float8`).
pub fn major_units(stored: i64) -> f64 {
    stored as f64 / 10_000.0
}

/// [`realtime_effect`] of one item.
pub fn item_realtime(item: &PipelineItem, change: &'static str) -> PipelineEffect {
    realtime_effect(
        item.workspace_id,
        item.pipeline_id,
        Some((item.id, item.contact_id)),
        change,
    )
}
