//! `externalWebhooksPublicRouter` (Q.D13; no TS test): register is
//! idempotent per `(event, url)`, private targets are refused, unregister
//! is scoped to the token workspace.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

#[tokio::test]
async fn external_webhook_register_list_and_unregister() {
    let fx = fixture().await;
    // A private address fails the SSRF guard (422).
    let (status, out) = call(
        &fx,
        "externalWebhooks.create",
        WS,
        &[],
        None,
        Some(json!({ "url": "http://127.0.0.1/hook", "event": "lead" })),
    )
    .await;
    assert_eq!(
        (status, out["code"].clone()),
        (422, json!("invalidRequestData")),
        "{out}"
    );

    let body = json!({ "url": "https://1.1.1.1/hook", "event": "lead" });
    let (status, first) = call(
        &fx,
        "externalWebhooks.create",
        WS,
        &[],
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(
        (status, first["provider"].clone()),
        (201, json!("make")),
        "{first}"
    );
    let (_, again) = call(&fx, "externalWebhooks.create", WS, &[], None, Some(body)).await;
    assert_eq!(
        again["id"], first["id"],
        "same (event, url) is the same row"
    );

    let (status, list) = call(&fx, "externalWebhooks.list", WS, &[], None, None).await;
    assert_eq!((status, list["data"].as_array().unwrap().len()), (200, 1));

    let id = first["id"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &fx,
        "externalWebhooks.delete",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &fx,
        "externalWebhooks.delete",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
}
