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
