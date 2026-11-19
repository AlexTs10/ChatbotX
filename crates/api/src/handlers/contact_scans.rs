//! `features/contact-scan/api/public.ts` (`contactScanPublicRouter`). The
//! worker that runs a scheduled scan is not part of this module.

use chatbotx_business::contact_scan as service;
use serde_json::Value;

use super::input::{Input, key};
use super::qr_codes::{public_paging, sort_query};
use super::{ApiState, Call, ErrorMap, LISTING, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contactScans.getStatus",
    "contactScans.list",
    "contactScans.create",
];

/// `possibleErrorsOnSchedulingContactScan`.
const SCHEDULING: ErrorMap = &[
    ("businessError", 400),
    ("contactScanFromTimeInvalid", 400),
    ("contactScanInboxNotFound", 404),
    ("contactScanChannelUnsupported", 400),
    ("contactScanIntegrationDisconnected", 400),
    ("contactScanCooldown", 409),
    ("contactScanAlreadyRunning", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `z.coerce.date()`: `new Date(value)` must be valid.
pub(crate) fn coerced_date(input: &mut Input, name: &str) -> Option<i64> {
    let parsed = match input.get(name) {
        Some(Value::String(text)) => chatbotx_core::js::parse_date(text),
        Some(Value::Number(n)) => n.as_f64().map(|n| n as i64),
        _ => None,
    };
    if parsed.is_none() {
        input.issue(
            &key(name),
            "invalid_type",
            "Invalid input: expected date, received Date",
        );
    }
    parsed
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "contactScans.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let inbox_id = input.digits_id("inboxId");
            let scan_from_at = coerced_date(&mut input, "scanFromAt");
            input.finish()?;
            let run_id = service::schedule(
                db,
                ws,
                inbox_id.unwrap_or(-1),
                scan_from_at.unwrap_or_default(),
                None,
            )
            .await
            .map_err(|e| business_error(e, SCHEDULING))?;
            Ok(json(
                202,
                serde_json::json!({ "runId": run_id.to_string() }),
            ))
        }
        "contactScans.getStatus" => {
            let mut input = Input::from_query(call.params, call.query);
            let inbox_id = input.digits_id("inboxId");
            input.finish()?;
            service::status(db, ws, inbox_id.unwrap_or(-1))
                .await
                .map(|view| json(200, view))
                .map_err(|e| business_error(e, LISTING))
        }
        _ => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let integration_id = input
                .has("integrationId")
                .then(|| input.digits_id("integrationId"))
                .flatten();
            // ponytail: `sort` is validated and ignored (newest first, the default).
            let _ = sort_query(&mut input, None);
            input.finish()?;
            service::list(db, ws, integration_id, page, per_page)
                .await
                .map(|view| json(200, view))
                .map_err(|e| business_error(e, LISTING))
        }
    }
}
