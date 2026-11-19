//! Contact import and Automatic Customer Scan pages (Q.F2-automation):
//! `features/contact-scan/{actions,api/private,queries}`,
//! `features/contacts/actions/import-contacts.action.ts` and
//! `features/import/queries/list-imports.queries.ts`.

use chatbotx_business::BusinessError;
use chatbotx_business::contact_scan as scan;
use chatbotx_business::import::contacts::start_contact_import;
use chatbotx_business::import::{ImportType, ListImports, list};
use serde_json::{Value, json};

use super::rest::input_at;
use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, schema_errors};
use crate::handlers::contact_imports::{import_json, run_started, start_import_input};
use crate::orpc::OrpcError;

/// The ported actions and page queries.
pub const ACTIONS: &[&str] = &[
    "scheduleContactScanAction",
    "importContactsAction",
    "listContactScanHistory",
    "listImports",
    "getContactsImportTemplate",
];

const IMPORT_STATUSES: &[&str] = &["pending", "processing", "completed", "failed"];

/// `requireUnrestrictedContactsScope`: contacts access
/// (`requireContactPermissionScope`), and not an assigned-only member (403
/// `contactScanForbidden`).
fn require_unrestricted(ctx: &Ctx<'_>) -> Result<(), BusinessError> {
    let scope = ctx.contact_scope().ok_or_else(|| {
        BusinessError::exception(
            "User is not authorized to access contacts",
            "systemError",
            400,
        )
    })?;
    if scope.restrict_to_assigned_user_id.is_some() {
        return Err(BusinessError::exception(
            "You do not have permission to run an Automatic Customer Scan.",
            "contactScanForbidden",
            403,
        ));
    }
    Ok(())
}

/// The action's `returnValidationErrors` for the mapped exception codes:
/// `(code, Some(field))` = a field error, `(code, None)` = a top-level one.
fn mapped(error: BusinessError, codes: &[(&str, Option<&str>)]) -> OrpcError {
    if let BusinessError::Exception { code, message, .. } = &error
        && let Some((_, field)) = codes.iter().find(|(c, _)| c == code)
    {
        let mut out = OrpcError::new("BAD_REQUEST", Some(400), Some(message));
        out.data = Some(match field {
            Some(field) => json!({ "validationErrors": { *field: { "_errors": [message] } } }),
            None => json!({ "validationErrors": { "_errors": [message] } }),
        });
        return out;
    }
    action_fail(error)
}

const SCAN_ERRORS: &[(&str, Option<&str>)] = &[
    ("contactScanFromTimeInvalid", Some("scanFromAt")),
    ("contactScanInboxNotFound", Some("inboxId")),
    ("contactScanChannelUnsupported", Some("inboxId")),
    ("contactScanIntegrationDisconnected", Some("inboxId")),
    ("contactScanForbidden", None),
    ("contactScanCooldown", None),
    ("contactScanAlreadyRunning", None),
];

const IMPORT_ERRORS: &[(&str, Option<&str>)] = &[
    ("contactImportFileNotFound", Some("fileId")),
    ("contactImportFileTypeInvalid", Some("fileId")),
    ("contactImportUnsupportedFormat", Some("fileId")),
    ("contactImportInboxNotFound", Some("inboxId")),
    ("contactImportAlreadyRunning", None),
];

/// `contactScanAPIs.getContactScanStatusAuthenticatedAPI`.
pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    if path != "contactScanAPIs.getContactScanStatusAuthenticatedAPI" {
        return None;
    }
    Some(
        async {
            require_unrestricted(ctx).map_err(fail)?;
            let inbox_id = In(input).id("inboxId")?;
            scan::status(&ctx.api.db, ctx.workspace_id, inbox_id)
                .await
                .map_err(fail)
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
    Some(
        async {
            match name {
                // [workspaceId], { inboxId, scanFromAt }
                "scheduleContactScanAction" => {
                    require_unrestricted(ctx).map_err(|e| mapped(e, SCAN_ERRORS))?;
                    let input = In(arg(args, 1));
                    let inbox_id = input.id("inboxId")?;
                    let scan_from_at = input
                        .0
                        .get("scanFromAt")
                        .and_then(crate::handlers::broadcasts::coerce_date)
                        .ok_or_else(invalid)?;
                    let run_id = scan::schedule(db, ws, inbox_id, scan_from_at, Some(ctx.user_id))
                        .await
                        .map_err(|e| mapped(e, SCAN_ERRORS))?;
                    Ok(json!({ "runId": run_id.to_string() }))
                }
                // [workspaceId], importContactsRequest
                "importContactsAction" => {
                    let mut start =
                        start_import_input(input_at(args, 1), ws).map_err(schema_errors)?;
                    start.user_id = Some(ctx.user_id);
                    let import_id = start_contact_import(db, start)
                        .await
                        .map_err(|e| mapped(e, IMPORT_ERRORS))?;
                    run_started(ctx.api, import_id).await;
                    Ok(json!({ "importId": import_id.to_string() }))
                }
                // [workspaceId, input]; the scan pages' gate (404 for a denied
                // caller, `requireContactScanPageAccess`).
                "listContactScanHistory" => {
                    if require_unrestricted(ctx).is_err() {
                        return Err(fail(BusinessError::not_found("Page not found")));
                    }
                    let input = In(arg(args, 1));
                    let page = input.opt_positive("page")?.unwrap_or(1);
                    let per_page = input.opt_positive("perPage")?.unwrap_or(10);
                    scan::list(db, ws, None, page, per_page).await.map_err(fail)
                }
                // [workspaceId]: `contacts/import/template/route.ts` (404
                // without contacts access), as `{ fileName, mimeType, content }`.
                "getContactsImportTemplate" => {
                    if ctx.contact_scope().is_none() {
                        return Err(fail(BusinessError::not_found("Page not found")));
                    }
                    super::public(ctx, "contacts.getImportTemplate", &[], None, None).await
                }
                // [workspaceId, input]: the import history pages. The page
                // guards run here (no server page): contacts access for
                // `contacts`, the `flows` permission for `flow`.
                _ => {
                    let input = In(arg(args, 1));
                    let import_type = match input.opt_str("type")? {
                        None => None,
                        Some(text) => Some(ImportType::parse(&text).ok_or_else(invalid)?),
                    };
                    let allowed = match import_type {
                        Some(ImportType::Contacts) => ctx.contact_scope().is_some(),
                        Some(ImportType::Flow) => ctx.has("flows"),
                        _ => true,
                    };
                    if !allowed {
                        return Err(fail(BusinessError::not_found("Page not found")));
                    }
                    let status = match input.opt_str("status")? {
                        Some(s) if !IMPORT_STATUSES.contains(&s.as_str()) => return Err(invalid()),
                        status => status,
                    };
                    let (rows, page_count) = list(
                        db,
                        ws,
                        ListImports {
                            import_type,
                            status,
                            keyword: input.opt_str("keyword")?,
                            page: input.opt_positive("page")?,
                            per_page: input.opt_positive("perPage")?,
                            sort: input.sort(),
                        },
                    )
                    .await
                    .map_err(fail)?;
                    let data: Vec<Value> = rows
                        .iter()
                        .map(|row| {
                            let kind =
                                ImportType::parse(&row.import_type).unwrap_or(ImportType::Contacts);
                            let mut out = import_json(row, kind);
                            out["workspaceId"] = json!(row.workspace_id.to_string());
                            out
                        })
                        .collect();
                    Ok(json!({ "data": data, "pageCount": page_count }))
                }
            }
        }
        .await,
    )
}
