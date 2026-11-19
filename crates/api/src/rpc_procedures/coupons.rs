//! Coupons: `features/coupons/api/private.ts` (`couponsAPI.*`) and
//! `features/coupons/actions/*` (Q.F2-automation). The import and the export
//! run in the request (the public `coupons.import` / `coupons.export`
//! precedent), not on a worker queue.

use chatbotx_business::BusinessError;
use chatbotx_business::coupon::{self as service, CouponFilter};
use chatbotx_db::repo::utils::{PaginationInput, get_pagination_with_defaults};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid};
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &["exportCouponsAction", "importCouponsAction"];

const ISSUE_STATUSES: &[&str] = &["published", "unpublished"];
const USAGE_STATUSES: &[&str] = &["used", "notUsed"];

/// `page` / `perPage` (`z.number().int().positive().optional()`) through
/// `getPaginationWithDefaults`: `(page, limit)`.
fn paging(input: &In<'_>) -> Result<(i64, i64), OrpcError> {
    let page = input.opt_positive("page")?;
    let per_page = input.opt_positive("perPage")?;
    let pagination = get_pagination_with_defaults(PaginationInput { page, per_page });
    Ok((pagination.offset / pagination.limit + 1, pagination.limit))
}

/// A nullish enum: `null` / absent = `None`, an unknown value = 400.
fn opt_enum(input: &In<'_>, key: &str, values: &[&str]) -> Result<Option<String>, OrpcError> {
    match input.opt_str(key)? {
        None => Ok(None),
        Some(value) if values.contains(&value.as_str()) => Ok(Some(value)),
        Some(_) => Err(invalid()),
    }
}

/// `listCouponsRequest` / `exportCouponRequest` filters.
fn filter(input: &In<'_>) -> Result<CouponFilter, OrpcError> {
    Ok(CouponFilter {
        topic_id: input.opt_id("topicId")?,
        issue_status: opt_enum(input, "issueStatus", ISSUE_STATUSES)?,
        usage_status: opt_enum(input, "usageStatus", USAGE_STATUSES)?,
        search: input.opt_str("search")?,
    })
}

/// `expiresAt: z.coerce.date().optional().nullable()`: absent = `None`,
/// `null` = `Some(None)`.
fn expires_at(input: &In<'_>) -> Result<Option<Option<i64>>, OrpcError> {
    match input.0.get("expiresAt") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => crate::handlers::broadcasts::coerce_date(value)
            .map(|ms| Some(Some(ms)))
            .ok_or_else(invalid),
    }
}

/// `createCouponTopicRequest` (`name` trimmed 1..=255, `description` ≤ 1000).
fn topic_fields(input: &In<'_>) -> Result<(String, Option<Option<String>>), OrpcError> {
    let name = input.str("name", true, 1, 255)?;
    let description = match input.0.get("description") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(text)) if text.trim().chars().count() <= 1000 => {
            Some(Some(text.trim().to_owned()))
        }
        Some(_) => return Err(invalid()),
    };
    Ok((name, description))
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let name = path.strip_prefix("couponsAPI.")?;
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let input = In(input);
    let topic = |topic: chatbotx_db::repo::models::CouponTopic| service::private_topic_json(&topic);
    Some(
        async {
            match name {
                "listCouponTopicsAPI" => {
                    let (page, per_page) = paging(&input)?;
                    let desc = input.sort().is_some_and(|sort| {
                        sort.iter().find(|s| s.id == "name").is_some_and(|s| s.desc)
                    });
                    let archived = input.opt_bool("archived")?.unwrap_or(false);
                    let out = service::list_topic_rows(
                        db,
                        ws,
                        archived,
                        input.opt_str("search")?,
                        page,
                        per_page,
                        desc,
                    )
                    .await
                    .map_err(fail)?;
                    let data: Vec<Value> = out
                        .rows
                        .iter()
                        .map(|(t, coupons)| {
                            let mut row = service::private_topic_json(t);
                            row["couponCount"] = json!(coupons);
                            row
                        })
                        .collect();
                    Ok(json!({ "data": data, "pageCount": out.page_count, "total": out.total }))
                }
                "createCouponTopicAPI" => {
                    let (topic_name, description) = topic_fields(&input)?;
                    service::create_topic_by(
                        db,
                        ws,
                        Some(ctx.user_id),
                        topic_name,
                        description.flatten(),
                        expires_at(&input)?.flatten(),
                    )
                    .await
                    .map(topic)
                    .map_err(fail)
                }
                "updateCouponTopicAPI" => {
                    let topic_id = input.id("topicId")?;
                    let (topic_name, description) = topic_fields(&input)?;
                    service::update_topic(
                        db,
                        ws,
                        topic_id,
                        Some(topic_name),
                        description,
                        expires_at(&input)?,
                    )
                    .await
                    .map(topic)
                    .map_err(fail)
                }
                "archiveCouponTopicAPI" | "unarchiveCouponTopicAPI" => {
                    let status = if name == "archiveCouponTopicAPI" {
                        "archived"
                    } else {
                        "active"
                    };
                    service::set_topic_status(db, ws, input.id("topicId")?, status)
                        .await
                        .map(topic)
                        .map_err(fail)
                }
                "deleteCouponTopicAPI" => service::delete_topic(db, ws, input.id("topicId")?)
                    .await
                    .map(topic)
                    .map_err(fail),
                "listCouponsAPI" => {
                    let (page, per_page) = paging(&input)?;
                    let (data, page_count, total) =
                        service::list_coupons_page(db, ws, filter(&input)?, page, per_page)
                            .await
                            .map_err(fail)?;
                    Ok(json!({ "data": data, "pageCount": page_count, "total": total }))
                }
                "listCouponTopicOptionsAPI" => service::list_active_topic_options(
                    db,
                    ws,
                    input.opt_str("keyword")?,
                    input.opt_bool("issueableOnly")?.unwrap_or(false),
                )
                .await
                .map(Value::Array)
                .map_err(fail),
                "countCouponExportAPI" => {
                    let count = service::count_coupons(db, ws, filter(&input)?)
                        .await
                        .map_err(fail)?;
                    Ok(json!({ "count": count }))
                }
                // The signed-in user's export (`getCouponExportFile`).
                "getCouponExportFileAPI" => service::get_export_for(
                    db,
                    ctx.api.files.as_deref(),
                    ws,
                    Some(ctx.user_id),
                    input.id("fileId")?,
                )
                .await
                .map_err(fail),
                // `requireContactPermissionScopeForMember`; an assigned-only
                // member reads only a contact assigned to them.
                "listContactCouponsAPI" => {
                    let contact_id = input.id("contactId")?;
                    let scope = ctx.require_contact_scope()?;
                    let visible = super::contacts::scoped_ids(ctx, scope, vec![contact_id]).await?;
                    if visible.is_empty() {
                        return Err(fail(BusinessError::not_found("Contact not found")));
                    }
                    service::list_for_contact(db, ws, contact_id)
                        .await
                        .map(Value::Array)
                        .map_err(fail)
                }
                _ => Err(OrpcError::new(
                    "NOT_FOUND",
                    Some(404),
                    Some("Operation not ported yet"),
                )),
            }
        }
        .await,
    )
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let input = In(arg(args, 1));
    Some(
        async {
            if name == "exportCouponsAction" {
                // [workspaceId], exportCouponRequest
                let file_id = service::export_for(
                    db,
                    ctx.api.files.as_deref(),
                    ws,
                    Some(ctx.user_id),
                    filter(&input)?,
                )
                .await
                .map_err(action_fail)?;
                return Ok(json!({ "fileId": file_id.to_string() }));
            }
            // [workspaceId], { topicId, fileId }
            let topic_id = input.id("topicId")?;
            let file_id = input.id("fileId")?;
            let files =
                ctx.api.files.as_deref().ok_or_else(|| {
                    crate::handlers::internal_error("file storage is not configured")
                })?;
            let import_id = chatbotx_business::import::coupons::start(
                db,
                files,
                ws,
                Some(ctx.user_id),
                file_id,
                topic_id,
            )
            .await
            .map_err(action_fail)?;
            if let Err(error) =
                chatbotx_business::import::contacts::run_import(db, files, import_id).await
            {
                tracing::error!(%error, import_id, "coupon import run failed");
            }
            Ok(json!({ "importId": import_id.to_string() }))
        }
        .await,
    )
}
