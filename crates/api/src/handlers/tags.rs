//! `apps/builder/src/features/tags/api/public.ts` (`tagsPublicRouter`) and
//! the tag routes of `features/contacts/api/public/tags.ts`.

use chatbotx_business::tag::{self as service, ListTags};
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
fn digits_at(
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
            service::create(db, ws, &name.unwrap_or_default(), folder_id)
                .await
                .map(|tag| json(201, tag_json(&tag)))
                .map_err(|e| business_error(e, CREATING_IN_FOLDER))
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
            service::soft_delete(db, ws, &[tag_id.unwrap_or_default()])
                .await
                .map(|_| no_content())
                .map_err(|e| business_error(e, MUTATING))
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
            let result = if route_id == "contacts.addTags" {
                service::attach_to_contact(db, ws, contact_id, &ids).await
            } else {
                service::detach_from_contact(db, ws, contact_id, &ids).await
            };
            result
                .map(|_| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "contacts.addTagsByName" | "contacts.removeTagsByName" | "contacts.setTags" => {
            let mut input = Input::from_body(call.params, call.body);
            let min = usize::from(route_id != "contacts.setTags");
            let names = tag_names(&mut input, min);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            let names = names.unwrap_or_default();
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
            .map(|()| no_content())
            .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
