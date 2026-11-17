//! `inbound-webhooks-public-api.test.ts` + `inbound-webhook-route.test.ts`
//! (Q.D13), the parts without `previewMapping` / the worker: CRUD, config
//! save, settings, token, request log and the `/api/hooks/{token}` intake.

mod handlers_common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chatbotx_api::inbound_webhook_intake::router;
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};
use tower::ServiceExt as _;

async fn deliver(
    fx: &handlers_common::Fixture,
    token: &str,
    body: &str,
    key: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/api/hooks/{token}"))
        .header("x-forwarded-for", "203.0.113.7");
    if let Some(key) = key {
        req = req.header("idempotency-key", key);
    }
    let response = router(Arc::new(fx.state.clone()))
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn token_of(resource: &Value) -> String {
    let url = resource["endpointUrl"].as_str().unwrap();
    url.rsplit('/').next().unwrap().to_owned()
}

#[tokio::test]
async fn inbound_webhook_crud_settings_and_intake() {
    let fx = fixture().await;
    let (status, created) = call(
        &fx,
        "inboundWebhooks.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "  Orders  " })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["name"], "Orders");
    assert_eq!(created["active"], false);
    assert_eq!(created["inboxResolution"], Value::Null);
    let token = token_of(&created);
    assert!(token.starts_with("iwh_"), "{token}");
    let id = created["id"].as_str().unwrap().to_owned();

    // Another workspace cannot see it.
    let (status, _) = call(
        &fx,
        "inboundWebhooks.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);

    // Activation of an empty config is refused with the first issue.
    let (status, out) = call(
        &fx,
        "inboundWebhooks.updateSettings",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "active": true })),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (422, json!("validation.inboundWebhook.inboxRequired")),
        "{out}"
    );

    // An inactive webhook does not accept deliveries; a bad shape is a 404.
    let (status, out) = deliver(&fx, &token, "{}", None).await;
    assert_eq!(
        (status, out["code"].clone()),
        (StatusCode::FORBIDDEN, json!("WEBHOOK_INACTIVE"))
    );
    let (status, _) = deliver(&fx, "iwh_short", "{}", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Config save: `sampleJson` is required, paths are stored canonical,
    // a custom-field target must exist in the workspace.
    let config = json!({
        "inboxResolution": { "mode": "payload", "channel": "messenger", "identifierKind": "pageId", "jsonPath": "page" },
        "identifiers": [{ "kind": "email", "jsonPath": "email" }],
        "fieldMappings": [{ "jsonPath": "$['name']", "target": "first_name" }],
        "actions": [],
    });
    let (status, out) = call(
        &fx,
        "inboundWebhooks.update",
        WS,
        &[("id", &id)],
        None,
        Some(config.clone()),
    )
    .await;
    assert_eq!(
        (status, out["code"].clone()),
        (422, json!("invalidRequestData")),
        "{out}"
    );
    let mut bad = config.clone();
    bad["sampleJson"] = Value::Null;
    bad["fieldMappings"][0]["target"] = json!("999");
    let (status, out) = call(
        &fx,
        "inboundWebhooks.update",
        WS,
        &[("id", &id)],
        None,
        Some(bad),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (422, json!("validation.inboundWebhook.unknownFieldTarget")),
        "{out}"
    );
    let mut good = config;
    good["sampleJson"] = Value::Null;
    let (status, out) = call(
        &fx,
        "inboundWebhooks.update",
        WS,
        &[("id", &id)],
        None,
        Some(good),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["inboxResolution"]["jsonPath"], "$.page");
    assert_eq!(out["identifiers"][0]["jsonPath"], "$.email");
    assert_eq!(out["fieldMappings"][0]["jsonPath"], "$.name");
    let (status, out) = call(
        &fx,
        "inboundWebhooks.updateSettings",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "active": true })),
    )
    .await;
    assert_eq!((status, out["active"].clone()), (200, json!(true)), "{out}");

    // Intake: not JSON → 400; accepted → 202; same key → duplicate.
    let (status, out) = deliver(&fx, &token, "42", None).await;
    assert_eq!(
        (status, out["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("INVALID_JSON"))
    );
    let (status, out) = deliver(&fx, &token, r#"{"email":"a@b.co"}"#, Some("k1")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{out}");
    let request_id = out["requestId"].as_str().unwrap().to_owned();
    let (status, out) = deliver(&fx, &token, r#"{"other":1}"#, Some("k1")).await;
    assert_eq!(
        (status, out),
        (
            StatusCode::OK,
            json!({ "accepted": true, "duplicate": true })
        )
    );

    let (status, list) = call(
        &fx,
        "inboundWebhooks.listRequests",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["status"], "received");
    let (_, hooks) = call(&fx, "inboundWebhooks.list", WS, &[], None, None).await;
    assert_eq!(hooks["data"][0]["requestCount"], 1);

    let (status, detail) = call(
        &fx,
        "inboundWebhooks.getRequest",
        WS,
        &[("id", &id), ("requestId", &request_id)],
        Some("includePayload=true"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(detail["payload"], json!({ "email": "a@b.co" }));
    let (_, detail) = call(
        &fx,
        "inboundWebhooks.getRequest",
        WS,
        &[("id", &id), ("requestId", &request_id)],
        None,
        None,
    )
    .await;
    assert!(detail.get("payload").is_none(), "{detail}");

    // A new token: the old URL stops working.
    let (status, out) = call(
        &fx,
        "inboundWebhooks.regenerateToken",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_ne!(token_of(&out), token);
    let (status, _) = deliver(&fx, &token, "{}", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Delete: 204, then 404.
    let (status, _) = call(
        &fx,
        "inboundWebhooks.delete",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = call(
        &fx,
        "inboundWebhooks.delete",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn inbound_webhook_channels_list_every_channel_in_enum_order() {
    let fx = fixture().await;
    let (status, out) = call(&fx, "inboundWebhookChannels.list", WS, &[], None, None).await;
    assert_eq!(status, 200);
    let channels: Vec<&str> = out
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["channel"].as_str().unwrap())
        .collect();
    assert_eq!(
        channels,
        [
            "omnichannel",
            "webchat",
            "messenger",
            "whatsapp",
            "zalo",
            "smtp",
            "telegram",
            "instagram",
            "threads",
            "tiktok",
            "api"
        ]
    );
    assert_eq!(out[2]["inboxIdentifiers"][0]["kind"], "pageId");
    assert_eq!(out[2]["channelUserId"]["kind"], "psid");
}
