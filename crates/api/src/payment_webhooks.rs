//! `apps/builder/src/app/api/payments/**/webhook/route.ts` +
//! `packages/business/src/order-payment/webhooks.ts`: the Stripe and PayPal
//! webhooks of the web shop. The signature is checked with the stored
//! secret; a payment event is queued (`confirmOrderPayment`) and answered
//! 200 at once, a bad signature 400, no secret 404, and an error 500 so the
//! provider delivers it again.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chatbotx_business::payment_provider;
use chatbotx_business::payments::{
    ErrorKind, PResult, PaymentMetadata, WebhookEvent, paypal, stripe,
};
use chatbotx_business::tx;
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::json;

use crate::handlers::ApiState;

/// The routes.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route(
            "/api/payments/stripe/workspace/{workspace_id}/webhook",
            post(stripe_account),
        )
        .route(
            "/api/payments/paypal/{workspace_id}/webhook",
            post(paypal_webhook),
        )
        .route(
            "/api/payments/stripe/webhook/{credential_id}",
            post(stripe_connect),
        )
        .with_state(state)
}

/// `ID_PATTERN` (`^\d+$`).
fn id(text: &str) -> Option<i64> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn reply(status: u16) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    (code, axum::Json(json!({ "received": status == 200 }))).into_response()
}

fn failed(error: impl std::fmt::Display) -> Response {
    tracing::error!(%error, "[payments] the webhook could not be handled");
    reply(500)
}

fn now_secs() -> i64 {
    chatbotx_core::js::now_ms() / 1000
}

/// `toReceiptOrThrow`: a bad signature is 400, any other failure 500.
fn parsed(result: PResult<WebhookEvent>) -> Result<WebhookEvent, Response> {
    result.map_err(|error| {
        if error.kind == ErrorKind::SignatureInvalid {
            reply(400)
        } else {
            failed(error)
        }
    })
}

async fn enqueue(
    state: &ApiState,
    provider: &str,
    workspace_id: i64,
    event_id: &str,
    reference: &str,
    metadata: Option<&PaymentMetadata>,
) -> Response {
    let Some(jobs) = state.jobs.as_deref() else {
        return failed("the job store is off");
    };
    let data = json!({ "type": "confirmOrderPayment", "data": {
        "provider": provider,
        "workspaceId": workspace_id.to_string(),
        "reference": reference,
        "metadata": metadata,
    }});
    let opts = JobOptions {
        job_id: Some(format!("order-payment-event-{provider}-{event_id}")),
        ..JobOptions::default()
    };
    match jobs
        .add(QueueName::Integration, "confirmOrderPayment", data, opts)
        .await
    {
        Ok(_) => reply(200),
        Err(error) => failed(error),
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// `receiveStripeAccountWebhook`: the store key's own webhook.
async fn stripe_account(
    State(state): State<Arc<ApiState>>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(ws) = id(&workspace_id) else {
        return reply(404);
    };
    let Some(enc) = state.encryption.as_deref().cloned() else {
        return reply(404);
    };
    let secret = match tx::read(&state.db, move |conn| {
        Ok(payment_provider::load_stripe_account_webhook(
            conn, &enc, ws,
        )?)
    })
    .await
    {
        Ok(Some(secret)) => secret,
        Ok(None) => return reply(404),
        Err(error) => return failed(error),
    };
    let signature = header(&headers, "stripe-signature");
    let event = match parsed(stripe::parse_account_webhook(
        &body,
        signature.as_deref(),
        &secret.webhook_secret,
        now_secs(),
    )) {
        Ok(event) => event,
        Err(response) => return response,
    };
    let WebhookEvent::Checkout {
        event_id,
        reference,
        metadata: Some(metadata),
        ..
    } = event
    else {
        return reply(200);
    };
    if metadata.workspace_id != ws.to_string() {
        return reply(200);
    }
    enqueue(&state, "stripe", ws, &event_id, &reference, Some(&metadata)).await
}

/// `receivePaypalWebhook`.
async fn paypal_webhook(
    State(state): State<Arc<ApiState>>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(ws) = id(&workspace_id) else {
        return reply(404);
    };
    let Some(enc) = state.encryption.as_deref().cloned() else {
        return reply(404);
    };
    let secret = match tx::read(&state.db, move |conn| {
        Ok(payment_provider::load_paypal_secret(conn, &enc, ws)?)
    })
    .await
    {
        Ok(Some(secret)) => secret,
        Ok(None) => return reply(404),
        Err(error) => return failed(error),
    };
    let event = match parsed(
        paypal::parse_webhook(&body, |name| header(&headers, name), &secret, None).await,
    ) {
        Ok(event) => event,
        Err(response) => return response,
    };
    let WebhookEvent::Checkout {
        event_id,
        reference,
        metadata,
        ..
    } = event
    else {
        return reply(200);
    };
    if metadata
        .as_ref()
        .is_some_and(|m| m.workspace_id != ws.to_string())
    {
        tracing::warn!(
            workspace_id = ws,
            event_id,
            "[payments] a PayPal event named another workspace"
        );
        return reply(200);
    }
    enqueue(
        &state,
        "paypal",
        ws,
        &event_id,
        &reference,
        metadata.as_ref(),
    )
    .await
}

/// `receiveStripeWebhook`: the platform's Stripe Connect webhook.
async fn stripe_connect(
    State(state): State<Arc<ApiState>>,
    Path(credential_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(credential) = id(&credential_id) else {
        return reply(404);
    };
    let Some(enc) = state.encryption.as_deref().cloned() else {
        return reply(404);
    };
    let secrets = match tx::read(&state.db, move |conn| {
        payment_provider::load_stripe_connect_webhook(conn, &enc, credential)
    })
    .await
    {
        Ok(Some(secrets)) => secrets,
        Ok(None) => return reply(404),
        Err(error) => return failed(error),
    };
    let signature = header(&headers, "stripe-signature");
    let event = match parsed(stripe::parse_connect_webhook(
        &body,
        signature.as_deref(),
        &secrets.1,
        now_secs(),
    )) {
        Ok(event) => event,
        Err(response) => return response,
    };
    let (event_id, reference, metadata, account) = match event {
        WebhookEvent::Deauthorized {
            account,
            created_at,
            ..
        } => {
            return match payment_provider::handle_stripe_deauthorized(
                &state.db, credential, &account, created_at,
            )
            .await
            {
                Ok(()) => reply(200),
                Err(error) => failed(error),
            };
        }
        // Events of the platform's own account carry no `account`: not a store's.
        WebhookEvent::Checkout {
            event_id,
            reference,
            metadata,
            account: Some(account),
        } => (event_id, reference, metadata, account),
        _ => return reply(200),
    };
    let store = match tx::read(&state.db, move |conn| {
        Ok(payment_provider::find_by_stripe_account(
            conn, credential, &account,
        )?)
    })
    .await
    {
        Ok(Some(store)) => store,
        Ok(None) => return reply(200),
        Err(error) => return failed(error),
    };
    if metadata
        .as_ref()
        .is_some_and(|m| m.workspace_id != store.workspace_id.to_string())
    {
        tracing::warn!(
            workspace_id = store.workspace_id,
            event_id,
            "[payments] a Stripe event named another workspace"
        );
        return reply(200);
    }
    enqueue(
        &state,
        "stripe",
        store.workspace_id,
        &event_id,
        &reference,
        metadata.as_ref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::id;

    #[test]
    fn ids_are_digits_only() {
        assert_eq!(id("123"), Some(123));
        assert_eq!(id(""), None);
        assert_eq!(id("12a"), None);
        assert_eq!(id("-1"), None);
    }
}
