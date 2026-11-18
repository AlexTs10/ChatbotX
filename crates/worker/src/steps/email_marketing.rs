//! Q.I3: the Mail Marketing steps of `step.ts` (`activeCampaignSyncContact`,
//! `dripSubscribeSubscriber`, `getResponseAddContact`, `klaviyoSyncProfile`,
//! `mailchimpAddMember`, `mailerLiteAddSubscriber`, `moosendCreateContact`,
//! `sendGridAddContact`). The provider logic lives in
//! `chatbotx_business::email_marketing`; this adapter logs, records the
//! provider error (`logStepProviderError`) and routes on the status.

use chatbotx_business::email_marketing::{
    self as em, Http, Outcome, ProviderError, active_campaign, drip, get_response, klaviyo,
    mailchimp, mailer_lite, moosend, sendgrid,
};
use chatbotx_jobs::JobError;
use serde_json::json;

use super::{StepCtx, step_error};
use crate::flow::StepStatus;

/// `Some(status)` for a Mail Marketing step.
pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    let (provider, label) = match step_type {
        "activeCampaignSyncContact" => {
            ("active-campaign", "ActiveCampaign sync-contact step failed")
        }
        "dripSubscribeSubscriber" => ("drip", "Drip sync-subscriber step failed"),
        "getResponseAddContact" => ("get-response", "GetResponse contact sync failed"),
        "klaviyoSyncProfile" => ("klaviyo", "Klaviyo profile sync failed"),
        "mailchimpAddMember" => ("mailchimp", "Mailchimp add-member step failed"),
        "mailerLiteAddSubscriber" => ("mailer-lite", "MailerLite subscriber sync failed"),
        "moosendCreateContact" => ("moosend", "Moosend contact sync failed"),
        "sendGridAddContact" => ("sendgrid", "SendGrid add-contact step failed"),
        "email" => return send_email(ctx).await.map(Some),
        _ => return Ok(None),
    };
    let outcome = execute(ctx, step_type, em::safe_http()).await;
    let Err(err) = outcome else {
        return Ok(Some(StepStatus::Success));
    };
    tracing::error!(
        workspace_id = ctx.workspace_id(),
        conversation_id = ctx.run.conversation.id,
        step_id = ctx.str("id"),
        provider_status = err.status,
        kind = err.kind,
        retry_after_seconds = err.retry_after_seconds,
        err = %err.message,
        "{label}"
    );
    if err.logged() {
        let input = crate::events::ProviderError {
            provider: provider.to_owned(),
            workspace_id: ctx.workspace_id().to_string(),
            contact_id: Some(ctx.contact_id().to_string()),
            source_id: Some(ctx.run.contact_inbox.source_id.clone()),
            error: json!({ "message": err.message, "statusCode": err.status }),
            ..Default::default()
        };
        crate::events::log_provider_errors(ctx.run.deps.events.as_ref(), &[input]).await;
    }
    Ok(Some(step_error(ctx, &err.message)))
}

async fn execute(ctx: &StepCtx<'_, '_>, step_type: &str, http: &dyn Http) -> Outcome<()> {
    let db = &ctx.run.deps.db;
    let (ws, contact, step) = (ctx.workspace_id(), ctx.contact_id(), ctx.step);
    match step_type {
        "activeCampaignSyncContact" => {
            active_campaign::sync_contact_step(db, http, ws, contact, step).await
        }
        "dripSubscribeSubscriber" => drip::subscribe_step(db, http, ws, contact, step).await,
        "getResponseAddContact" => {
            get_response::add_contact_step(db, http, ws, contact, step).await
        }
        "klaviyoSyncProfile" => klaviyo::sync_profile_step(db, http, ws, contact, step).await,
        "mailchimpAddMember" => mailchimp::add_member_step(db, http, ws, contact, step).await,
        "mailerLiteAddSubscriber" => {
            mailer_lite::add_subscriber_step(db, http, ws, contact, step).await
        }
        "moosendCreateContact" => moosend::create_contact_step(db, http, ws, contact, step).await,
        "sendGridAddContact" => sendgrid::add_contact_step(db, http, ws, contact, step).await,
        other => Err(ProviderError::new(format!("Unknown step type: {other}"))),
    }
}

/// One `IntegrationSmtp` row of the step.
struct SmtpRow {
    auth: String,
    name: String,
    from_address: String,
}

/// `sendEmail` (`send-email.ts`): opt-in check, SMTP row, variables, the
/// resolved elements and one SMTP send. Always `success` (the original
/// returns `void`, also when the send fails).
///
/// ponytail: the email-topic tracking (`createRecipient`, open pixel,
/// signed click URLs, `markFailed` / `delivered`) belongs to Q.D5; until
/// then buttons link straight to their URL.
pub(super) async fn send_email(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    use chatbotx_business::email_marketing::smtp::{
        self, Mail, MailElement, SmtpAuth, UNSUBSCRIBE_PLACEHOLDER,
    };
    use chatbotx_db::rusqlite::{OptionalExtension as _, params};
    let run = ctx.run;
    let (ws, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    let smtp_id = ctx
        .str("integrationSmtpId")
        .and_then(|id| id.parse::<i64>().ok());
    let inbox_id = run.contact_inbox.inbox_id;
    let (opt_in, row, workspace_name, inbox) = run
        .deps
        .db
        .read(move |conn| {
            let opt_in: Option<bool> = conn
                .query_row(r#"SELECT "emailOptIn" FROM "Contact" WHERE "id" = ?1"#, params![contact_id], |r| r.get(0))
                .optional()?;
            let row = match smtp_id {
                Some(id) => conn
                    .query_row(
                        r#"SELECT "auth", "name", "fromAddress" FROM "IntegrationSmtp" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                        params![ws, id],
                        |r| Ok(SmtpRow { auth: r.get(0)?, name: r.get(1)?, from_address: r.get(2)? }),
                    )
                    .optional()?,
                None => None,
            };
            let name: Option<String> = conn
                .query_row(r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#, params![ws], |r| r.get(0))
                .optional()?;
            let inbox = chatbotx_business::inbox::links::list_connected_link_inboxes(conn, ws)
                .ok()
                .and_then(|inboxes| inboxes.into_iter().find(|i| i.id == inbox_id));
            Ok((opt_in, row, name, inbox))
        })
        .await
        .map_err(crate::db_fail)?;
    if opt_in != Some(true) {
        tracing::info!(
            contact_id,
            "handleSendEmail: contact has opted out of email, skipping"
        );
        return Ok(StepStatus::Success);
    }
    let Some(row) = row else {
        tracing::warn!(
            "handleSendEmail: smtp integration {} not found",
            ctx.str("integrationSmtpId").unwrap_or("")
        );
        return Ok(StepStatus::Success);
    };
    let Some(auth) = em::open_auth(&row.auth)
        .as_ref()
        .and_then(SmtpAuth::from_value)
    else {
        tracing::error!(workspace_id = ws, "handleSendEmail: invalid SMTP auth");
        return Ok(StepStatus::Success);
    };
    let step = ctx
        .resolve_variables(ctx.step, false)
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(err = %err, "handleSendEmail: variables not resolved");
            ctx.step.clone()
        });
    let text = |key: &str| {
        step.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let app_url = run.deps.app_url.as_str();
    let unsubscribe = smtp::unsubscribe_url(app_url, contact_id, ws).unwrap_or_default();
    let elements: Vec<MailElement> = step
        .get("elements")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|el| {
            let s = |k: &str| {
                el.get(k)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let body = || {
                s("text")
                    .unwrap_or_default()
                    .replace(UNSUBSCRIBE_PLACEHOLDER, &unsubscribe)
            };
            Some(match s("type")?.as_str() {
                "heading" => MailElement::Heading(body()),
                "text" => MailElement::Text(body()),
                "code" => MailElement::Code(body()),
                "image" => MailElement::Image(s("url")),
                "line" => MailElement::Line,
                "spacing" => MailElement::Spacing,
                "button" => MailElement::Button {
                    label: s("label"),
                    url: button_url(app_url, el, inbox.as_ref(), run.flow_version.flow_id),
                },
                _ => return None,
            })
        })
        .collect();
    let subject = text("subject");
    let html = smtp::render_email_html(&subject, &text("preheader"), &elements);
    let from = Some(text("from"))
        .filter(|f| !f.is_empty())
        .unwrap_or(row.from_address);
    let to = text("to");
    let _brand = workspace_name.unwrap_or(row.name);
    let mail = Mail {
        from: &from,
        to: &to,
        subject: &subject,
        html: &html,
    };
    if let Err(err) = smtp::send_mail(&auth, &mail).await {
        tracing::error!(integration_smtp_id = smtp_id, workspace_id = ws, err = %err, "handleSendEmail: SMTP send failed");
    }
    Ok(StepStatus::Success)
}

/// `resolveButtonUrl` (`lib/convert-button.ts`).
fn button_url(
    app_url: &str,
    button: &serde_json::Value,
    inbox: Option<&chatbotx_business::inbox::links::LinkInbox>,
    flow_id: i64,
) -> Option<String> {
    use chatbotx_business::inbox::links::build_inbox_link;
    use chatbotx_business::referral::RefConfig;
    let before = |k: &str| {
        button
            .pointer(&format!("/beforeStep/{k}"))
            .and_then(serde_json::Value::as_str)
    };
    let kind = button
        .get("buttonType")
        .and_then(serde_json::Value::as_str)?;
    if kind == "openWebsite" {
        return before("url").map(str::to_owned);
    }
    let inbox = inbox?;
    let flow = |flow_id: i64, node_id: Option<&str>| RefConfig::Flow {
        flow_id,
        node_id: node_id.map(str::to_owned),
    };
    match kind {
        "startExternalFlow" => {
            let target = before("flowId").and_then(|f| f.parse().ok())?;
            build_inbox_link(app_url, inbox, Some(&flow(target, None)))
        }
        "startExternalNode" => {
            let target = before("flowId").and_then(|f| f.parse().ok())?;
            build_inbox_link(app_url, inbox, Some(&flow(target, before("nodeId"))))
        }
        "sendMessage" | "performAction" | "startAnotherNode" => {
            build_inbox_link(app_url, inbox, Some(&flow(flow_id, before("nodeId"))))
        }
        _ => None,
    }
}
