//! The WhatsApp templates and Flows screens (Q.D12):
//! `whatsappMessageTemplateAPIs` / `whatsappFlowAPIs`
//! (`features/integration-whatsapp/{message-templates,flows}/api/private.ts`) and
//! `syncMessageTemplateAction` / `syncWhatsappFlowsAction`; the coexist
//! switch `integrationWhatsappAPIs.setCoexistWhatsappAPI`.

use chatbotx_business::tx;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, arg, fail, invalid, public, query_of};
use crate::handlers::{internal_error, whatsapp_flows, whatsapp_templates};
use crate::orpc::OrpcError;

/// The workspace actions of this module.
pub(super) const ACTIONS: &[&str] = &["syncMessageTemplateAction", "syncWhatsappFlowsAction"];

const STATUSES: &[&str] = &["APPROVED", "PENDING", "REJECTED"];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let inp = In(input);
    let filters = || -> Result<(Option<i64>, Option<i64>), OrpcError> {
        Ok((inp.opt_id("inboxId")?, inp.opt_id("integrationWhatsappId")?))
    };
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(match path {
        "whatsappMessageTemplateAPIs.listWhatsappMessageTemplatesInternalAPI" => {
            let run = async {
                let (inbox, integration) = filters()?;
                let status = inp.opt_str("status")?;
                if status.as_deref().is_some_and(|s| !STATUSES.contains(&s)) {
                    return Err(invalid());
                }
                let rows = whatsapp_templates::list(db, ws, inbox, integration, status)
                    .await
                    .map_err(fail)?;
                Ok(Value::Array(rows))
            };
            run.await
        }
        "whatsappMessageTemplateAPIs.searchMetaCatalogProductsAPI" => {
            let query = query_of(input, &["keyword", "retailerId"]);
            public(
                ctx,
                "whatsappTemplates.searchCatalogProducts",
                &[],
                Some(&query),
                None,
            )
            .await
            .map_err(|error| match error.status {
                422 => invalid(),
                _ => error,
            })
        }
        "whatsappFlowAPIs.listWhatsappFlowsInternalAPI" => match filters() {
            Ok((inbox, integration)) => whatsapp_flows::list(db, ws, inbox, integration, true)
                .await
                .map(Value::Array)
                .map_err(fail),
            Err(error) => Err(error),
        },
        "whatsappFlowAPIs.getWhatsappFlowScreensInternalAPI" => match inp.id("flowId") {
            Ok(flow_id) => whatsapp_flows::get_screens(db, &ctx.api.graph_url, ws, flow_id)
                .await
                .map(|screens| json!({ "screens": screens }))
                .map_err(fail),
            Err(error) => Err(error),
        },
        "integrationWhatsappAPIs.setCoexistWhatsappAPI" => {
            let run = async {
                let integration_id = inp.id("integrationId")?;
                let enabled = inp.opt_bool("enabled")?.ok_or_else(invalid)?;
                let ai_reads = inp.opt_bool("aiReadsSyncedHistory")?.unwrap_or(false);
                let result = chatbotx_business::coexist::set_whatsapp_coexist(
                    db,
                    &ctx.api.graph_url,
                    ws,
                    integration_id,
                    enabled,
                    ai_reads,
                )
                .await
                .map_err(fail)?;
                Ok(crate::handlers::whatsapp_coexist::rpc_output(&result))
            };
            run.await
        }
        _ => return None,
    })
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(sync(ctx, name, args).await)
}

/// Both actions bind `[workspaceId, integrationWhatsappId]`; a number of another
/// workspace is the original's plain `Error` (the default server error).
async fn sync(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let id = In(&json!({ "id": arg(args, 1) })).id("id")?;
    let ws = ctx.workspace_id;
    let integration = tx::read(&ctx.api.db, move |conn| {
        Ok(chatbotx_db::repo::integration_whatsapp::find_by_id_for_workspace(conn, id, ws)?)
    })
    .await
    .map_err(fail)?
    .ok_or_else(|| internal_error("Whatsapp integration not found"))?;
    let (db, graph_url) = (&ctx.api.db, ctx.api.graph_url.as_str());
    let result = if name == "syncMessageTemplateAction" {
        whatsapp_templates::sync_integration(db, graph_url, &integration).await
    } else {
        whatsapp_flows::sync_integration(db, graph_url, &integration).await
    };
    result.map_err(|f| crate::handlers::messenger_templates::reply_error(f, &[]))?;
    Ok(Value::Null)
}
