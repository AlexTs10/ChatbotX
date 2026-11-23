//! `features/contacts/api/public/bulk.ts` (`contactsBulkPublicRouter`,
//! P3.6-D2 slice contacts bulk). The by-stats job runs in the request (no
//! default-queue worker is ported, see `bulk_tag_stats`).

use chatbotx_business::contact::bulk_tag_stats::{
    BulkTagRequest, BulkTagSource, assert_source_owned, prepare, run,
};
use chatbotx_business::contact::delete_and_record;
use chatbotx_business::contact_sequence::{subscribe_contacts, unsubscribe_contacts};
use chatbotx_business::events::trigger;
use chatbotx_business::tag::{
    ContactsOutcome, attach_by_names_to_contacts_with_tags,
    detach_by_names_from_contacts_with_pairs,
};
use chatbotx_business::tag_sync;
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, key};
use super::{ApiState, CREATING, Call, MUTATING, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.bulkAddTags",
    "contacts.bulkRemoveTags",
    "contacts.bulkTagByStats",
    "contacts.bulkDelete",
    "contacts.bulkSubscribeSequences",
    "contacts.bulkUnsubscribeSequences",
];

/// `MAX_BULK_CONTACT_IDS`.
const MAX_BULK_CONTACT_IDS: usize = 1000;

/// The tracked stats events (`trackedStatsEvent`).
const TRACKED_EVENTS: &[&str] = &[
    "message:sent",
    "message:delivered",
    "message:seen",
    "message:failed",
    "flow:clicked",
];

/// `commentAutomationEventType`.
const COMMENT_EVENTS: &[&str] = &[
    "message:sent",
    "message:delivered",
    "message:seen",
    "message:failed",
    "flow:clicked",
    "comment:missed",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "contacts.bulkAddTags" => {
            let mut input = Input::from_body(call.params, call.body);
            let contact_ids = contact_ids(&mut input);
            let tags = names(&mut input, "tags");
            input.finish()?;
            let (contact_ids, tags) = (contact_ids.unwrap_or_default(), tags.unwrap_or_default());
            let (outcome, tag_ids, linked) =
                attach_by_names_to_contacts_with_tags(db, ws, &contact_ids, &tags)
                    .await
                    .map_err(|e| business_error(e, CREATING))?;
            if let Some(jobs) = state.jobs.as_deref() {
                tag_sync::enqueue_attach_many(jobs, ws, &linked).await;
            }
            // `emitTagApplied` for every attempted pair (`emitFor: "all"`).
            let workspace = ws.to_string();
            let events: Vec<_> = outcome
                .processed_contact_ids
                .iter()
                .flat_map(|contact| {
                    let contact = contact.to_string();
                    let workspace = workspace.clone();
                    tag_ids.iter().map(move |tag| {
                        trigger::tag_applied(&workspace, &contact, &tag.to_string(), None)
                    })
                })
                .collect();
            state.emit_triggers(events).await;
            Ok(result(&outcome))
        }
        "contacts.bulkRemoveTags" => {
            let mut input = Input::from_body(call.params, call.body);
            let contact_ids = contact_ids(&mut input);
            let tags = names(&mut input, "tags");
            input.finish()?;
            let (contact_ids, tags) = (contact_ids.unwrap_or_default(), tags.unwrap_or_default());
            let (outcome, removed) =
                detach_by_names_from_contacts_with_pairs(db, ws, &contact_ids, &tags)
                    .await
                    .map_err(|e| business_error(e, CREATING))?;
            let workspace = ws.to_string();
            let events: Vec<_> = removed
                .iter()
                .map(|(contact, tag)| {
                    trigger::tag_removed(&workspace, &contact.to_string(), &tag.to_string(), None)
                })
                .collect();
            state.emit_triggers(events).await;
            if let Some(jobs) = state.jobs.as_deref() {
                tag_sync::enqueue_detach_many(jobs, ws, &removed).await;
            }
            Ok(result(&outcome))
        }
        "contacts.bulkTagByStats" => {
            let request = by_stats_request(&call)?;
            assert_source_owned(db, ws, &request.source)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let owner = workspace_owner(state, ws).await?;
            let job = prepare(db, ws, owner, &request, None)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let queued = job.is_some();
            if let Some(job) = job {
                let linked = run(db, &job)
                    .await
                    .map_err(|e| business_error(e, MUTATING))?;
                let workspace = ws.to_string();
                let events: Vec<_> = linked
                    .iter()
                    .map(|(contact, tag)| {
                        trigger::tag_applied(
                            &workspace,
                            &contact.to_string(),
                            &tag.to_string(),
                            None,
                        )
                    })
                    .collect();
                state.emit_triggers(events).await;
                if let Some(jobs) = state.jobs.as_deref() {
                    tag_sync::enqueue_attach_many(jobs, ws, &linked).await;
                }
            }
            Ok(json(202, json!({ "queued": queued })))
        }
        "contacts.bulkDelete" => {
            let mut input = Input::from_body(call.params, call.body);
            let contact_ids = contact_ids(&mut input);
            input.finish()?;
            let contact_ids = contact_ids.unwrap_or_default();
            let outcome = delete_and_record(db, ws, &contact_ids)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            super::contacts::audit_deleted(state, &outcome.processed_contact_ids).await;
            Ok(json(
                200,
                json!({
                    "processed": outcome.processed_contact_ids.len(),
                    "skippedContactIds": strings(&outcome.skipped_contact_ids),
                }),
            ))
        }
        "contacts.bulkSubscribeSequences" | "contacts.bulkUnsubscribeSequences" => {
            let mut input = Input::from_body(call.params, call.body);
            let contact_ids = contact_ids(&mut input);
            let sequence_ids = ids(&mut input, "sequenceIds", 1, 20);
            input.finish()?;
            let (contact_ids, sequence_ids) = (
                contact_ids.unwrap_or_default(),
                sequence_ids.unwrap_or_default(),
            );
            let outcome = if route_id == "contacts.bulkSubscribeSequences" {
                subscribe_contacts(db, ws, &contact_ids, &sequence_ids)
                    .await
                    .map_err(|e| business_error(e, CREATING))?
            } else {
                unsubscribe_contacts(db, ws, &contact_ids, &sequence_ids)
                    .await
                    .map_err(|e| business_error(e, MUTATING))?
            };
            Ok(json(
                200,
                json!({
                    "processed": outcome.processed_contact_ids.len(),
                    "skippedContactIds": strings(&outcome.skipped_contact_ids),
                }),
            ))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

fn strings(ids: &[i64]) -> Vec<String> {
    ids.iter().map(i64::to_string).collect()
}

/// `bulkResultPublicResponse`.
fn result(outcome: &ContactsOutcome) -> axum::response::Response {
    json(
        200,
        json!({
            "processed": outcome.processed_contact_ids.len(),
            "skippedContactIds": strings(&outcome.skipped_contact_ids),
        }),
    )
}

/// `context.workspace.ownerId` (the job's `requestedUserId`).
async fn workspace_owner(
    state: &ApiState,
    workspace_id: i64,
) -> Result<i64, crate::orpc::OrpcError> {
    state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .await
        .map_err(|e| super::internal_error(&e.to_string()))
}

/// `z.array(element).min(min).max(max)` at a top-level key; `None` when an
/// issue was recorded.
fn array<T>(
    input: &mut Input,
    name: &str,
    min: usize,
    max: usize,
    mut element: impl FnMut(&mut Input, &[Seg], &Value) -> Option<T>,
) -> Option<Vec<T>> {
    let items = match input.get(name).cloned() {
        None => {
            input.issue(
                &key(name),
                "invalid_type",
                "Invalid input: expected array, received undefined",
            );
            return None;
        }
        Some(Value::Array(items)) => items,
        Some(_) => {
            input.issue(&key(name), "invalid_type", "Invalid input: expected array");
            return None;
        }
    };
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let path = vec![Seg::Key(name.to_string()), Seg::Index(index)];
        match element(input, &path, item) {
            Some(value) => out.push(value),
            None => ok = false,
        }
    }
    if items.len() < min {
        input.issue(
            &key(name),
            "too_small",
            &format!("Too small: expected array to have >={min} items"),
        );
        ok = false;
    }
    if items.len() > max {
        input.issue(
            &key(name),
            "too_big",
            &format!("Too big: expected array to have <={max} items"),
        );
        ok = false;
    }
    ok.then_some(out)
}

/// `z.array(zodBigintAsString())` at `name`.
pub(super) fn ids(input: &mut Input, name: &str, min: usize, max: usize) -> Option<Vec<i64>> {
    array(input, name, min, max, |input, path, value| {
        input.id_at(path, Some(value), true)
    })
}

/// `contactIds`: 1 to 1000 ids.
fn contact_ids(input: &mut Input) -> Option<Vec<i64>> {
    ids(input, "contactIds", 1, MAX_BULK_CONTACT_IDS)
}

/// `z.array(z.string().trim().min(1)).min(1).max(20)`.
fn names(input: &mut Input, name: &str) -> Option<Vec<String>> {
    array(input, name, 1, 20, |input, path, value| {
        input.string_at(path, Some(value), Str::trimmed(1, usize::MAX), true)
    })
}

/// `bulkTagByStatsPublicRequest` (a discriminated union on `source`).
pub(crate) fn by_stats_request(call: &Call<'_>) -> Result<BulkTagRequest, crate::orpc::OrpcError> {
    let mut input = Input::from_body(call.params, call.body);
    let source = input.get("source").cloned();
    let source = match source.as_ref().and_then(Value::as_str) {
        Some(kind @ ("broadcast" | "sequenceStep" | "commentAutomation")) => kind.to_string(),
        _ => {
            input.issue(&key("source"), "invalid_union", "Invalid input");
            input.finish()?;
            return Err(crate::routes::not_ported_error());
        }
    };
    let id = |input: &mut Input, name: &str| {
        let value = input.get(name).cloned();
        input.id_at(&key(name), value.as_ref(), true)
    };
    let event = |input: &mut Input, options: &[&str]| {
        let value = input.get("eventType").cloned();
        input.enum_at(&key("eventType"), value.as_ref(), options, true)
    };
    let parsed = match source.as_str() {
        "broadcast" => {
            let broadcast_id = id(&mut input, "broadcastId");
            let event_type = event(&mut input, TRACKED_EVENTS);
            broadcast_id
                .zip(event_type)
                .map(|(broadcast_id, event_type)| BulkTagSource::Broadcast {
                    broadcast_id,
                    event_type,
                })
        }
        "sequenceStep" => {
            let sequence_id = id(&mut input, "sequenceId");
            let step_id = id(&mut input, "stepId");
            let event_type = event(&mut input, TRACKED_EVENTS);
            match (sequence_id, step_id, event_type) {
                (Some(sequence_id), Some(step_id), Some(event_type)) => {
                    Some(BulkTagSource::SequenceStep {
                        sequence_id,
                        step_id,
                        event_type,
                    })
                }
                _ => None,
            }
        }
        _ => {
            let automation_id = id(&mut input, "automationId");
            let event_type = event(&mut input, COMMENT_EVENTS);
            automation_id
                .zip(event_type)
                .map(
                    |(automation_id, event_type)| BulkTagSource::CommentAutomation {
                        automation_id,
                        event_type,
                    },
                )
        }
    };
    let tags = names(&mut input, "tags");
    let excluded = if input.get("excludedContactIds").is_some() {
        ids(&mut input, "excludedContactIds", 0, usize::MAX)
    } else {
        Some(Vec::new())
    };
    input.finish()?;
    match (parsed, tags, excluded) {
        (Some(source), Some(tags), Some(excluded_contact_ids)) => Ok(BulkTagRequest {
            source,
            tags,
            excluded_contact_ids,
        }),
        _ => Err(crate::routes::not_ported_error()),
    }
}
