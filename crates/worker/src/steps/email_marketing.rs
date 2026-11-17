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
