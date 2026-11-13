//! `features/ecommerce-settings/api/public.ts`: `ecommerceSettings.get` /
//! `update` (+ `lib/get-ecommerce-settings.ts`).

use chatbotx_business::ecommerce_settings;
use serde_json::{Value, json};

use super::input::Input;
use super::{ApiState, Call, ErrorMap, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["ecommerceSettings.get", "ecommerceSettings.update"];

/// `possibleErrorsOnUpdatingEcommerceSettings`.
const UPDATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("ecommerceProFeatureRequired", 403),
    ("paymentProviderNotConnected", 400),
    ("paymentCurrencyUnsupported", 400),
    ("externalStoreNotConnected", 400),
];

/// `getEcommerceSettings`: the settings, `proFeatures`, `productsInOtherCurrency`.
async fn resource(state: &ApiState, workspace_id: i64, errors: ErrorMap) -> Reply {
    let db = &state.db;
    let mut settings = ecommerce_settings::get(db, workspace_id)
        .await
        .map_err(|e| business_error(e, errors))?;
    let currency = settings["payments"]["currency"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let other = ecommerce_settings::count_products_in_other_currency(db, workspace_id, &currency)
        .await
        .map_err(|e| business_error(e, errors))?;
    if let Value::Object(map) = &mut settings {
        map.insert("proFeatures".into(), ecommerce_settings::pro_features());
        map.insert("productsInOtherCurrency".into(), json!(other));
    }
    Ok(json(200, settings))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "ecommerceSettings.get" => resource(state, ws, super::FINDING).await,
        "ecommerceSettings.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let body = Value::Object(input.obj.clone());
            let patch = match ecommerce_settings::parse_patch(&body) {
                Ok(patch) => Some(patch),
                Err(issues) => {
                    input.extend_issues(issues);
                    None
                }
            };
            input.finish()?;
            let patch = patch.unwrap_or_default();
            ecommerce_settings::update(&state.db, ws, patch)
                .await
                .map_err(|e| business_error(e, UPDATING))?;
            resource(state, ws, UPDATING).await
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
