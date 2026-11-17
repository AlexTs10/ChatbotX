//! `createCapiRoutes` (`features/channel-integrations/api/public.ts`):
//! `{messenger,instagram,whatsapp}Channels.{setCapiDataset, disconnectCapi,
//! setCapiTestEventCode, sendCapiTestEvent}` (scope `channels`).

use chatbotx_business::meta_capi::{self as service, CapiChannel};
use chatbotx_core::utils::meta_capi::CAPI_TEST_MESSAGING_ID_MAX_LENGTH;
use serde_json::{Value, json};

use super::input::{Input, Str};
use super::{ApiState, Call, ErrorMap, MUTATING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "messengerChannels.setCapiDataset",
    "messengerChannels.disconnectCapi",
    "messengerChannels.setCapiTestEventCode",
    "messengerChannels.sendCapiTestEvent",
    "instagramChannels.setCapiDataset",
    "instagramChannels.disconnectCapi",
    "instagramChannels.setCapiTestEventCode",
    "instagramChannels.sendCapiTestEvent",
    "whatsappChannels.setCapiDataset",
    "whatsappChannels.disconnectCapi",
    "whatsappChannels.setCapiTestEventCode",
    "whatsappChannels.sendCapiTestEvent",
];

/// `possibleErrorsOnSettingCapiDataset`.
const SETTING_DATASET: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("capiRequestFailed", 400),
];
/// `possibleErrorsOnSendingCapiTestEvent`.
const SENDING_TEST_EVENT: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("capiRequestFailed", 400),
    ("capiTestEventRefused", 422),
];

/// `TEST_EVENT_CODE_PATTERN` (`/^[A-Za-z0-9_-]*$/`).
fn test_event_code(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let Some((group, operation)) = route_id.split_once('.') else {
        return Err(crate::routes::not_ported_error());
    };
    let channel = match group {
        "messengerChannels" => CapiChannel::Messenger,
        "instagramChannels" => CapiChannel::Instagram,
        "whatsappChannels" => CapiChannel::Whatsapp,
        _ => return Err(crate::routes::not_ported_error()),
    };
    let db = &state.db;
    let ws = call.workspace_id;
    let mut input = Input::from_body(call.params, call.body);
    let id = input.id("id");
    match operation {
        "setCapiDataset" => {
            let dataset_id = input.opt_string(
                "datasetId",
                Str {
                    trim: true,
                    min: Some(1),
                    ..Str::default()
                },
            );
            input.finish()?;
            service::set_dataset(db, channel, ws, id.unwrap_or_default(), dataset_id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, SETTING_DATASET))
        }
        "disconnectCapi" => {
            input.finish()?;
            service::disconnect(db, channel, ws, id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "setCapiTestEventCode" => {
            let spec = Str {
                trim: true,
                max: Some(64),
                pattern: Some(test_event_code),
                ..Str::default()
            };
            let code = if input.get("testEventCode") == Some(&Value::Null) {
                None
            } else {
                input.string("testEventCode", spec)
            };
            input.finish()?;
            let code = code.filter(|c| !c.is_empty());
            service::set_test_event_code(db, channel, ws, id.unwrap_or_default(), code)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "sendCapiTestEvent" => {
            let messaging_id = input.string(
                "messagingId",
                Str::trimmed(1, CAPI_TEST_MESSAGING_ID_MAX_LENGTH),
            );
            input.finish()?;
            service::send_test_event(
                db,
                channel,
                ws,
                id.unwrap_or_default(),
                &messaging_id.unwrap_or_default(),
            )
            .await
            .map(|()| json(200, json!({ "success": true })))
            .map_err(|e| business_error(e, SENDING_TEST_EVENT))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
