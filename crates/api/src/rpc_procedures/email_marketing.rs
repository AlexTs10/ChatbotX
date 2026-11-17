//! Q.I3: the flow-editor option lists of the email-marketing integrations
//! (`features/integration-<name>/api/index.ts`, `integration<Name>API.*`).
//! The workspace check ran in `/rpc`; a provider failure is a 500 unless the
//! original maps it (rate limits, Moosend's kinds).

use chatbotx_business::email_marketing::{
    self as em, Outcome, ProviderError, active_campaign, drip, get_response, klaviyo, mailchimp,
    mailer_lite, moosend, sendgrid,
};
use serde_json::{Value, json};

use super::{Ctx, In, invalid};
use crate::handlers::internal_error;
use crate::orpc::OrpcError;

/// `z.coerce.number().int().positive().max(max).default(default)`.
fn page_arg(input: &In<'_>, key: &str, max: i64, default: i64) -> Result<u32, OrpcError> {
    let n = input.opt_positive(key)?.unwrap_or(default);
    if n > max {
        return Err(invalid());
    }
    u32::try_from(n).map_err(|_| invalid())
}

/// `z.string().min(1)` (`trim` for the cursor / token inputs).
fn text_arg(input: &In<'_>, key: &str, trim: bool) -> Result<Option<String>, OrpcError> {
    Ok(match input.opt_str(key)? {
        Some(text) => {
            let text = if trim { text.trim().to_owned() } else { text };
            if text.is_empty() {
                return Err(invalid());
            }
            Some(text)
        }
        None => None,
    })
}

fn required(input: &In<'_>, key: &str) -> Result<String, OrpcError> {
    text_arg(input, key, false)?.ok_or_else(invalid)
}

fn too_many(message: &str) -> OrpcError {
    OrpcError::new("TOO_MANY_REQUESTS", Some(429), Some(message))
}

/// The original's per-provider error mapping.
fn map_error(provider: &str, err: ProviderError) -> OrpcError {
    let rate_limited = err.status == Some(429);
    match provider {
        "moosend" => match err.kind {
            Some("rate_limited") => too_many("Moosend rate limit exceeded"),
            Some("invalid_credentials" | "user_not_enabled") => OrpcError::new(
                "BAD_REQUEST",
                Some(400),
                Some("Moosend integration is unavailable"),
            ),
            Some(_) => OrpcError::new(
                "INTERNAL_SERVER_ERROR",
                Some(502),
                Some("Moosend request failed"),
            ),
            None if err.message == "Moosend integration not found" => OrpcError::new(
                "FAILED_PRECONDITION",
                None,
                Some("Moosend integration not connected"),
            ),
            None => internal_error(&err.message),
        },
        "klaviyo" if rate_limited => too_many("Klaviyo rate limit exceeded"),
        "mailerLite" if rate_limited => too_many("MailerLite rate limit exceeded"),
        "activeCampaign" | "drip" | "getResponse" | "sendGrid" if rate_limited => {
            too_many(&err.message)
        }
        _ => internal_error(&err.message),
    }
}

const PROVIDERS: &[(&str, &str)] = &[
    ("integrationActiveCampaignAPI", "activeCampaign"),
    ("integrationDripAPI", "drip"),
    ("integrationGetResponseAPI", "getResponse"),
    ("integrationKlaviyoAPI", "klaviyo"),
    ("integrationMailchimpAPI", "mailchimp"),
    ("integrationMailerLiteAPI", "mailerLite"),
    ("integrationMoosendAPI", "moosend"),
    ("integrationSendGridAPI", "sendGrid"),
];

fn data(items: Vec<Value>) -> Value {
    json!({ "data": items })
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> super::Outcome {
    if path == "integrationSmtpAPI.listIntegrationSmtps" {
        let rows = em::smtp_service::list_by_workspace(&ctx.api.db, ctx.workspace_id).await;
        return Some(rows.map(data).map_err(|e| internal_error(&e.to_string())));
    }
    let (router, name) = path.split_once('.')?;
    let provider = PROVIDERS.iter().find(|(key, _)| *key == router)?.1;
    let input = In(input);
    let out = run(ctx, provider, name, &input).await?;
    Some(out.and_then(|outcome| outcome.map_err(|err| map_error(provider, err))))
}

async fn run(
    ctx: &Ctx<'_>,
    provider: &'static str,
    name: &str,
    input: &In<'_>,
) -> Option<Result<Outcome<Value>, OrpcError>> {
    let http = em::safe_http();
    // Input checks first (oRPC validates before the handler).
    let args = (|| -> Result<(u32, u32, Option<String>), OrpcError> {
        Ok(match (provider, name) {
            ("drip", "listTags" | "listCustomFields") => {
                (0, 0, Some(required(input, "accountId")?))
            }
            ("mailchimp", "listTags" | "listMergeFields") => {
                (0, 0, Some(required(input, "listId")?))
            }
            ("getResponse", "listCampaigns") => (
                page_arg(input, "page", i64::from(u32::MAX), 1)?,
                page_arg(input, "perPage", 100, 100)?,
                None,
            ),
            ("getResponse", "listTags") => (
                page_arg(input, "page", i64::from(u32::MAX), 1)?,
                page_arg(input, "perPage", 1000, 1000)?,
                None,
            ),
            ("klaviyo", "listLists") => (
                0,
                page_arg(input, "size", 10, 10)?,
                text_arg(input, "cursor", true)?,
            ),
            ("mailerLite", _) => (
                page_arg(input, "page", i64::from(u32::MAX), 1)?,
                page_arg(input, "limit", 100, 100)?,
                None,
            ),
            ("moosend", _) => (
                page_arg(input, "page", i64::from(u32::MAX), 1)?,
                page_arg(input, "pageSize", 20, 20)?,
                None,
            ),
            ("sendGrid", "listLists") => (
                0,
                page_arg(input, "pageSize", 1000, 1000)?,
                text_arg(input, "pageToken", true)?,
            ),
            _ => (0, 0, None),
        })
    })();
    let known = matches!(
        (provider, name),
        (
            "activeCampaign",
            "listLists" | "listAutomations" | "listTags" | "listCustomFields"
        ) | ("drip", "listAccounts" | "listTags" | "listCustomFields")
            | ("getResponse", "listCampaigns" | "listTags")
            | ("klaviyo", "listLists")
            | (
                "mailchimp",
                "listAudiences" | "listTags" | "listMergeFields"
            )
            | ("mailerLite", "listGroups" | "listFields")
            | ("moosend", "listMailingLists")
            | ("sendGrid", "listLists" | "listCustomFields")
    );
    if !known {
        return None;
    }
    let (page, size, arg) = match args {
        Ok(args) => args,
        Err(err) => return Some(Err(err)),
    };
    let arg = arg.unwrap_or_default();
    let outcome = async {
        let auth = em::load_auth(&ctx.api.db, provider, ctx.workspace_id).await?;
        let auth = &auth;
        Ok::<Value, ProviderError>(match (provider, name) {
            ("activeCampaign", "listLists") => data(active_campaign::list_lists(http, auth).await?),
            ("activeCampaign", "listAutomations") => {
                data(active_campaign::list_automations(http, auth).await?)
            }
            ("activeCampaign", "listTags") => data(active_campaign::list_tags(http, auth).await?),
            ("activeCampaign", _) => data(active_campaign::list_custom_fields(http, auth).await?),
            ("drip", "listAccounts") => data(drip::list_accounts(http, auth).await?),
            ("drip", "listTags") => data(drip::list_tags(http, auth, &arg).await?),
            ("drip", _) => data(drip::list_custom_fields(http, auth, &arg).await?),
            ("getResponse", "listCampaigns") => {
                get_response::list_campaigns(http, auth, page, size).await?
            }
            ("getResponse", _) => get_response::list_tags(http, auth, page, size).await?,
            ("klaviyo", _) => {
                klaviyo::list_lists(http, auth, (!arg.is_empty()).then_some(arg.as_str()), size)
                    .await?
            }
            ("mailchimp", "listAudiences") => data(mailchimp::list_audiences(http, auth).await?),
            ("mailchimp", "listTags") => data(mailchimp::list_tags(http, auth, &arg).await?),
            ("mailchimp", _) => data(mailchimp::list_merge_fields(http, auth, &arg).await?),
            ("mailerLite", "listGroups") => {
                mailer_lite::list_groups(http, auth, page, size).await?
            }
            ("mailerLite", _) => mailer_lite::list_fields(http, auth, page, size).await?,
            ("moosend", _) => moosend::list_mailing_lists(http, auth, page, size).await?,
            ("sendGrid", "listLists") => {
                sendgrid::list_lists(http, auth, size, (!arg.is_empty()).then_some(arg.as_str()))
                    .await?
            }
            _ => data(sendgrid::list_custom_fields(http, auth).await?),
        })
    }
    .await;
    Some(Ok(outcome))
}

/// `features/integration-smtp/actions/*`.
/// and `disconnectGoogleSheetsAction` (Q.I4).
pub(super) const ACTIONS: &[&str] = &[
    "createSmtpAction",
    "updateSmtpAction",
    "deleteSmtpAction",
    "disconnectGoogleSheetsAction",
];

/// `zodBigintAsString` bind argument.
fn bind_id(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    match super::arg(args, index) {
        Value::String(t) if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(t.parse().unwrap_or(-1))
        }
        _ => Err(invalid()),
    }
}

/// `createSmtpRequest` / `updateSmtpRequest` of the action input.
fn smtp_input(value: &Value, create: bool) -> Result<em::smtp_service::SmtpInput, OrpcError> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    let mut input = crate::handlers::input::Input::from_body(&[], &body);
    let parsed = crate::handlers::smtp_integrations::parse(&mut input, create);
    input.finish().map_err(|_| invalid())?;
    parsed.ok_or_else(invalid)
}

async fn audit(ctx: &Ctx<'_>, action: &str, detail: String) {
    ctx.api
        .audit
        .record(chatbotx_business::audit::AuditRecordInput {
            user_id: Some(ctx.user_id.to_string()),
            workspace_id: Some(ctx.workspace_id.to_string()),
            ..chatbotx_business::audit::AuditRecordInput::new(action, detail)
        })
        .await;
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> super::Outcome {
    use em::smtp_service as smtp;
    if !ACTIONS.contains(&name) {
        return None;
    }
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let fail = super::action_fail;
    Some(
        async {
            match name {
                "createSmtpAction" => {
                    let data = smtp_input(super::arg(args, 1), true)?;
                    let (host, port) = smtp::prepare_auth(&data).await.map_err(fail)?;
                    let id = smtp::connect(db, ws, Some(ctx.user_id), &data, &host, port)
                        .await
                        .map_err(fail)?;
                    audit(
                        ctx,
                        "connect",
                        format!("connected a new SMTP channel (#{id})"),
                    )
                    .await;
                    Ok(json!({ "id": id.to_string() }))
                }
                "updateSmtpAction" => {
                    let id = bind_id(args, 1)?;
                    let data = smtp_input(super::arg(args, 2), false)?;
                    let (host, port) = smtp::prepare_auth(&data).await.map_err(fail)?;
                    let changed = smtp::update(db, ws, id, &data, &host, port)
                        .await
                        .map_err(fail)?;
                    if changed {
                        audit(
                            ctx,
                            "update",
                            "updated the SMTP channel configuration".into(),
                        )
                        .await;
                    }
                    let row = smtp::find_by_id_for_workspace(db, id, ws)
                        .await
                        .map_err(fail)?;
                    // ponytail: the original returns the whole row; the `auth`
                    // blob (password) stays on the server.
                    Ok(json!({
                        "id": row.id.to_string(),
                        "workspaceId": ws.to_string(),
                        "inboxId": row.inbox_id.to_string(),
                        "name": row.name,
                        "fromAddress": row.from_address,
                    }))
                }
                "disconnectGoogleSheetsAction" => {
                    chatbotx_business::google_sheets::disconnect(db, em::safe_http(), ws)
                        .await
                        .map_err(fail)?;
                    audit(
                        ctx,
                        "disconnect",
                        "disconnected the Google Sheets integration".into(),
                    )
                    .await;
                    Ok(Value::Null)
                }
                _ => {
                    let id = bind_id(args, 1)?;
                    smtp::disconnect(db, ws, id).await.map_err(fail)?;
                    audit(
                        ctx,
                        "disconnect",
                        format!("disconnected the SMTP channel (#{id})"),
                    )
                    .await;
                    Ok(Value::Null)
                }
            }
        }
        .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_provider_errors() {
        let limited = ProviderError {
            status: Some(429),
            ..ProviderError::new("slow down")
        };
        assert_eq!(map_error("drip", limited.clone()).status, 429);
        assert_eq!(map_error("mailchimp", limited).status, 500);
        let moosend = ProviderError {
            kind: Some("user_not_enabled"),
            ..ProviderError::new("x")
        };
        assert_eq!(map_error("moosend", moosend).status, 400);
        assert_eq!(
            map_error(
                "moosend",
                ProviderError::new("Moosend integration not found")
            )
            .code,
            "FAILED_PRECONDITION"
        );
    }
}
