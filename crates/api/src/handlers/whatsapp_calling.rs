//! `whatsappCallingPublicRouter` (`features/integration-whatsapp/calling/api/public.ts`,
//! scope `integrations`): `GET` / `PATCH /v1/whatsapp-channels/{id}/calling` and
//! `PUT /v1/whatsapp-channels/{id}/calling/hours`.
use chatbotx_business::BusinessError;
use chatbotx_business::whatsapp_voip_actions::{
    self as calling, CallingSettingsInput, ENGLISH_CALLING_MESSAGES,
};
use serde_json::Value;

use super::input::{Input, Num, key};
use super::tags::digits_at;
use super::{ApiState, Call, ErrorMap, FINDING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "whatsappChannels.getCallingSettings",
    "whatsappChannels.updateCallingSettings",
    "whatsappChannels.updateCallHours",
];

/// `possibleErrorsOnUpdatingWhatsappCalling`.
const UPDATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("validation", 422),
    ("whatsappCallingUpstream", 502),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

fn channel_id(input: &mut Input) -> i64 {
    let value = input.get("id").cloned();
    digits_at(input, &key("id"), value.as_ref(), true).unwrap_or_default()
}

pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "whatsappChannels.getCallingSettings" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = channel_id(&mut input);
            input.finish()?;
            calling::calling_settings_resource(&state.db, ws, id)
                .await
                .and_then(|found| {
                    found
                        .ok_or_else(|| BusinessError::not_found(ENGLISH_CALLING_MESSAGES.not_found))
                })
                .map(|resource| json(200, resource))
                .map_err(|e| business_error(e, FINDING))
        }
        "whatsappChannels.updateCallingSettings" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = channel_id(&mut input);
            let toggle = ["ENABLED", "DISABLED"];
            let status = input.get("status").cloned();
            let status = input.enum_at(&key("status"), status.as_ref(), &toggle, false);
            let icon = input.get("callIconVisibility").cloned();
            let icon = input.enum_at(
                &key("callIconVisibility"),
                icon.as_ref(),
                &["DEFAULT", "DISABLE_ALL"],
                false,
            );
            let callback = input.get("callbackPermissionStatus").cloned();
            let callback = input.enum_at(
                &key("callbackPermissionStatus"),
                callback.as_ref(),
                &toggle,
                false,
            );
            let recording = input.opt_bool("recordingEnabled");
            let retention = input.opt_number(
                "callRecordingRetentionDays",
                Num {
                    int: true,
                    min: Some(1.0),
                    max: Some(3650.0),
                },
            );
            let transcription = input.opt_bool("callTranscriptionEnabled");
            let inbound = input.opt_bool("inboundCallsEnabled");
            input.finish()?;
            let settings = CallingSettingsInput {
                status,
                call_icon_visibility: icon,
                callback_permission_status: callback,
                recording_enabled: recording,
                #[allow(clippy::cast_possible_truncation)]
                call_recording_retention_days: retention.map(|d| d as i64),
                call_transcription_enabled: transcription,
                inbound_calls_enabled: inbound,
            };
            calling::update_calling_settings(
                &state.db,
                &state.graph_url,
                ws,
                id,
                settings,
                &ENGLISH_CALLING_MESSAGES,
            )
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, UPDATING))
        }
        _ => {
            let mut input = Input::from_body(call.params, call.body);
            let id = channel_id(&mut input);
            input.finish()?;
            // Re-validated with the builder's schema (ranges, overlaps, open hours).
            // ponytail: a shape error is the same 422 `validation` exception, not oRPC's issue list.
            let body: Value = serde_json::from_slice(call.body).unwrap_or(Value::Null);
            let form = calling::parse_call_hours(&body).map_err(|issue| {
                business_error(
                    BusinessError::exception(
                        calling::call_hours_issue_message(&issue),
                        "validation",
                        422,
                    ),
                    UPDATING,
                )
            })?;
            calling::update_call_hours(
                &state.db,
                &state.graph_url,
                ws,
                id,
                &form,
                &ENGLISH_CALLING_MESSAGES,
            )
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, UPDATING))
        }
    }
}
