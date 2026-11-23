//! The `whatsappIdentityChange` (`handlers/whatsapp-identity-change.ts`) and
//! `providerAccountEvent` (`handlers/provider-account-event.ts`) integration
//! jobs.

use chatbotx_business::audit::{AuditActor, with_audit_context};
use chatbotx_business::connection::provider_account_event::{self as account, Request};
use chatbotx_business::contact_identity::{
    ChangeResult, RotationChange, adopt_phone_number_if_safe, change_primary_phone,
    rotate_scoped_user_id,
};
use chatbotx_business::events::Event;
use chatbotx_business::events::trigger;
use chatbotx_core::utils::connection::ProviderAccountEventKind;
use chatbotx_db::repo::integration_whatsapp::{
    PROVIDER_ACCOUNT_TARGET_BATCH_SIZE, ProviderAccountRouteScope, list_account_targets_by_waba_id,
};
use chatbotx_db::rusqlite::OptionalExtension as _;
use chatbotx_jobs::JobError;
use serde_json::Value;

use crate::{Deps, db_fail, id_of, is_blocked_workspace, str_of};

fn opt(v: &Value, key: &str) -> Option<String> {
    str_of(v, key).map(str::to_owned)
}

/// `handleWhatsappIdentityChange`.
pub(crate) async fn whatsapp_identity_change(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let identifier = str_of(data, "integrationIdentifier")
        .unwrap_or("")
        .to_owned();
    let lookup = identifier.clone();
    let inbox = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT i."id", i."workspaceId" FROM "IntegrationWhatsapp" t
                       JOIN "Inbox" i ON i."id" = t."inboxId" WHERE t."phoneNumberId" = ?1 LIMIT 1"#,
                    [lookup],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some((inbox_id, workspace_id)) = inbox else {
        return Err(JobError::unrecoverable(format!(
            "Integration not found: whatsapp {identifier}"
        )));
    };
    let change = data["payload"]["change"].clone();
    let kind = str_of(&change, "kind").unwrap_or("").to_owned();
    let message_id = str_of(&data["payload"], "messageId")
        .unwrap_or("")
        .to_owned();
    let (tx_change, tx_kind) = (change.clone(), kind.clone());
    let outcome = deps
        .db
        .write(move |tx| {
            let result = match tx_kind.as_str() {
                "userIdChanged" => rotate_scoped_user_id(
                    tx,
                    inbox_id,
                    &RotationChange {
                        previous_user_id: opt(&tx_change, "previousUserId"),
                        user_id: opt(&tx_change, "userId").unwrap_or_default(),
                        previous_parent_user_id: opt(&tx_change, "previousParentUserId"),
                        parent_user_id: opt(&tx_change, "parentUserId"),
                        previous_phone: opt(&tx_change, "previousPhone"),
                        new_phone: opt(&tx_change, "newPhone"),
                    },
                ),
                "phoneChanged" => change_primary_phone(
                    tx,
                    inbox_id,
                    str_of(&tx_change, "previousPhone").unwrap_or(""),
                    str_of(&tx_change, "newPhone").unwrap_or(""),
                    str_of(&tx_change, "userId"),
                ),
                _ => Ok(ChangeResult::Invalid),
            }
            .map_err(|e| chatbotx_db::DbError::other(e.to_string()))?;
            // `updateContactPhoneIfSafe`: a failure is logged, not thrown.
            let adoption = match &result {
                ChangeResult::Done {
                    contact_id,
                    phone_transition: Some(t),
                    ..
                } => match adopt_phone_number_if_safe(
                    tx,
                    workspace_id,
                    *contact_id,
                    t.previous_phone.as_deref(),
                    &t.new_phone,
                ) {
                    Ok(None) => {
                        tracing::info!(contact_id, "Whatsapp identity change phone number adoption skipped");
                        None
                    }
                    Ok(Some(adoption)) => Some((*contact_id, adoption)),
                    Err(err) => {
                        tracing::warn!(%err, contact_id, "Whatsapp identity change phone number update failed");
                        None
                    }
                },
                _ => None,
            };
            Ok((result, adoption))
        })
        .await
        .map_err(db_fail)?;
    let (result, adoption) = outcome;
    let status = match &result {
        ChangeResult::Invalid => "invalid",
        ChangeResult::NotFound => "notFound",
        ChangeResult::Done { status, .. } => status,
    };
    tracing::info!(
        change_kind = kind,
        identifier,
        message_id,
        outcome = status,
        "Whatsapp identity change processed"
    );
    if status == "invalid" {
        tracing::warn!(
            change_kind = kind,
            identifier,
            message_id,
            "Whatsapp identity change ignored: invalid identity transition"
        );
    }
    if let Some((contact_id, (before, after))) = adoption {
        let events = trigger::contact_info_changes(
            &workspace_id.to_string(),
            &contact_id.to_string(),
            [before.as_deref(), None],
            [Some(&after), None],
        );
        let bus = crate::order_payment::bus(deps);
        for event in events {
            bus.publish(Event::Trigger(event)).await;
        }
    }
    Ok(())
}

/// `ProviderAccountRouteScope` from the job's `routeScope`.
fn route_scope(value: &Value) -> Option<ProviderAccountRouteScope> {
    let app_id = || opt(value, "appId");
    match str_of(value, "kind")? {
        "platform" => Some(ProviderAccountRouteScope::Platform { app_id: app_id()? }),
        "credentialOwner" => Some(ProviderAccountRouteScope::CredentialOwner {
            owner_id: id_of(&value["ownerId"])?,
            app_id: app_id()?,
        }),
        "manualIntegration" => Some(ProviderAccountRouteScope::ManualIntegration {
            integration_id: id_of(&value["integrationId"])?,
            workspace_id: id_of(&value["workspaceId"])?,
        }),
        _ => None,
    }
}

/// `handleProviderAccountEvent`: every number of the account inside the
/// route's scope, batch by batch.
pub(crate) async fn provider_account_event(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("").to_owned();
    let event_name = str_of(data, "event").unwrap_or("");
    let Some(scope) = route_scope(&data["routeScope"]) else {
        tracing::error!(
            integration_type,
            event = event_name,
            "Provider account event dropped: job carries no route scope"
        );
        return Ok(());
    };
    if integration_type != "whatsapp" {
        tracing::warn!(
            integration_type,
            "Provider account event dropped: integration type has no target loader"
        );
        return Ok(());
    }
    let Some(event) = ProviderAccountEventKind::parse(event_name) else {
        return Err(JobError::unrecoverable(format!(
            "Unknown provider account event: {event_name}"
        )));
    };
    let account_id = str_of(&data["account"], "id").unwrap_or("").to_owned();
    let request = Request {
        integration_type: integration_type.clone(),
        event,
        reason: str_of(&data["detail"], "reason")
            .unwrap_or("unknown")
            .to_owned(),
        initiated_by: str_of(&data["detail"], "initiatedBy")
            .unwrap_or("unknown")
            .to_owned(),
        occurred_at: data["occurredAt"].as_i64().unwrap_or_default(),
    };
    let mut after = None;
    let mut handled = 0;
    loop {
        let (waba, scope_ref) = (account_id.clone(), scope.clone());
        let batch = deps
            .db
            .read(move |conn| {
                list_account_targets_by_waba_id(conn, &waba, &scope_ref, after)
                    .map_err(crate::repo_err)
            })
            .await
            .map_err(db_fail)?;
        for target in &batch {
            if is_blocked_workspace(&deps.db, &target.workspace_id.to_string()).await? {
                continue;
            }
            let actor = AuditActor {
                workspace_id: Some(target.workspace_id.to_string()),
                source: Some("integration:providerAccountEvent".into()),
                ..AuditActor::default()
            };
            with_audit_context(
                actor,
                account::apply_to_target(&deps.db, request.clone(), *target),
            )
            .await
            .map_err(|e| crate::fail(e.to_string()))?;
            handled += 1;
        }
        if (batch.len() as i64) < PROVIDER_ACCOUNT_TARGET_BATCH_SIZE {
            break;
        }
        after = batch.last().map(|t| t.integration_id);
    }
    tracing::info!(
        integration_type,
        account_id,
        event = event_name,
        targets = handled,
        "Provider account event handled"
    );
    Ok(())
}
