//! Port of `packages/business/__tests__/pipeline-mutation-engine.test.ts`.
//! Ids are integers (`s-1` = 1, `s-2` = 2, `p-1` = 1, `i-1` = 1, `c-1` = 1,
//! `u-1` = 1) and values the stored x 10,000 integers. Not-applicable:
//! `dispatchPipelineEffects` (event bus, conversation service, counts cache;
//! none is ported), see the test map.

use std::collections::{BTreeMap, HashMap};

use chatbotx_business::pipeline::currency::PipelineView;
use chatbotx_business::pipeline::events::PipelineItemChanged;
use chatbotx_business::pipeline::mutation::{
    ActivityContext, Change, EventSubject, ItemSnapshot, PipelineActor, PipelineEffect,
    activity_rows_from_diff, diff_pipeline_item, effects_from_diff, is_empty_diff, realtime_effect,
};
use chatbotx_db::repo::Json;
use chatbotx_db::repo::models::{Pipeline, PipelineItem, PipelineStage};
use serde_json::json;

fn snapshot() -> ItemSnapshot {
    ItemSnapshot {
        stage_id: 1,
        status: "open".to_string(),
        priority: "medium".to_string(),
        value: Some(100),
        due_date: None,
        title: "Deal".to_string(),
        description: None,
        field_values: BTreeMap::new(),
    }
}

fn item() -> PipelineItem {
    PipelineItem {
        id: 1,
        created_at: 0,
        updated_at: 0,
        workspace_id: 9,
        pipeline_id: 1,
        stage_id: 1,
        contact_id: 1,
        number: 7,
        title: "Deal".to_string(),
        description: None,
        value: Some(100),
        status: "open".to_string(),
        priority: "medium".to_string(),
        due_date: None,
        field_values: Json(json!({})),
        created_by_id: None,
        closed_by_id: None,
        stage_entered_at: 0,
        status_changed_at: None,
        closed_at: None,
        last_comment_at: None,
        idempotency_key: None,
    }
}

fn pipeline() -> PipelineView {
    PipelineView {
        pipeline: Pipeline {
            id: 1,
            created_at: 0,
            updated_at: 0,
            workspace_id: 9,
            name: "Sales".to_string(),
            r#type: "opportunity".to_string(),
            reference_name: "sales".to_string(),
            currency: Some("USD".to_string()),
            settings: Json(
                json!({ "statusLabels": { "open": "Open", "won": "Won", "lost": "Lost" } }),
            ),
            created_by_id: None,
        },
        currency: "USD".to_string(),
        currency_override: Some("USD".to_string()),
    }
}

fn stage() -> PipelineStage {
    PipelineStage {
        id: 2,
        created_at: 0,
        updated_at: 0,
        workspace_id: 9,
        pipeline_id: 1,
        name: "Won stage".to_string(),
        description: None,
        text_color: None,
        background_color: None,
        position: 1,
        bulk_job_id: None,
    }
}

fn actor() -> PipelineActor {
    PipelineActor {
        kind: "user".to_string(),
        user_id: Some(1),
        source_id: None,
    }
}

fn stage_names() -> HashMap<i64, String> {
    HashMap::from([(1, "Lead".to_string()), (2, "Won stage".to_string())])
}

const NOW: i64 = 1_792_922_400_000; // 2026-10-25T10:00:00.000Z

#[test]
fn diff_pipeline_item_reports_nothing_when_nothing_changed() {
    assert!(is_empty_diff(&diff_pipeline_item(&snapshot(), &snapshot())));
}

#[test]
fn diff_pipeline_item_reports_each_changed_attribute_with_from_and_to() {
    let after = ItemSnapshot {
        stage_id: 2,
        status: "won".to_string(),
        priority: "high".to_string(),
        value: Some(250),
        title: "Bigger deal".to_string(),
        ..snapshot()
    };
    let diff = diff_pipeline_item(&snapshot(), &after);
    assert_eq!(diff.stage, Some(Change { from: 1, to: 2 }));
    assert_eq!(
        diff.status,
        Some(Change {
            from: "open".to_string(),
            to: "won".to_string()
        })
    );
    assert_eq!(
        diff.priority,
        Some(Change {
            from: "medium".to_string(),
            to: "high".to_string()
        })
    );
    assert_eq!(
        diff.value,
        Some(Change {
            from: Some(100),
            to: Some(250)
        })
    );
    assert_eq!(
        diff.title,
        Some(Change {
            from: "Deal".to_string(),
            to: "Bigger deal".to_string()
        })
    );
}

#[test]
fn diff_pipeline_item_compares_dates_by_instant_and_field_values_by_content() {
    let due = 1_792_886_400_000; // 2026-10-25T00:00:00.000Z
    let before = ItemSnapshot {
        due_date: Some(due),
        field_values: BTreeMap::from([("a".to_string(), "1".to_string())]),
        ..snapshot()
    };
    assert!(is_empty_diff(&diff_pipeline_item(&before, &before.clone())));
    let changed = ItemSnapshot {
        field_values: BTreeMap::from([("a".to_string(), "2".to_string())]),
        ..before.clone()
    };
    assert!(diff_pipeline_item(&before, &changed).field_values);
    let cleared = ItemSnapshot {
        due_date: None,
        ..before.clone()
    };
    assert_eq!(
        diff_pipeline_item(&before, &cleared).due_date,
        Some(Change {
            from: Some(due),
            to: None
        })
    );
}

#[test]
fn diff_pipeline_item_a_value_cleared_to_null_is_a_change() {
    let after = ItemSnapshot {
        value: None,
        ..snapshot()
    };
    assert_eq!(
        diff_pipeline_item(&snapshot(), &after).value,
        Some(Change {
            from: Some(100),
            to: None
        })
    );
}

#[test]
fn activity_rows_from_diff_writes_one_row_per_changed_attribute() {
    let after = ItemSnapshot {
        stage_id: 2,
        priority: "high".to_string(),
        ..snapshot()
    };
    let diff = diff_pipeline_item(&snapshot(), &after);
    let (item, actor) = (item(), actor());
    let context = ActivityContext {
        item: &item,
        actor: &actor,
        now: NOW,
        from_pipeline_id: None,
    };
    let rows = activity_rows_from_diff(&diff, &context);
    let mut kinds: Vec<&str> = rows.iter().map(|row| row.kind.as_str()).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["priorityChanged", "stageChanged"]);
    assert!(
        rows.iter()
            .all(|row| row.item_number == 7 && row.actor_user_id == Some(1))
    );
}

#[test]
fn activity_rows_from_diff_a_status_change_records_the_value_at_closure_for_analytics() {
    let after = ItemSnapshot {
        status: "won".to_string(),
        ..snapshot()
    };
    let diff = diff_pipeline_item(&snapshot(), &after);
    let item = PipelineItem {
        status: "won".to_string(),
        value: Some(480),
        ..item()
    };
    let actor = actor();
    let rows = activity_rows_from_diff(
        &diff,
        &ActivityContext {
            item: &item,
            actor: &actor,
            now: NOW,
            from_pipeline_id: None,
        },
    );
    let row = rows.first().expect("one row");
    assert_eq!(row.kind, "statusChanged");
    assert_eq!(row.from_status.as_deref(), Some("open"));
    assert_eq!(row.to_status.as_deref(), Some("won"));
    assert_eq!(row.to_value, Some(480));
}

#[test]
fn activity_rows_from_diff_does_not_store_description_or_field_bodies() {
    let after = ItemSnapshot {
        description: Some("secret text".to_string()),
        field_values: BTreeMap::from([("a".to_string(), "private".to_string())]),
        ..snapshot()
    };
    let diff = diff_pipeline_item(&snapshot(), &after);
    let (item, actor) = (item(), actor());
    let rows = activity_rows_from_diff(
        &diff,
        &ActivityContext {
            item: &item,
            actor: &actor,
            now: NOW,
            from_pipeline_id: None,
        },
    );
    let text = format!("{rows:?}");
    assert!(!text.contains("secret text"));
    assert!(!text.contains("private"));
    let mut kinds: Vec<&str> = rows.iter().map(|row| row.kind.as_str()).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["descriptionChanged", "fieldValuesChanged"]);
}

fn with_subject<R>(run: impl FnOnce(&EventSubject<'_>) -> R) -> R {
    let (pipeline, stage, item, actor, names) =
        (pipeline(), stage(), item(), actor(), stage_names());
    run(&EventSubject {
        pipeline: &pipeline,
        stage: &stage,
        item: &item,
        actor: &actor,
        stage_names: &names,
    })
}

#[test]
fn effects_from_diff_emits_one_event_per_changed_event_bearing_attribute() {
    let after = ItemSnapshot {
        stage_id: 2,
        value: Some(5),
        priority: "low".to_string(),
        status: "lost".to_string(),
        ..snapshot()
    };
    let effects = with_subject(|subject| {
        effects_from_diff(&diff_pipeline_item(&snapshot(), &after), subject)
    });
    let mut events: Vec<&str> = effects
        .iter()
        .filter_map(|effect| match effect {
            PipelineEffect::Event { event_type, .. } => Some(*event_type),
            PipelineEffect::Realtime(_) => None,
        })
        .collect();
    events.sort_unstable();
    assert_eq!(
        events,
        [
            "pipelineItemMovedToStage",
            "pipelineItemPriorityChanged",
            "pipelineItemStatusChanged",
            "pipelineItemValueChanged",
        ]
    );
}

#[test]
fn effects_from_diff_a_title_only_change_emits_no_trigger_event_but_still_refreshes_the_board() {
    let after = ItemSnapshot {
        title: "Renamed".to_string(),
        ..snapshot()
    };
    let effects = with_subject(|subject| {
        effects_from_diff(&diff_pipeline_item(&snapshot(), &after), subject)
    });
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], PipelineEffect::Realtime(_)));
}

#[test]
fn effects_from_diff_an_unchanged_item_emits_nothing() {
    let effects = with_subject(|subject| effects_from_diff(&Default::default(), subject));
    assert!(effects.is_empty());
}

#[test]
fn effects_from_diff_the_moved_event_names_the_previous_stage_and_the_pipeline_as_source() {
    let after = ItemSnapshot {
        stage_id: 2,
        ..snapshot()
    };
    let effects = with_subject(|subject| {
        effects_from_diff(&diff_pipeline_item(&snapshot(), &after), subject)
    });
    let Some(PipelineEffect::Event {
        event_type,
        metadata,
        ..
    }) = effects.first()
    else {
        panic!("first effect is an event: {effects:?}");
    };
    assert_eq!(*event_type, "pipelineItemMovedToStage");
    assert_eq!(metadata["sourceId"], "1");
    assert_eq!(metadata["pipelineReferenceName"], "sales");
    assert_eq!(metadata["fromStageId"], "1");
    assert_eq!(metadata["fromStageName"], "Lead");
    assert_eq!(metadata["stageName"], "Won stage");
    assert_eq!(metadata["statusLabel"], "Open");
}

#[test]
fn realtime_effects_an_item_effect_carries_its_contact_a_pipeline_wide_one_does_not() {
    assert_eq!(
        realtime_effect(9, 1, Some((1, 3)), "updated"),
        PipelineEffect::Realtime(PipelineItemChanged {
            workspace_id: 9,
            pipeline_id: 1,
            item_id: Some(1),
            contact_id: Some(3),
            change: "updated",
        })
    );
    assert_eq!(
        realtime_effect(9, 1, None, "bulk"),
        PipelineEffect::Realtime(PipelineItemChanged {
            workspace_id: 9,
            pipeline_id: 1,
            item_id: None,
            contact_id: None,
            change: "bulk",
        })
    );
}
