//! `features/qr-codes/api/public.ts`: `qrCodes.*`.

use chatbotx_business::qr_code::{self as service, DEFAULT_SIZE, ListQrCodes, QrCodeData};
use chatbotx_db::repo::models::Reflink;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Str};
use super::{ApiState, Call, Reply, business_error, date, id, json, no_content, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "qrCodes.list",
    "qrCodes.create",
    "qrCodes.get",
    "qrCodes.update",
    "qrCodes.delete",
];

const DUPLICATE_NAME: &str = "QR Code name already exists";
/// `QR_CODE_SIZE.MIN` / `.MAX`.
const SIZE: Num = Num::int(64.0, 1024.0);

/// `qrCodeResource` without `workspaceId`.
pub fn qr_code_json(row: &Reflink) -> Map<String, Value> {
    let Value::Object(out) = json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "name": row.name,
        "type": row.r#type,
        "flowId": id(row.flow_id),
        "customFieldId": opt_id(row.custom_field_id),
        "qrStyles": row.qr_styles.as_ref().map(|s| s.0.clone()),
    }) else {
        unreachable!("json! object")
    };
    out
}

/// `/^[a-zA-Z0-9]+$/` names (`createQrCodeRequest`, `createReflinkRequest`).
pub fn alnum_name(input: &mut Input, required: bool) -> Option<String> {
    let spec = Str {
        trim: false,
        min: Some(1),
        max: Some(50),
        pattern: None,
    };
    let name = if required {
        input.string("name", spec)
    } else {
        input.opt_string("name", spec)
    }?;
    if !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        input.issue(&super::input::key("name"), "custom", "Invalid input");
        return None;
    }
    Some(name)
}

/// `sort[i][id]` / `sort[i][desc]` query keys (`[{ id, desc }]`); `allowed`
/// limits the ids (`publicSortRequest`).
pub fn sort_query(
    input: &mut Input,
    allowed: Option<&[&str]>,
) -> Option<Vec<chatbotx_db::repo::SortItem>> {
    let mut items: Vec<(usize, Option<String>, Option<String>)> = Vec::new();
    let keys: Vec<(String, Value)> = input
        .obj
        .iter()
        .filter(|(k, _)| k.starts_with("sort["))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (key, value) in keys {
        let rest = &key["sort[".len()..];
        let Some((index, field)) = rest.split_once("][") else {
            continue;
        };
        let (Ok(index), Some(field)) = (index.parse::<usize>(), field.strip_suffix(']')) else {
            continue;
        };
        let text = value.as_str().unwrap_or_default().to_string();
        let slot = match items.iter_mut().position(|(i, _, _)| *i == index) {
            Some(at) => at,
            None => {
                items.push((index, None, None));
                items.len() - 1
            }
        };
        match field {
            "id" => items[slot].1 = Some(text),
            "desc" => items[slot].2 = Some(text),
            _ => {}
        }
    }
    if items.is_empty() {
        return None;
    }
    items.sort_by_key(|(i, _, _)| *i);
    let mut out = Vec::new();
    for (index, sort_id, desc) in items {
        let at = |field: &str| {
            vec![
                super::input::Seg::Key("sort".into()),
                super::input::Seg::Index(index),
                super::input::Seg::Key(field.into()),
            ]
        };
        let sort_id = match sort_id {
            Some(value) if allowed.is_none_or(|list| list.contains(&value.as_str())) => Some(value),
            _ => {
                input.issue(&at("id"), "invalid_value", "Invalid option");
                None
            }
        };
        let desc = match desc.as_deref() {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => {
                input.issue(
                    &at("desc"),
                    "invalid_type",
                    "Invalid input: expected boolean",
                );
                None
            }
        };
        if let (Some(id), Some(desc)) = (sort_id, desc) {
            out.push(chatbotx_db::repo::SortItem { id, desc });
        }
    }
    Some(out)
}

/// `publicListRequest`: `page` (default 1), `perPage` (1–50, default 50).
pub fn public_paging(input: &mut Input) -> (i64, i64) {
    let page = input.opt_coerce_int(
        "page",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
    (page.unwrap_or(1), per_page.unwrap_or(50))
}

fn size(input: &mut Input) -> Option<i64> {
    let value = input.get("size").cloned()?;
    input
        .coerce_number_at(&super::input::key("size"), Some(&value), SIZE, true)
        .map(|n| n as i64)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "qrCodes.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let keyword = input.opt_string("keyword", Str::default());
            let sort = sort_query(&mut input, None);
            input.finish()?;
            let page = service::list(
                db,
                ListQrCodes {
                    workspace_id: ws,
                    page: Some(page),
                    per_page,
                    sort,
                    keyword,
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| {
                    let mut out = qr_code_json(&row.qr_code);
                    out.insert(
                        "flow".into(),
                        json!({ "id": id(row.flow.id), "name": row.flow.name }),
                    );
                    Value::Object(out)
                })
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "qrCodes.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let qr_id = input.id("id");
            input.finish()?;
            service::find_or_fail(db, ws, qr_id.unwrap_or_default())
                .await
                .map(|row| json(200, Value::Object(qr_code_json(&row))))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "qrCodes.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = alnum_name(&mut input, true);
            let flow_id = input.id("flowId");
            let size = size(&mut input).or(Some(DEFAULT_SIZE));
            input.finish()?;
            let data = QrCodeData {
                name,
                flow_id,
                custom_field_id: None,
                size,
            };
            service::create(db, ws, data, DUPLICATE_NAME)
                .await
                .map(|new_id| json(201, json!({ "id": id(new_id) })))
                .map_err(|e| business_error(e, super::CREATING))
        }
        "qrCodes.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let qr_id = input.id("id");
            let name = alnum_name(&mut input, false);
            let flow_id = if input.has("flowId") {
                input.id("flowId")
            } else {
                None
            };
            let size = size(&mut input);
            input.finish()?;
            let data = QrCodeData {
                name,
                flow_id,
                custom_field_id: None,
                size,
            };
            service::update(db, ws, qr_id.unwrap_or_default(), data, DUPLICATE_NAME)
                .await
                .map(|row| json(200, Value::Object(qr_code_json(&row))))
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "qrCodes.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let qr_id = input.id("id");
            input.finish()?;
            service::delete_many(db, ws, vec![qr_id.unwrap_or_default()])
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
