//! `apps/builder/src/features/tags/api/public.ts` (`tagsPublicRouter`) and
//! the tag routes of `features/contacts/api/public/tags.ts`.

use chatbotx_business::events::trigger;
use chatbotx_business::tag::{self as service, ListTags};
use chatbotx_business::tag_sync;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::Tag;
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, key};
use super::qr_codes::{public_paging, sort_query};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error, id, json,
    no_content, opt_id,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "tags.list",
    "tags.create",
    "tags.get",
    "tags.update",
    "tags.delete",
    "contacts.listTags",
    "contacts.addTags",
    "contacts.removeTags",
    "contacts.setTags",
    "contacts.addTagsByName",
    "contacts.removeTagsByName",
];

/// `possibleErrorsOnCreatingInFolder`.
const CREATING_IN_FOLDER: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
];

/// `publicTagResource`.
pub fn tag_json(tag: &Tag) -> Value {
    json!({ "id": id(tag.id), "name": tag.name, "folderId": opt_id(tag.folder_id) })
}

/// `zodBigintAsString()` at `path` (digits, any length; beyond `i64` = `-1`,
/// which matches no row).
pub(crate) fn digits_at(
    input: &mut Input,
    path: &[Seg],
    value: Option<&Value>,
    required: bool,
) -> Option<i64> {
    let text = input.string_at(path, value, Str::default(), required)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            path,
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    Some(text.parse().unwrap_or(-1))
}

fn digits(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name).cloned();
    digits_at(input, &key(name), value.as_ref(), true)
}

fn nullish_digits(input: &mut Input, name: &str) -> Option<Option<i64>> {
    match input.get(name).cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => digits_at(input, &key(name), Some(&value), true).map(Some),
    }
}

/// `z.array(element).min(min).max(max)` at a top-level key.
pub(crate) fn array<T>(
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

fn tag_ids(input: &mut Input) -> Option<Vec<i64>> {
    array(input, "tagIds", 1, 100, |input, path, value| {
        digits_at(input, path, Some(value), true)
    })
}

fn tag_names(input: &mut Input, min: usize) -> Option<Vec<String>> {
    array(input, "tags", min, 100, |input, path, value| {
        input.string_at(path, Some(value), Str::trimmed(1, usize::MAX), true)
    })
}

fn tag_name(input: &mut Input) -> Option<String> {
    input.string("name", Str::trimmed(1, 255))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "tags.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.nullish_string("name", Str::max(usize::MAX)).flatten();
            let folder_id = nullish_digits(&mut input, "folderId").flatten();
            let sort = sort_query(&mut input, Some(&["name", "createdAt", "updatedAt"]));
            input.finish()?;
            let sort = sort.unwrap_or_else(|| {
                vec![SortItem {
                    id: "createdAt".into(),
                    desc: true,
                }]
            });
            let page = service::list(
                db,
                ws,
                ListTags {
                    name,
                    folder_id,
                    page: Some(page),
                    per_page: Some(per_page),
                    sort: Some(sort),
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| {
                    let mut out = tag_json(&row.tag);
                    out["contactsCount"] = json!(row.contacts_count);
                    out
                })
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "tags.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = tag_name(&mut input);
            let folder_id = nullish_digits(&mut input, "folderId").flatten();
            input.finish()?;
            let tag = service::create(db, ws, &name.unwrap_or_default(), folder_id)
                .await
                .map_err(|e| business_error(e, CREATING_IN_FOLDER))?;
            if let Some(jobs) = state.jobs.as_deref() {
                tag_sync::enqueue_create(jobs, ws, tag.id).await;
            }
            Ok(json(201, tag_json(&tag)))
        }
        "tags.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let tag_key = input.string("idOrName", Str::max(usize::MAX));
            input.finish()?;
            service::find_by_key_or_fail(db, ws, &tag_key.unwrap_or_default())
                .await
                .map(|tag| json(200, tag_json(&tag)))
                .map_err(|e| business_error(e, FINDING))
        }
        "tags.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = tag_name(&mut input);
            let folder_id = nullish_digits(&mut input, "folderId");
            let tag_id = digits(&mut input, "id");
            input.finish()?;
            service::update(
                db,
                ws,
                tag_id.unwrap_or_default(),
                &name.unwrap_or_default(),
                folder_id,
            )
            .await
            .map(|tag| json(200, tag_json(&tag)))
            .map_err(|e| business_error(e, MUTATING))
        }
        "tags.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let tag_id = digits(&mut input, "id");
            input.finish()?;
            let deleted = service::soft_delete(db, ws, &[tag_id.unwrap_or_default()])
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            enqueue_tag_deletes(state, ws, &deleted).await;
            Ok(no_content())
        }
        "contacts.listTags" => {
            let contact_id = super::contacts::resolve(state, &call, FINDING).await?;
            let tags = service::list_for_contact(db, ws, contact_id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let data: Vec<Value> = tags.iter().map(tag_json).collect();
            Ok(json(200, json!({ "data": data })))
        }
        "contacts.addTags" | "contacts.removeTags" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = tag_ids(&mut input);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            let ids = ids.unwrap_or_default();
            let applied = route_id == "contacts.addTags";
            let changed = if applied {
                service::attach_to_contact(db, ws, contact_id, &ids).await
            } else {
                service::detach_from_contact(db, ws, contact_id, &ids).await
            }
            .map_err(|e| business_error(e, MUTATING))?;
            // `emitTagApplied` / `emitTagRemoved` per newly linked / removed tag.
            let (added, removed) = if applied {
                (changed, Vec::new())
            } else {
                (Vec::new(), changed)
            };
            emit_tag_changes(state, ws, contact_id, &added, &removed).await;
            Ok(no_content())
        }
        "contacts.addTagsByName" | "contacts.removeTagsByName" | "contacts.setTags" => {
            let mut input = Input::from_body(call.params, call.body);
            let min = usize::from(route_id != "contacts.setTags");
            let names = tag_names(&mut input, min);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            let names = names.unwrap_or_default();
            let before = tag_ids_of(state, ws, contact_id).await?;
            match route_id {
                "contacts.addTagsByName" => {
                    service::attach_by_names_to_contacts(db, ws, &[contact_id], &names)
                        .await
                        .map(|_| ())
                }
                "contacts.removeTagsByName" => {
                    service::detach_by_names_from_contacts(db, ws, &[contact_id], &names)
                        .await
                        .map(|_| ())
                }
                _ => service::replace_contact_tags_by_names(db, ws, contact_id, &names)
                    .await
                    .map(|_| ()),
            }
            .map_err(|e| business_error(e, MUTATING))?;
            let after = tag_ids_of(state, ws, contact_id).await?;
            let added: Vec<i64> = after
                .iter()
                .copied()
                .filter(|id| !before.contains(id))
                .collect();
            let removed: Vec<i64> = before
                .iter()
                .copied()
                .filter(|id| !after.contains(id))
                .collect();
            emit_tag_changes(state, ws, contact_id, &added, &removed).await;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// The tag ids of a contact (before / after a by-name write: the services
/// do not return the changed pairs).
async fn tag_ids_of(
    state: &ApiState,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<i64>, crate::orpc::OrpcError> {
    service::list_for_contact(&state.db, workspace_id, contact_id)
        .await
        .map(|tags| tags.iter().map(|tag| tag.id).collect())
        .map_err(|e| business_error(e, MUTATING))
}

/// `emitTagApplied` for each added tag, then `emitTagRemoved` for each
/// removed one.
async fn emit_tag_changes(
    state: &ApiState,
    workspace_id: i64,
    contact_id: i64,
    added: &[i64],
    removed: &[i64],
) {
    let (ws, contact) = (workspace_id.to_string(), contact_id.to_string());
    let events = added
        .iter()
        .map(|tag| trigger::tag_applied(&ws, &contact, &tag.to_string(), None))
        .chain(
            removed
                .iter()
                .map(|tag| trigger::tag_removed(&ws, &contact, &tag.to_string(), None)),
        );
    state.emit_triggers(events.collect::<Vec<_>>()).await;
    if let Some(jobs) = state.jobs.as_deref() {
        let pairs = |tags: &[i64]| {
            tags.iter()
                .map(|tag| (contact_id, *tag))
                .collect::<Vec<_>>()
        };
        tag_sync::enqueue_attach_many(jobs, workspace_id, &pairs(added)).await;
        tag_sync::enqueue_detach_many(jobs, workspace_id, &pairs(removed)).await;
    }
}

/// `tagSyncService.enqueueDelete` after a soft delete commits.
pub(crate) async fn enqueue_tag_deletes(state: &ApiState, workspace_id: i64, tag_ids: &[i64]) {
    if let Some(jobs) = state.jobs.as_deref() {
        for tag_id in tag_ids {
            tag_sync::enqueue_delete(jobs, workspace_id, *tag_id, None).await;
        }
    }
}
