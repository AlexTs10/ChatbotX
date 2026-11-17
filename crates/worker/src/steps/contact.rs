//! `integration/handlers/contact.ts`, `custom-field-operation.ts` and the
//! `stepBlockContact` handler of `step-handlers.ts`.
//!
//! ponytail: the contact events these services emit in TypeScript
//! (`tagApplied` / `tagRemoved`, custom-field changes, contact
//! unsubscribed, sequence subscribed, contact-info change) and the cache
//! invalidation come with the event workers (Q.W2).

use chatbotx_business::{
    BusinessError, contact, contact_custom_field, contact_note, contact_sequence, tag,
};
use chatbotx_core::utils::custom_field_operation::FieldOperationType;
use chatbotx_core::utils::datetime::TemporalInputParsing;
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::StepCtx;
use crate::flow::StepStatus;
use crate::{db_fail, id_of};

pub(super) fn biz_fail(error: BusinessError) -> JobError {
    JobError::retryable(error.message())
}

fn names(step: &Value, key: &str) -> Vec<String> {
    step.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The contact steps; `None` = not a contact step.
pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    let db = &ctx.run.deps.db;
    let (ws, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    match step_type {
        "addContactTag" => {
            tag::attach_by_names_to_contacts(db, ws, &[contact_id], &names(ctx.step, "tags"))
                .await
                .map_err(biz_fail)?;
        }
        "removeContactTag" => {
            tag::detach_by_names_from_contacts(db, ws, &[contact_id], &names(ctx.step, "tags"))
                .await
                .map_err(biz_fail)?;
        }
        "setCustomField" => set_custom_field(ctx).await,
        "clearCustomField" => {
            contact_custom_field::delete_by_key(
                db,
                ws,
                contact_id,
                ctx.str("inputFieldId").unwrap_or(""),
                true,
            )
            .await
            .map_err(biz_fail)?;
        }
        "addContactNotes" => {
            contact_note::create(db, ws, contact_id, ctx.str("content").unwrap_or(""), None)
                .await
                .map_err(biz_fail)?;
        }
        "markEmailVerified" => set_contact_flag(ctx, "emailVerified", true).await?,
        "optInEmail" => set_contact_flag(ctx, "emailOptIn", true).await?,
        "optOutEmail" => set_contact_flag(ctx, "emailOptIn", false).await?,
        "deleteContact" => {
            contact::service::delete_and_record(db, ws, &[contact_id])
                .await
                .map_err(biz_fail)?;
        }
        "blockContact" => {
            contact::service::block_and_record(db, ws, contact_id, now_ms())
                .await
                .map_err(biz_fail)?;
        }
        "subscribeSequence" => {
            if let Some(sequence_id) = ctx.step.get("sequenceId").and_then(id_of) {
                contact_sequence::subscribe_contacts(db, ws, &[contact_id], &[sequence_id])
                    .await
                    .map_err(biz_fail)?;
            }
        }
        "unsubscribeSequence" => {
            if let Some(sequence_id) = ctx.step.get("sequenceId").and_then(id_of) {
                contact_sequence::remove_contact_sequences_for_contacts(
                    db,
                    ws,
                    &[contact_id],
                    &[sequence_id],
                )
                .await
                .map_err(biz_fail)?;
            }
        }
        "subscribeBroadcast" => set_broadcast_subscription(ctx, true).await?,
        "unsubscribeBroadcast" => set_broadcast_subscription(ctx, false).await?,
        _ => return Ok(None),
    }
    Ok(Some(StepStatus::Success))
}

/// `contactService.update({ <column>: value })`.
async fn set_contact_flag(
    ctx: &StepCtx<'_, '_>,
    column: &'static str,
    value: bool,
) -> Result<(), JobError> {
    let (ws, id) = (ctx.workspace_id(), ctx.contact_id());
    ctx.run
        .deps
        .db
        .write(move |tx| {
            let changed = tx.execute(
                &format!(
                    r#"UPDATE "Contact" SET "{column}" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "workspaceId" = ?4"#
                ),
                params![value, now_ms(), id, ws],
            )?;
            if changed == 0 {
                return Err(chatbotx_db::DbError::other("Contact not found"));
            }
            Ok(())
        })
        .await
        .map_err(db_fail)
}

/// `contactService.setBroadcastSubscription`: subscribing keeps the first
/// subscription time.
async fn set_broadcast_subscription(
    ctx: &StepCtx<'_, '_>,
    subscribed: bool,
) -> Result<(), JobError> {
    let (ws, id) = (ctx.workspace_id(), ctx.contact_id());
    ctx.run
        .deps
        .db
        .write(move |tx| {
            if subscribed {
                tx.execute(
                    r#"UPDATE "Contact" SET "broadcastSubscribedAt" = ?1 WHERE "id" = ?2 AND "workspaceId" = ?3 AND "broadcastSubscribedAt" IS NULL"#,
                    params![now_ms(), id, ws],
                )?;
            } else {
                tx.execute(
                    r#"UPDATE "Contact" SET "broadcastSubscribedAt" = NULL WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws],
                )?;
            }
            Ok(())
        })
        .await
        .map_err(db_fail)
}

/// `setContactCustomField` + `applyCustomFieldOperation`: any failure is
/// logged and the flow goes on (a throw would drop the node's next steps).
async fn set_custom_field(ctx: &StepCtx<'_, '_>) {
    let field = ctx.str("inputFieldId").unwrap_or("").to_owned();
    let raw = ctx.str("value").unwrap_or("").to_owned();
    // `contactVariableService.replaceAll`: no spintax.
    let value = if raw.contains("{{") {
        match ctx.resolve_variables(&json!(raw), false).await {
            Ok(Value::String(text)) => text,
            Ok(_) => raw,
            Err(err) => {
                tracing::error!(err = %err, step_id = ctx.str("id"), input_field_id = %field, "Set custom field step failed; continuing with the remaining steps");
                return;
            }
        }
    } else {
        raw
    };
    let input = contact_custom_field::SetValueByKeyInput {
        workspace_id: ctx.workspace_id(),
        contact_id: ctx.contact_id(),
        keyword: field.clone(),
        value,
        source_timezone_override: ctx.str("timezone").map(str::to_owned),
        temporal_input_parsing: Some(TemporalInputParsing::Lenient),
        fill_empty_temporal_with_now: true,
        operation: ctx.str("operation").and_then(FieldOperationType::parse),
        touch_unchanged: false,
        allow_bot_fields: true,
    };
    match contact_custom_field::set_value_by_key(&ctx.run.deps.db, input).await {
        Ok(_) => {}
        Err(err) if err.code() == Some("invalidFieldOperation") => {
            tracing::warn!(reason = %err.message(), step_id = ctx.str("id"), input_field_id = %field, "Set custom field step rejected; continuing with the remaining steps");
        }
        Err(err) => {
            tracing::error!(err = %err.message(), step_id = ctx.str("id"), input_field_id = %field, "Set custom field step failed; continuing with the remaining steps");
        }
    }
}
