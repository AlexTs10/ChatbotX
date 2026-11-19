//! The `/rpc` procedures and `/rpc/actions/*` server actions the SPA
//! (`apps/web`, P4) needs, as thin handlers over `chatbotx_business`.
//!
//! One module per domain. A procedure keeps the input / output JSON of the
//! builder's oRPC procedure (`apps/builder/src/features/*/api/private.ts`), an
//! action the one of its `"use server"` export (`features/*/actions/*`;
//! protocol: `conversion/specs/P4-web.md` §5.1). Where the builder procedure
//! returns the same JSON as a ported public `/v1` route, the handler runs that
//! route ([`public`]) after the member checks, so both stay one code path.

mod admin;
mod analytics;
mod appointments;
mod automated_responses;
mod broadcasts;
mod comments;
mod contacts;
mod conversations;
mod ecommerce;
mod email_marketing;
mod email_topics;
mod flows;
mod google_ads;
mod inboxes;
pub(crate) mod integrations;
mod lists;
mod media_library;
mod mini_apps;
mod pipeline_board;
mod pipelines;
mod products;
mod rest;
mod screens;
mod sequences;
mod settings;
mod templates;
mod tools;
mod triggers;
mod user_persistent_menus;
mod whatsapp;
mod working_hours;
mod workspaces;

use axum::body::to_bytes;
use axum::http::StatusCode;
use chatbotx_business::BusinessError;
use chatbotx_db::repo::SortItem;
use serde_json::{Value, json};

use crate::handlers::{ApiState, Call, Member, business_error, dispatch_as_member};
use crate::orpc::OrpcError;
use crate::rpc::{RpcContext, RpcState};

/// What a handler gets: the services, the caller and the workspace.
pub(crate) struct Ctx<'a> {
    pub api: &'a ApiState,
    pub user_id: i64,
    /// `0` for a call without a workspace (`authActionClient` actions).
    pub workspace_id: i64,
    pub role: &'a str,
    pub permissions: &'a Value,
    pub now_ms: i64,
}

impl Ctx<'_> {
    /// `hasWorkspacePermission(permissions, key)`.
    pub fn has(&self, key: &str) -> bool {
        self.permissions.get("superAdmin") == Some(&Value::Bool(true))
            || self.permissions.get(key) == Some(&Value::Bool(true))
    }

    /// `isWorkspaceAdminMember`: the owner or a `superAdmin` member.
    pub fn is_admin(&self) -> bool {
        self.role == "owner" || self.permissions.get("superAdmin") == Some(&Value::Bool(true))
    }

    /// `buildContactPermissionScope`: `None` = no contacts access.
    pub fn contact_scope(&self) -> Option<Member> {
        if !(self.has("contacts") || self.has("onlyAssignedContacts")) {
            return None;
        }
        Some(Member {
            user_id: self.user_id,
            is_admin: self.is_admin(),
            can_view_email_and_phone: self.has("emailAndPhone"),
            restrict_to_assigned_user_id: (!self.has("superAdmin")
                && self.has("onlyAssignedContacts"))
            .then_some(self.user_id),
        })
    }

    /// `requireContactPermissionScopeForMember`: 404 "Contact not found".
    pub fn require_contact_scope(&self) -> Result<Member, OrpcError> {
        self.contact_scope()
            .ok_or_else(|| fail(BusinessError::not_found("Contact not found")))
    }
}

type Outcome = Option<Result<Value, OrpcError>>;

fn ctx<'a>(api: &'a ApiState, context: &'a RpcContext) -> Ctx<'a> {
    static NO_PERMISSIONS: Value = Value::Null;
    Ctx {
        api,
        user_id: context.session.user_id,
        workspace_id: context.workspace.as_ref().map_or(0, |w| w.workspace_id),
        role: context.workspace.as_ref().map_or("", |w| w.role.as_str()),
        permissions: context
            .workspace
            .as_ref()
            .map_or(&NO_PERMISSIONS, |w| &w.permissions),
        now_ms: context.request_issued_at_ms,
    }
}

/// Runs the procedure at `path` (`tagsAPI.privateListWorkspaceTagsAPI`);
/// `None` = not ported. The `/rpc` auth and workspace check ran.
pub async fn procedure(
    state: &RpcState,
    path: &str,
    context: &RpcContext,
    input: &Value,
) -> Outcome {
    let api = state.api.as_ref()?;
    context.workspace.as_ref()?;
    let ctx = ctx(api, context);
    let ctx = &ctx;
    if let Some(outcome) = broadcasts::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = contacts::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = analytics::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = templates::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = flows::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = email_topics::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = sequences::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = whatsapp::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = crate::rpc_ai::call(api, path, &ai_context(ctx), input).await {
        return Some(outcome.map(|output| output.json));
    }
    if let Some(outcome) = inboxes::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = pipelines::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = media_library::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = conversations::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = products::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = tools::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = mini_apps::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = comments::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = email_marketing::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = appointments::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = working_hours::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = templates::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = rest::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    if let Some(outcome) = user_persistent_menus::procedure(ctx, path).await {
        return Some(outcome);
    }
    if let Some(outcome) = google_ads::procedure(ctx, path, input).await {
        return Some(outcome);
    }
    lists::procedure(ctx, path, input).await
}

/// The AI procedures' caller (Q.D6). No support sessions in Rust yet.
fn ai_context(ctx: &Ctx<'_>) -> crate::rpc_ai::RpcAiContext {
    crate::rpc_ai::RpcAiContext::new(ctx.workspace_id, ctx.user_id)
        .with_member(ctx.has("superAdmin"), false)
}

/// Runs the server action `name` with `{ args, workspaceId? }`; `None` = not
/// ported. A workspace action (`workspaceId` set) passed the membership check.
pub async fn action(state: &RpcState, name: &str, context: &RpcContext, input: &Value) -> Outcome {
    let api = state.api.as_ref()?;
    let ctx = ctx(api, context);
    let ctx = &ctx;
    let args = input
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(outcome) = workspaces::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = admin::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = integrations::auth_action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = templates::auth_action(ctx, name, &args).await {
        return Some(outcome);
    }
    // Every other action is a `workspaceActionClient` one.
    if context.workspace.is_none() {
        return match name {
            n if broadcasts::ACTIONS.contains(&n)
                || contacts::ACTIONS.contains(&n)
                || flows::ACTIONS.contains(&n)
                || crate::rpc_ai::PORTED.contains(&format!("actions.{n}").as_str())
                || conversations::ACTIONS.contains(&n)
                || lists::ACTIONS.contains(&n)
                || media_library::ACTIONS.contains(&n)
                || products::ACTIONS.contains(&n)
                || ecommerce::ACTIONS.contains(&n)
                || pipelines::ACTIONS.contains(&n)
                || pipeline_board::ACTIONS.contains(&n)
                || tools::ACTIONS.contains(&n)
                || mini_apps::ACTIONS.contains(&n)
                || comments::is_action(n)
                || rest::ACTIONS.contains(&n)
                || screens::ACTIONS.contains(&n)
                || integrations::ACTIONS.contains(&n)
                || email_topics::ACTIONS.contains(&n)
                || sequences::ACTIONS.contains(&n)
                || triggers::ACTIONS.contains(&n)
                || automated_responses::ACTIONS.contains(&n)
                || settings::ACTIONS.contains(&n)
                || user_persistent_menus::ACTIONS.contains(&n)
                || email_marketing::ACTIONS.contains(&n)
                || appointments::ACTIONS.contains(&n)
                || working_hours::ACTIONS.contains(&n)
                || whatsapp::ACTIONS.contains(&n)
                || google_ads::ACTIONS.contains(&n) =>
            {
                Some(Err(fail(BusinessError::not_found("Workspace not found"))))
            }
            _ => None,
        };
    }
    if let Some(outcome) = broadcasts::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = contacts::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = conversations::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = media_library::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = lists::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = flows::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = integrations::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = email_marketing::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = products::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = ecommerce::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = pipelines::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = tools::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = mini_apps::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = comments::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = email_topics::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = sequences::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = triggers::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = automated_responses::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = settings::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = user_persistent_menus::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = rest::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = screens::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = appointments::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = working_hours::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = whatsapp::action(ctx, name, &args).await {
        return Some(outcome);
    }
    if let Some(outcome) = google_ads::action(ctx, name, &args).await {
        return Some(outcome);
    }
    let path = format!("actions.{name}");
    crate::rpc_ai::call(api, &path, &ai_context(ctx), input)
        .await
        .map(|outcome| outcome.map(|output| output.json))
}

// ---- errors ---------------------------------------------------------------

/// A business error as the procedure error (no declared error map).
pub(crate) fn fail(error: BusinessError) -> OrpcError {
    business_error(error, &[])
}

/// A business error of an action: a `validation` exception with a field is
/// the next-safe-action `validationErrors` (`returnValidationErrors`), any
/// other error the `serverError` message.
pub fn action_fail(error: BusinessError) -> OrpcError {
    if let BusinessError::Exception {
        code,
        field: Some(field),
        message,
        ..
    } = &error
        && code == "validation"
    {
        let mut out = OrpcError::new("BAD_REQUEST", Some(400), Some(message));
        out.data = Some(json!({
            "validationErrors": {
                "_errors": ["Validation Exception"],
                field.as_str(): { "_errors": [message] },
            }
        }));
        return out;
    }
    fail(error)
}

/// A public route's schema failure (422, zod issues) as the action's
/// next-safe-action `validationErrors` (the same schema checked the input).
pub(crate) fn schema_errors(error: OrpcError) -> OrpcError {
    if error.status != 422 {
        return error;
    }
    let issues = error
        .data
        .as_ref()
        .and_then(|data| data.get("issues"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    out.data = Some(json!({
        "validationErrors": crate::rpc::actions::validation_errors(&issues),
    }));
    out
}

/// oRPC's input validation error.
pub(crate) fn invalid() -> OrpcError {
    let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    error.defined = false;
    error
}

// ---- input ----------------------------------------------------------------

/// A JSON input object (`None` keys are absent or `null`).
pub(crate) struct In<'a>(pub &'a Value);

impl<'a> In<'a> {
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.0.get(key).filter(|value| !value.is_null())
    }

    /// `z.string().nullish()`.
    pub fn opt_str(&self, key: &str) -> Result<Option<String>, OrpcError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(invalid()),
        }
    }

    /// `z.string()` (trimmed when `trim`), `min` / `max` characters.
    pub fn str(&self, key: &str, trim: bool, min: usize, max: usize) -> Result<String, OrpcError> {
        let text = self.opt_str(key)?.ok_or_else(invalid)?;
        let text = if trim { text.trim().to_string() } else { text };
        let count = text.chars().count();
        if count < min || count > max {
            return Err(invalid());
        }
        Ok(text)
    }

    /// `zodBigintAsString().nullish()`.
    pub fn opt_id(&self, key: &str) -> Result<Option<i64>, OrpcError> {
        self.get(key).map(id_of).transpose()
    }

    /// `zodBigintAsString()`.
    pub fn id(&self, key: &str) -> Result<i64, OrpcError> {
        self.opt_id(key)?.ok_or_else(invalid)
    }

    /// `z.array(zodBigintAsString())`.
    pub fn ids(&self, key: &str) -> Result<Vec<i64>, OrpcError> {
        match self.get(key) {
            Some(Value::Array(items)) => items.iter().map(id_of).collect(),
            _ => Err(invalid()),
        }
    }

    /// `z.array(z.string())`.
    pub fn strings(&self, key: &str) -> Result<Vec<String>, OrpcError> {
        match self.get(key) {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_string).ok_or_else(invalid))
                .collect(),
            _ => Err(invalid()),
        }
    }

    /// `z.boolean().nullish()`.
    pub fn opt_bool(&self, key: &str) -> Result<Option<bool>, OrpcError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(invalid()),
        }
    }

    /// `z.coerce.number().int().min(1).nullish()` (`page`, `perPage`).
    pub fn opt_positive(&self, key: &str) -> Result<Option<i64>, OrpcError> {
        let number = match self.get(key) {
            None => return Ok(None),
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            Some(_) => None,
        };
        match number {
            Some(n) if n.fract() == 0.0 && n >= 1.0 => Ok(Some(n as i64)),
            _ => Err(invalid()),
        }
    }

    /// `basePaginationRequest.sort`: `[{ id, desc }]` or its JSON text; any
    /// other value is dropped (the preprocess returns `undefined`).
    pub fn sort(&self) -> Option<Vec<SortItem>> {
        let value = match self.get("sort")? {
            Value::String(text) => serde_json::from_str(text).ok()?,
            other => other.clone(),
        };
        value
            .as_array()?
            .iter()
            .map(|item| {
                Some(SortItem {
                    id: item.get("id")?.as_str()?.to_string(),
                    desc: item.get("desc")?.as_bool()?,
                })
            })
            .collect()
    }
}

/// A digit string as an id (`zodBigintAsString`; beyond `i64` = `-1`, no row).
fn id_of(value: &Value) -> Result<i64, OrpcError> {
    match value {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(text.parse().unwrap_or(-1))
        }
        _ => Err(invalid()),
    }
}

/// The action argument at `index` (`bindArgs` first, then the input).
pub(crate) fn arg(args: &[Value], index: usize) -> &Value {
    static NULL: Value = Value::Null;
    args.get(index).unwrap_or(&NULL)
}

/// `{ data, pageCount }` of a paged list.
pub(crate) fn page(data: impl IntoIterator<Item = Value>, page_count: i64) -> Value {
    json!({ "data": data.into_iter().collect::<Vec<_>>(), "pageCount": page_count })
}

/// `conversationService.findDMByContact`: the contact's conversation
/// without a source (not a comment thread), oldest first.
pub(crate) async fn dm_conversation_id(
    ctx: &Ctx<'_>,
    contact_id: i64,
) -> Result<Option<i64>, OrpcError> {
    use chatbotx_db::rusqlite::OptionalExtension;
    let ws = ctx.workspace_id;
    chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "id" FROM "Conversation" WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL ORDER BY "id" LIMIT 1"#,
                [ws, contact_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
    .map_err(fail)
}

// ---- public route bridge --------------------------------------------------

/// Runs the public `/v1` route `route_id` for the member (`params`, a query
/// string, a JSON body) and returns its JSON body (`null` for a 204) or its
/// error.
pub(crate) async fn public(
    ctx: &Ctx<'_>,
    route_id: &str,
    params: &[(&'static str, String)],
    query: Option<&str>,
    body: Option<&Value>,
) -> Result<Value, OrpcError> {
    let body = body.map(Value::to_string).unwrap_or_default();
    let call = Call {
        workspace_id: ctx.workspace_id,
        params,
        query,
        body: body.as_bytes(),
    };
    let member = ctx.contact_scope().unwrap_or(Member {
        user_id: ctx.user_id,
        is_admin: ctx.is_admin(),
        can_view_email_and_phone: false,
        restrict_to_assigned_user_id: Some(ctx.user_id),
    });
    let Some(response) = dispatch_as_member(ctx.api, route_id, call, member).await else {
        return Err(OrpcError::new(
            "NOT_FOUND",
            Some(404),
            Some("Operation not ported yet"),
        ));
    };
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| crate::handlers::internal_error(&e.to_string()))?;
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|e| crate::handlers::internal_error(&e.to_string()))?
    };
    if status.is_success() {
        return Ok(value);
    }
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("INTERNAL_SERVER_ERROR");
    let message = value.get("message").and_then(Value::as_str);
    let mut error = OrpcError::new(code, Some(status.as_u16()), message);
    error.data = value.get("data").cloned().filter(|d| !d.is_null());
    error.defined = false;
    if status == StatusCode::BAD_REQUEST && code == "BAD_REQUEST" {
        error.data = None;
    }
    Err(error)
}

/// `?a=1&b=x` of flat input keys (strings, numbers, booleans).
pub(crate) fn query_of(input: &Value, keys: &[&str]) -> String {
    let mut out = String::new();
    for key in keys {
        let text = match input.get(*key) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            Some(Value::Bool(flag)) => flag.to_string(),
            _ => continue,
        };
        if !out.is_empty() {
            out.push('&');
        }
        out.push_str(key);
        out.push('=');
        out.push_str(&percent(&text));
    }
    out
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The procedure futures run inside the axum `/rpc` handler, so they must be `Send`.
#[cfg(test)]
#[allow(dead_code)]
mod send_checks {
    use super::*;

    fn assert_send<T: Send>(_: T) {}

    fn procedure_futures_are_send(ctx: &Ctx<'_>, args: &[Value], input: &Value) {
        assert_send(conversations::action(ctx, "threadControlAction", args));
        assert_send(whatsapp::procedure(ctx, "", input));
        assert_send(whatsapp::action(ctx, "", args));
    }

    fn handler_futures_are_send(state: &ApiState, call: Call<'_>) {
        assert_send(crate::handlers::conversations::handle(state, "", call));
    }

    fn whatsapp_handler_futures_are_send(state: &ApiState, a: Call<'_>, b: Call<'_>) {
        assert_send(crate::handlers::whatsapp_flows::handle(state, "", a));
        assert_send(crate::handlers::whatsapp_templates::handle(state, "", b));
        assert_send(crate::handlers::whatsapp_coexist::handle(state, "", a));
        assert_send(crate::handlers::whatsapp_calls::handle(state, "", a));
    }
}
