//! `features/sequences/api/public.ts` (P3.6-D2 slice contact sequences):
//! `sequences.create` only.

use chatbotx_business::sequence::create;
use serde_json::json;

use super::input::{Input, Str};
use super::{ApiState, Call, ErrorMap, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["sequences.create"];

/// `possibleErrorsOnCreatingInFolder`.
const CREATING_IN_FOLDER: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "sequences.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", Str::trimmed(1, 255));
            let folder_id = input.nullish_id("folderId");
            input.finish()?;
            let sequence_id = create(
                &state.db,
                call.workspace_id,
                &name.unwrap_or_default(),
                folder_id.flatten(),
            )
            .await
            .map_err(|e| business_error(e, CREATING_IN_FOLDER))?;
            Ok(json(201, json!({ "sequenceId": sequence_id.to_string() })))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
