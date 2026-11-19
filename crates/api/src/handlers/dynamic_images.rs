//! `features/dynamic-images/api/public.ts`: `dynamicImages.*` (scope `media`),
//! plus the request checks and resources the `/rpc` actions share.

use chatbotx_business::BusinessError;
use chatbotx_business::dynamic_image::{self as service, Deps, ListInput, Upsert};
use chatbotx_db::repo::models::DynamicImage;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content, number, opt_id,
};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "dynamicImages.list",
    "dynamicImages.create",
    "dynamicImages.get",
    "dynamicImages.update",
    "dynamicImages.delete",
    "dynamicImages.setEnabled",
];

const IMAGE_TYPES: &[&str] = &["url", "avatarUser", "customField"];
const IMAGE_STYLES: &[&str] = &["square", "circle"];
const FONT_FAMILIES: &[&str] = &["arial", "serif", "roboto", "greatVibes"];
const ALIGNS: &[&str] = &["left", "center", "right"];

/// The file store of a save / render; 500 without one.
pub fn files(state: &ApiState) -> Result<&chatbotx_files::Uploader, OrpcError> {
    state
        .files
        .as_deref()
        .ok_or_else(|| super::internal_error("File storage is not configured"))
}

/// The trigger URL a resource publishes (`imageUrl`).
pub fn trigger_url(state: &ApiState, image_id: i64) -> String {
    format!(
        "{}/dynamic-images?dynamicImageId={image_id}&userId={{{{user_id}}}}",
        state.app_url
    )
}

/// `dynamicImageResource` (the builder's own reads: workspace id, raw key).
pub fn resource_json(row: &DynamicImage) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "name": row.name,
        "workspaceId": id(row.workspace_id),
        "customFieldId": opt_id(row.custom_field_id),
        "data": row.data.0,
        "backgroundUrl": row.background_url,
        "enabled": row.enabled,
    })
}

/// `publicDynamicImageResource` (`toPublicResources`).
pub fn public_json(state: &ApiState, storage_url: &str, row: &DynamicImage) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "name": row.name,
        "customFieldId": opt_id(row.custom_field_id),
        "data": row.data.0,
        "backgroundUrl": service::background_url(row, storage_url),
        "enabled": row.enabled,
        "imageUrl": trigger_url(state, row.id),
    })
}

/// `createDynamicImageRequest` (= the update body): `(name, customFieldId, data)`.
pub fn upsert_fields(input: &mut Input) -> Option<(String, Option<i64>, Value)> {
    let name = input.string(
        "name",
        Str {
            min: Some(1),
            max: Some(100),
            ..Str::default()
        },
    );
    let custom_field_id = match input.get("customFieldId").cloned() {
        None | Some(Value::Null) => Some(None),
        Some(value) => digits(input, &key("customFieldId"), &value).map(Some),
    };
    let data = input.get("data").cloned();
    let data = document(input, &key("data"), data.as_ref());
    Some((name?, custom_field_id?, data?))
}

/// `zodBigintAsString()` (`/^\d+$/`, no length cap): the id.
fn digits(input: &mut Input, path: &[Seg], value: &Value) -> Option<i64> {
    let text = input.string_at(path, Some(value), Str::default(), true)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            path,
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    // A longer one cannot name a row: the lookup finds nothing.
    Some(text.parse().unwrap_or(i64::MAX))
}

/// `z.number().int().positive()`.
fn positive_int(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<f64> {
    let n = input.number_at(
        path,
        value,
        Num {
            int: true,
            ..Num::default()
        },
        true,
    )?;
    if n <= 0.0 {
        input.issue(path, "too_small", "Too small: expected number to be >0");
        return None;
    }
    Some(n)
}

/// `dynamicImageDocument` with its defaults applied.
fn document(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<Value> {
    let Some(Value::Object(obj)) = value else {
        match value {
            None => input.issue(
                path,
                "invalid_type",
                "Invalid input: expected object, received undefined",
            ),
            Some(other) => input.issue(
                path,
                "invalid_type",
                &format!(
                    "Invalid input: expected object, received {}",
                    super::input::type_name(Some(other))
                ),
            ),
        }
        return None;
    };
    let width = positive_int(input, &child(path, "width"), obj.get("width"));
    let height = positive_int(input, &child(path, "height"), obj.get("height"));
    let elements_path = child(path, "elements");
    let elements = match obj.get("elements") {
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            let mut ok = true;
            for (index, item) in items.iter().enumerate() {
                let mut at = elements_path.clone();
                at.push(Seg::Index(index));
                match element(input, &at, item) {
                    Some(element) => out.push(element),
                    None => ok = false,
                }
            }
            ok.then_some(out)
        }
        other => {
            input.issue(
                &elements_path,
                "invalid_type",
                &format!(
                    "Invalid input: expected array, received {}",
                    super::input::type_name(other)
                ),
            );
            None
        }
    };
    Some(json!({
        "width": number(width?),
        "height": number(height?),
        "elements": elements?,
    }))
}

fn element(input: &mut Input, path: &Path, value: &Value) -> Option<Value> {
    let Value::Object(obj) = value else {
        input.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected object, received {}",
                super::input::type_name(Some(value))
            ),
        );
        return None;
    };
    let kind = obj.get("type").and_then(Value::as_str);
    if !matches!(kind, Some("image" | "qrCode" | "text")) {
        input.issue(&child(path, "type"), "invalid_union", "Invalid input");
        return None;
    }
    let mut out = Map::new();
    let before = input.issue_count();
    let id_value = input.string_at(&child(path, "id"), obj.get("id"), Str::default(), true);
    out.insert("id".into(), json!(id_value));
    for field in ["x", "y", "width", "height"] {
        let n = input.number_at(&child(path, field), obj.get(field), Num::default(), true);
        out.insert(field.into(), n.map_or(Value::Null, number));
    }
    let priority = opt_bool(input, path, obj, "priority");
    out.insert("priority".into(), json!(priority));
    out.insert("type".into(), json!(kind));
    let opt_string = |input: &mut Input, out: &mut Map<String, Value>, field: &str| {
        if let Some(text) =
            input.string_at(&child(path, field), obj.get(field), Str::default(), false)
        {
            out.insert(field.into(), json!(text));
        }
    };
    let string = |input: &mut Input, out: &mut Map<String, Value>, field: &str| {
        let text = input.string_at(&child(path, field), obj.get(field), Str::default(), true);
        out.insert(field.into(), json!(text));
    };
    match kind {
        Some("image") => {
            let t = input.enum_at(
                &child(path, "imageType"),
                obj.get("imageType"),
                IMAGE_TYPES,
                true,
            );
            out.insert("imageType".into(), json!(t));
            opt_string(input, &mut out, "url");
            opt_string(input, &mut out, "customFieldId");
            let style = match obj.get("imageStyle") {
                None => Some("square".to_owned()),
                value => input.enum_at(&child(path, "imageStyle"), value, IMAGE_STYLES, true),
            };
            out.insert("imageStyle".into(), json!(style));
        }
        Some("qrCode") => {
            string(input, &mut out, "text");
            let size = input.number_at(
                &child(path, "size"),
                obj.get("size"),
                Num::int(64.0, 1024.0),
                true,
            );
            out.insert("size".into(), size.map_or(Value::Null, number));
            string(input, &mut out, "color");
            opt_string(input, &mut out, "logoUrl");
        }
        _ => {
            string(input, &mut out, "text");
            let size = input.number_at(
                &child(path, "fontSize"),
                obj.get("fontSize"),
                Num::default(),
                true,
            );
            out.insert("fontSize".into(), size.map_or(Value::Null, number));
            let family = input.enum_at(
                &child(path, "fontFamily"),
                obj.get("fontFamily"),
                FONT_FAMILIES,
                true,
            );
            out.insert("fontFamily".into(), json!(family));
            let align = input.enum_at(&child(path, "align"), obj.get("align"), ALIGNS, true);
            out.insert("align".into(), json!(align));
            string(input, &mut out, "color");
            for flag in ["bold", "italic", "uppercase"] {
                let value = opt_bool(input, path, obj, flag);
                out.insert(flag.into(), json!(value));
            }
        }
    }
    (input.issue_count() == before).then_some(Value::Object(out))
}

/// `z.boolean().default(false)`.
fn opt_bool(input: &mut Input, path: &[Seg], obj: &Map<String, Value>, field: &str) -> bool {
    input
        .bool_at(&child(path, field), obj.get(field), false)
        .unwrap_or(false)
}

fn image_id(input: &mut Input) -> Option<i64> {
    let value = input.get("id").cloned();
    value
        .as_ref()
        .map(|v| digits(input, &key("id"), v))
        .unwrap_or_else(|| {
            input.issue(
                &key("id"),
                "invalid_type",
                "Invalid input: expected string, received undefined",
            );
            None
        })
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let storage = super::contacts::tenant_urls(state).storage_url;
    let deps = || -> Result<Deps<'_>, OrpcError> {
        Ok(Deps {
            db,
            files: files(state)?,
            storage_url: &storage,
        })
    };
    match route_id {
        "dynamicImages.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            let name = input.opt_string("name", Str::default());
            input.finish()?;
            let page_out = service::list(
                db,
                ListInput {
                    workspace_id: ws,
                    page: Some(page),
                    per_page: Some(per_page),
                    name,
                    sort: None,
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            Ok(json(
                200,
                json!({
                    "data": page_out.data.iter().map(|r| public_json(state, &storage, r)).collect::<Vec<_>>(),
                    "pageCount": page_out.page_count,
                }),
            ))
        }
        "dynamicImages.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let image = image_id(&mut input);
            input.finish()?;
            let row = service::find(db, ws, image.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, public_json(state, &storage, &row)))
        }
        "dynamicImages.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let fields = upsert_fields(&mut input);
            input.finish()?;
            let (name, custom_field_id, data) = fields.unwrap_or_default();
            let row = service::create(
                deps()?,
                Upsert {
                    workspace_id: ws,
                    name,
                    custom_field_id,
                    data,
                },
            )
            .await
            .map_err(|e| business_error(e, CREATING))?;
            Ok(json(201, public_json(state, &storage, &row)))
        }
        "dynamicImages.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let fields = upsert_fields(&mut input);
            let image = image_id(&mut input);
            input.finish()?;
            let (name, custom_field_id, data) = fields.unwrap_or_default();
            let row = service::update(
                deps()?,
                image.unwrap_or_default(),
                Upsert {
                    workspace_id: ws,
                    name,
                    custom_field_id,
                    data,
                },
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, public_json(state, &storage, &row)))
        }
        "dynamicImages.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let image = image_id(&mut input);
            input.finish()?;
            service::delete(deps()?, ws, image.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "dynamicImages.setEnabled" => {
            let mut input = Input::from_body(call.params, call.body);
            let image = image_id(&mut input);
            let enabled = input.bool("enabled");
            input.finish()?;
            let row = service::set_enabled(
                db,
                ws,
                image.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, public_json(state, &storage, &row)))
        }
        _ => Err(super::internal_error("unknown dynamic image route")),
    }
}

/// A unique `(workspaceId, name)` violation (`isUniqueViolationError`).
pub fn is_duplicate_name(error: &BusinessError) -> bool {
    matches!(error, BusinessError::Repo(e) if chatbotx_db::repo::is_unique_violation_error(e, None))
}
