//! `find-places-near-me.ts` (`findPlacesNearMe`): the place nearest to the
//! contact's shared location (upstream Get User Data steps) as a WhatsApp
//! location pin. Every "cannot send" case is a silent skip.

use chatbotx_business::place_near_me::contact_location::ContactLocationInput;
use chatbotx_business::place_near_me::find_contact_location;
use chatbotx_business::place_near_me::location::{FindNearestPlace, find_nearest};
use chatbotx_core::flow_config::find_upstream_location_field_keys;
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::StepCtx;
use super::contact::biz_fail;
use crate::flow::{StepStatus, send_flow_message};
use crate::id_of;

pub(super) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let skip = |reason: &str| {
        tracing::info!(
            conversation_id = ctx.run.conversation.id,
            step_id = ctx.str("id"),
            reason,
            "findPlacesNearMe: skipped"
        );
        Ok(StepStatus::Success)
    };
    let run = ctx.run;
    if run.contact_inbox.channel != "whatsapp" {
        return skip("notWhatsapp");
    }
    let slice = |v: &Value| v.as_array().map(Vec::as_slice).unwrap_or(&[]).to_vec();
    let keys = find_upstream_location_field_keys(
        &slice(&run.flow_version.nodes.0),
        &slice(&run.flow_version.edges.0),
        ctx.str("id").unwrap_or(""),
    );
    if keys.is_empty() {
        return skip("noLocationSource");
    }
    let db = &run.deps.db;
    let origin = find_contact_location(
        db,
        ContactLocationInput {
            workspace_id: ctx.workspace_id(),
            contact_id: ctx.contact_id(),
            keys,
        },
    )
    .await
    .map_err(biz_fail)?;
    let Some(origin) = origin else {
        return skip("noLocationValue");
    };
    let place = find_nearest(
        db,
        FindNearestPlace {
            workspace_id: ctx.workspace_id(),
            category_id: ctx.step.get("placeCategoryId").and_then(id_of),
            latitude: origin.latitude,
            longitude: origin.longitude,
        },
    )
    .await
    .map_err(biz_fail)?;
    let Some(place) = place else {
        return skip("noPlaceFound");
    };
    let pin = json!({
        "id": ctx.step.get("id"),
        "nodeId": ctx.step.get("nodeId"),
        "stepType": "whatsappLocation",
        "latitude": place.latitude,
        "longitude": place.longitude,
        "name": place.name,
    });
    send_flow_message(run, &pin, None, None).await?;
    Ok(StepStatus::Success)
}
