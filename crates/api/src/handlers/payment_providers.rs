//! `features/payment-providers/api/public.ts`: list / getConnectLink /
//! connectStripeKey / disconnect.

use chatbotx_business::BusinessError;
use chatbotx_business::payment_provider::{self, Ctx};
use chatbotx_core::utils::web_url::join_app_url;
use chatbotx_db::repo;
use serde_json::json;

use super::input::{Input, Str, key};
use super::{ApiState, Call, ErrorMap, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "paymentProviders.list",
    "paymentProviders.getConnectLink",
    "paymentProviders.connectStripeKey",
    "paymentProviders.disconnect",
];

/// `possibleErrorsOnConnectingPaymentProvider`.
const CONNECTING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("paymentProviderNotConnected", 400),
    ("paymentProviderCredentialsRejected", 400),
    ("paymentProviderRejected", 400),
    ("paymentProviderUnavailable", 503),
    ("paymentAmountUnsupported", 400),
    ("stripePlatformNotConfigured", 400),
    ("stripeAccountAlreadyConnected", 409),
    ("stripeSecretKeyInvalid", 400),
    ("paymentProviderInUse", 409),
];

fn ctx(state: &ApiState) -> Result<Ctx<'_>, BusinessError> {
    let encryption = state
        .encryption
        .as_deref()
        .ok_or_else(|| BusinessError::Repo(repo::Error::msg("ENCRYPTION_KEY is not set")))?;
    Ok(Ctx {
        db: &state.db,
        encryption,
        app_url: &state.app_url,
    })
}

fn provider(input: &mut Input) -> Option<&'static str> {
    let value = input.get("provider").cloned();
    match input
        .enum_at(
            &key("provider"),
            value.as_ref(),
            payment_provider::PROVIDERS,
            true,
        )?
        .as_str()
    {
        "stripe" => Some("stripe"),
        _ => Some("paypal"),
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "paymentProviders.list" => {
            let data = payment_provider::list(&state.db, ws)
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            let available = match state.encryption.as_deref() {
                Some(encryption) => {
                    let owner =
                        chatbotx_business::tenant::resolve_credential_owner_id_for_workspace(
                            &state.db, ws, None,
                        )
                        .await
                        .map_err(|e| business_error(e, super::LISTING))?;
                    payment_provider::is_stripe_available(&state.db, encryption, owner).await
                }
                None => false,
            };
            Ok(json(
                200,
                json!({ "data": data, "stripeAvailable": available }),
            ))
        }
        "paymentProviders.getConnectLink" => {
            let mut input = Input::from_query(call.params, call.query);
            let provider = provider(&mut input);
            input.finish()?;
            let path = format!(
                "/space/{ws}/settings/integrations/{}",
                provider.unwrap_or_default()
            );
            Ok(json(
                200,
                json!({ "url": join_app_url(&state.app_url, &path) }),
            ))
        }
        "paymentProviders.connectStripeKey" => {
            let mut input = Input::from_body(call.params, call.body);
            let secret_key = input.string("secretKey", Str::trimmed(1, 512));
            input.finish()?;
            let ctx = ctx(state).map_err(|e| business_error(e, CONNECTING))?;
            let view =
                payment_provider::connect_stripe_key(ctx, ws, &secret_key.unwrap_or_default())
                    .await
                    .map_err(|e| business_error(e, CONNECTING))?;
            state
                .record_audit("connect", "connected Stripe for the web shop")
                .await;
            Ok(json(200, view))
        }
        "paymentProviders.disconnect" => {
            let mut input = Input::from_query(call.params, call.query);
            let provider = provider(&mut input);
            input.finish()?;
            let provider = provider.unwrap_or("stripe");
            let ctx = ctx(state).map_err(|e| business_error(e, CONNECTING))?;
            if payment_provider::disconnect(ctx, ws, provider)
                .await
                .map_err(|e| business_error(e, CONNECTING))?
            {
                state
                    .record_audit("disconnect", &format!("disconnected {provider}"))
                    .await;
            }
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
