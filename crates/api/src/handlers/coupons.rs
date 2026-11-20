//! `features/coupons/api/public.ts`: coupon topics, their coupons, issuing
//! one to a contact and marking it used.

use chatbotx_business::contact::service::resolve_id_by_identifier;
use chatbotx_business::coupon::{self, CouponFilter};
use serde_json::{Value, json};

use super::broadcasts::coerce_date;
use super::input::{Input, Num, Seg, Str, key};
use super::{ApiState, Call, ErrorMap, Reply, business_error, json};
use crate::orpc::OrpcError;

/// The routes of this module. The import and the export run in the request
/// (the contact export precedent), not on a worker queue.
pub const ROUTES: &[&str] = &[
    "coupons.listTopics",
    "coupons.createTopic",
    "coupons.getTopic",
    "coupons.updateTopic",
    "coupons.deleteTopic",
    "coupons.archiveTopic",
    "coupons.unarchiveTopic",
    "coupons.listCoupons",
    "coupons.issueCoupon",
    "coupons.markCouponUsed",
    "coupons.listContactCoupons",
    "coupons.bulkCreate",
    "coupons.createImportUploadUrl",
    "coupons.import",
    "coupons.export",
    "coupons.getExport",
];

/// The `couponImportErrors` a route declares.
const IMPORTING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("couponImportFileNotFound", 404),
    ("couponImportUnsupportedFile", 400),
    ("couponImportFileTooLarge", 400),
    ("couponTopicInactive", 400),
];

/// `possibleErrorsOnMutatingResource` + the bulk refusals.
const BULK: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("couponTopicInactive", 400),
    ("couponImportLimitExceeded", 400),
];

const ISSUE_STATUSES: &[&str] = &["published", "unpublished"];
const USAGE_STATUSES: &[&str] = &["used", "notUsed"];

/// `withPublicPaging`: `page` >= 1 (1), `perPage` 1..=50 (50).
fn paging(input: &mut Input) -> (i64, i64) {
    let page = input.opt_coerce_int("page", Num::int(1.0, f64::MAX));
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
    (page.unwrap_or(1), per_page.unwrap_or(50))
}

fn opt_enum(input: &mut Input, name: &str, options: &[&str]) -> Option<String> {
    let value = input.get(name).cloned()?;
    input.enum_at(&key(name), Some(&value), options, true)
}

fn path_id(call: &Call<'_>, name: &str) -> Result<i64, OrpcError> {
    let mut input = Input::from_query(call.params, None);
    let id = input.digits_id(name);
    input.finish()?;
    Ok(id.unwrap_or_default())
}

/// `z.coerce.date().optional().nullable()`: `None` absent, `Some(None)` null.
fn expires_at(input: &mut Input) -> Option<Option<i64>> {
    match input.get("expiresAt").cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => match coerce_date(&value) {
            Some(at) => Some(Some(at)),
            None => {
                input.issue(
                    &key("expiresAt"),
                    "invalid_type",
                    "Invalid input: expected date, received Date",
                );
                None
            }
        },
    }
}

/// `createCouponTopicPublicRequest`.
fn topic_input(input: &mut Input) -> (Option<String>, Option<Option<String>>, Option<Option<i64>>) {
    let name = input.string("name", Str::trimmed(1, 255));
    let description = input.nullish_string("description", Str::trimmed(0, 1000));
    let expires = expires_at(input);
    (name, description, expires)
}

/// An undeclared refusal of the route (`new ORPCError(code, {message, data})`).
fn undeclared(error: chatbotx_business::BusinessError, codes: &[&str]) -> OrpcError {
    if let chatbotx_business::BusinessError::Exception {
        code,
        message,
        data,
        ..
    } = &error
        && codes.contains(&code.as_str())
    {
        let mut out = OrpcError::new(code, Some(500), Some(message));
        out.defined = false;
        out.data = data.clone();
        return out;
    }
    business_error(error, super::MUTATING)
}

/// `{ id, contactId }` of issue / mark-used.
fn contact_target(call: &Call<'_>) -> Result<(i64, i64), OrpcError> {
    let mut input = Input::from_body(call.params, call.body);
    let id = input.digits_id("id");
    let contact = input.digits_id("contactId");
    input.finish()?;
    Ok((id.unwrap_or_default(), contact.unwrap_or_default()))
}

/// `bulkCreateCouponsPublicRequest.codes`.
fn codes(input: &mut Input) -> Vec<String> {
    let Some(value) = input.get("codes").cloned() else {
        input.issue(
            &key("codes"),
            "invalid_type",
            "Invalid input: expected array, received undefined",
        );
        return Vec::new();
    };
    let Value::Array(list) = value else {
        input.issue(
            &key("codes"),
            "invalid_type",
            "Invalid input: expected array",
        );
        return Vec::new();
    };
    if list.is_empty() {
        input.issue(
            &key("codes"),
            "too_small",
            "Too small: expected array to have >=1 items",
        );
    } else if list.len() > 10_000 {
        input.issue(
            &key("codes"),
            "too_big",
            "Too big: expected array to have <=10000 items",
        );
    }
    let mut out = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        let path = vec![Seg::Key("codes".into()), Seg::Index(index)];
        if let Some(code) = input.string_at(&path, Some(item), Str::trimmed(1, 255), true) {
            out.push(code);
        }
    }
    out
}

/// `publicListCouponsRequest`: the new names or their legacy aliases.
fn coupon_filter(input: &mut Input) -> CouponFilter {
    let topic_id = if input.has("topicId") {
        input.digits_id("topicId")
    } else {
        None
    };
    let status = opt_enum(input, "status", ISSUE_STATUSES);
    let usage = opt_enum(input, "usage", USAGE_STATUSES);
    let keyword = input.opt_string("keyword", Str::default());
    let issue_status = opt_enum(input, "issueStatus", ISSUE_STATUSES);
    let usage_status = opt_enum(input, "usageStatus", USAGE_STATUSES);
    let search = input.opt_string("search", Str::default());
    for (canonical, legacy) in [
        ("status", "issueStatus"),
        ("usage", "usageStatus"),
        ("keyword", "search"),
    ] {
        if input.has(canonical) && input.has(legacy) {
            input.issue(
                &key(canonical),
                "custom",
                &format!("Use either `{canonical}` or legacy `{legacy}`, not both."),
            );
        }
    }
    CouponFilter {
        topic_id,
        issue_status: status.or(issue_status),
        usage_status: usage.or(usage_status),
        search: keyword.or(search),
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let db = &state.db;
    let topic = |t: &chatbotx_db::repo::models::CouponTopic| Ok(json(200, coupon::topic_json(t)));
    match route_id {
        "coupons.listTopics" => {
            let mut input = Input::from_query(call.params, call.query);
            let archived = input.opt_coerced_bool("archived");
            let search = input.opt_string("search", Str::default());
            let (page, per_page) = paging(&mut input);
            input.finish()?;
            let (data, page_count) =
                coupon::list_topics(db, ws, archived.unwrap_or(false), search, page, per_page)
                    .await
                    .map_err(|e| business_error(e, super::LISTING))?;
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "coupons.getTopic" => {
            let id = path_id(&call, "id")?;
            let row = coupon::get_topic(db, ws, id)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            topic(&row)
        }
        "coupons.createTopic" => {
            let mut input = Input::from_body(call.params, call.body);
            let (name, description, expires) = topic_input(&mut input);
            input.finish()?;
            let row = coupon::create_topic(
                db,
                ws,
                name.unwrap_or_default(),
                description.flatten(),
                expires.flatten(),
            )
            .await
            .map_err(|e| business_error(e, super::CREATING))?;
            topic(&row)
        }
        "coupons.updateTopic" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let (name, description, expires) = topic_input(&mut input);
            input.finish()?;
            let row =
                coupon::update_topic(db, ws, id.unwrap_or_default(), name, description, expires)
                    .await
                    .map_err(|e| business_error(e, super::MUTATING))?;
            topic(&row)
        }
        "coupons.archiveTopic" | "coupons.unarchiveTopic" => {
            let id = path_id(&call, "id")?;
            let status = if route_id == "coupons.archiveTopic" {
                "archived"
            } else {
                "active"
            };
            let row = coupon::set_topic_status(db, ws, id, status)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            topic(&row)
        }
        "coupons.deleteTopic" => {
            let id = path_id(&call, "id")?;
            let row = coupon::delete_topic(db, ws, id)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            topic(&row)
        }
        "coupons.listCoupons" => {
            let mut input = Input::from_query(call.params, call.query);
            let filter = coupon_filter(&mut input);
            let (page, per_page) = paging(&mut input);
            input.finish()?;
            let (data, page_count) = coupon::list_coupons(db, ws, filter, page, per_page)
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "coupons.issueCoupon" => {
            let (id, contact) = contact_target(&call)?;
            let (row, _) = coupon::issue(db, ws, id, contact)
                .await
                .map_err(|e| undeclared(e, &["couponIssueUnavailable"]))?;
            Ok(json(200, coupon::coupon_json(&row)))
        }
        "coupons.markCouponUsed" => {
            let (id, contact) = contact_target(&call)?;
            let row = coupon::mark_used(db, ws, id, contact)
                .await
                .map_err(|e| undeclared(e, &["couponNotIssued"]))?;
            Ok(json(200, coupon::coupon_json(&row)))
        }
        "coupons.listContactCoupons" => {
            let identifier = call
                .params
                .iter()
                .find(|(name, _)| *name == "identifier")
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            let contact_id = resolve_id_by_identifier(db, ws, &identifier)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            let data = coupon::list_for_contact(db, ws, contact_id)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(200, json!({ "data": data })))
        }
        "coupons.createImportUploadUrl" => {
            let mut input = Input::from_body(call.params, call.body);
            let file_name = input.string("fileName", Str::trimmed(1, 255));
            if file_name
                .as_deref()
                .is_some_and(|n| !n.to_lowercase().ends_with(".csv"))
            {
                input.issue(
                    &key("fileName"),
                    "custom",
                    "Coupon import files must use the .csv extension",
                );
            }
            if input.get("mimeType").and_then(Value::as_str) != Some("text/csv") {
                input.issue(
                    &key("mimeType"),
                    "invalid_value",
                    "Invalid input: expected \"text/csv\"",
                );
            }
            let size = input.number("size", Num::int(1.0, 10.0 * 1024.0 * 1024.0));
            input.finish()?;
            let files = state
                .files
                .as_deref()
                .ok_or_else(|| super::internal_error("file storage is not configured"))?;
            let storage_url = super::contacts::tenant_urls(state).storage_url;
            let upload = chatbotx_business::import::create_import_upload(
                db,
                files,
                &storage_url,
                chatbotx_business::import::CreateImportUpload {
                    workspace_id: ws,
                    user_id: None,
                    import_type: chatbotx_business::import::ImportType::Coupons,
                    file_name: file_name.unwrap_or_default(),
                    mime_type: "text/csv".into(),
                    file_size: size.unwrap_or_default() as i64,
                },
            )
            .await
            .map_err(|e| business_error(e, super::CREATING))?;
            Ok(json(
                201,
                json!({ "fileId": upload.file_id.to_string(), "uploadUrl": upload.presigned_post_url }),
            ))
        }
        "coupons.import" => {
            let mut input = Input::from_body(call.params, call.body);
            let topic_id = input.digits_id("topicId");
            let file_id = input.digits_id("fileId");
            input.finish()?;
            let files = state
                .files
                .as_deref()
                .ok_or_else(|| super::internal_error("file storage is not configured"))?;
            let import_id = chatbotx_business::import::coupons::start(
                db,
                files,
                ws,
                None,
                file_id.unwrap_or_default(),
                topic_id.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, IMPORTING))?;
            if let Err(error) =
                chatbotx_business::import::contacts::run_import(db, files, import_id).await
            {
                tracing::error!(%error, import_id, "coupon import run failed");
            }
            Ok(json(201, json!({ "importId": import_id.to_string() })))
        }
        "coupons.export" => {
            let mut input = Input::from_body(call.params, call.body);
            let topic_id = if input.has("topicId") {
                input.digits_id("topicId")
            } else {
                None
            };
            let issue_status = opt_enum(&mut input, "issueStatus", ISSUE_STATUSES);
            let usage_status = opt_enum(&mut input, "usageStatus", USAGE_STATUSES);
            let search = input.opt_string("search", Str::trimmed(1, 255));
            input.finish()?;
            let filter = CouponFilter {
                topic_id,
                issue_status,
                usage_status,
                search,
            };
            let file_id = coupon::export(db, state.files.as_deref(), ws, filter)
                .await
                .map_err(|e| business_error(e, super::CREATING))?;
            Ok(json(201, json!({ "fileId": file_id.to_string() })))
        }
        "coupons.getExport" => {
            let file_id = path_id(&call, "fileId")?;
            let file = coupon::get_export(db, state.files.as_deref(), ws, file_id)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(200, file))
        }
        _ => {
            let mut input = Input::from_body(call.params, call.body);
            let topic_id = input.digits_id("topicId");
            let codes = codes(&mut input);
            input.finish()?;
            let result = coupon::import_batch(db, ws, topic_id.unwrap_or_default(), codes)
                .await
                .map_err(|e| business_error(e, BULK))?;
            Ok(json(200, result))
        }
    }
}
