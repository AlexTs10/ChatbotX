//! `integration/handlers/condition.ts` (`handleCondition`) and
//! `contactService.matchesContactFilter`.

use chatbotx_core::utils::datetime::resolve_filter_timezone;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::queries::contact_filter::{
    ContactWhereInput, FilterCriteria, build_contact_where, contact_filter_has_predicate,
};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::StepCtx;
use crate::flow::{StepStatus, seek_connected_node};

/// `matchesContactFilter`: the contact matches the case's filter.
async fn matches(ctx: &StepCtx<'_, '_>, criteria: FilterCriteria) -> Result<bool, String> {
    let (ws, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    ctx.run
        .deps
        .db
        .read(move |conn| {
            let mut criteria = criteria;
            if !criteria.conditions.is_empty() && !contact_filter_has_predicate(&criteria, Some(ws))
            {
                return Ok(false);
            }
            // `withWorkingHoursTimezone`.
            if criteria
                .conditions
                .iter()
                .any(|c| c.get("field").and_then(Value::as_str) == Some("isWithinWorkingHours"))
            {
                let timezone: Option<String> = conn
                    .query_row(
                        r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                        params![ws],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                criteria.working_hours_timezone =
                    Some(resolve_filter_timezone(timezone.as_deref()));
            }
            let mut sql = build_contact_where(&ContactWhereInput {
                workspace_id: ws,
                keyword: None,
                contact_filter: Some(criteria),
                now: now_ms(),
            });
            let mut query = chatbotx_db::repo::sql::Sql::raw(r#"SELECT 1 FROM "Contact" WHERE ("#);
            sql.push(r#") AND "Contact"."id" = "#).push_bind(contact_id);
            query.append(sql);
            Ok(!query
                .query_map(conn, |_| Ok(()))
                .map_err(crate::repo_err)?
                .is_empty())
        })
        .await
        .map_err(|e| e.to_string())
}

/// The first case whose filter matches, else `otherwiseId`; a failure
/// routes to otherwise.
async fn matched_handle(ctx: &StepCtx<'_, '_>) -> String {
    let otherwise = ctx.str("otherwiseId").unwrap_or("").to_owned();
    let cases = ctx.step.get("cases").cloned().unwrap_or(json!([]));
    // ponytail: `refreshContactProfileSnapshot` for the Instagram follower
    // fields needs the channel profile lookup (Q.I1); the stored snapshot is
    // used until then.
    let evaluated = async {
        let resolved = ctx.resolve_variables(&cases, false).await?;
        for case in resolved.as_array().into_iter().flatten() {
            let conditions = case.get("conditions").and_then(Value::as_array);
            if conditions.is_none_or(Vec::is_empty) {
                continue;
            }
            let criteria: FilterCriteria = serde_json::from_value(json!({
                "operator": case.get("operator").cloned().unwrap_or(json!("and")),
                "conditions": conditions,
                "timezone": case.get("timezone"),
            }))
            .map_err(|e| e.to_string())?;
            if matches(ctx, criteria).await? {
                return Ok(case.get("id").and_then(Value::as_str).map(str::to_owned));
            }
        }
        Ok::<_, String>(None)
    }
    .await;
    match evaluated {
        Ok(Some(id)) => id,
        Ok(None) => otherwise,
        Err(err) => {
            tracing::error!(
                err = %err,
                conversation_id = ctx.run.conversation.id,
                step_id = ctx.str("id"),
                "[handleCondition] filter evaluation failed; routing to otherwise"
            );
            otherwise
        }
    }
}

/// `handleCondition`: starts the node connected to the matched handle.
pub(super) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let handle = matched_handle(ctx).await;
    let Some(node_id) = seek_connected_node(&ctx.run.flow_version.edges.0, &handle) else {
        return Ok(StepStatus::Success);
    };
    let job = ctx.send_flow_job(
        &node_id,
        &[
            "metadata",
            "appointmentId",
            "trackingContext",
            "sendFrom",
            "sentByUserId",
            "startedByContactMessage",
            "nodeVisits",
            "commentAnchor",
        ],
    );
    ctx.enqueue(job).await?;
    Ok(StepStatus::Success)
}
