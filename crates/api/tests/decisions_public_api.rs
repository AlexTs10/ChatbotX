//! `packages/business/src/decision/{connection-service,profile-service}.test.ts`
//! (Q.D13b), adapted: the mocked repository becomes a migrated database and
//! the `/v1/decision-*` routes are called through `handlers::dispatch`.

mod handlers_common;

use std::sync::Arc;

use chatbotx_core::encryption::{Encryption, Keys};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

async fn setup() -> Fixture {
    let mut f = fixture().await;
    f.state = f
        .state
        .clone()
        .with_encryption(Arc::new(Encryption::new(Keys::new(KEY).expect("keys"))));
    f
}

async fn connection(f: &Fixture) -> String {
    let (status, out) = call(
        f,
        "decisionConnections.create",
        WS,
        &[],
        None,
        Some(json!({
            "name": "Decision",
            "providerKind": "typesafe",
            "modelCatalog": [" model-a ", "model-a", "model-b"],
            "defaultModel": "model-b",
            "credential": "secret",
        })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    out["id"].as_str().unwrap().to_owned()
}

fn form(connection_id: &str) -> Value {
    json!({
        "connectionId": connection_id,
        "decision": {
            "type": "choice",
            "instructions": "Classify the current customer intent.",
            "options": [
                { "value": "purchase", "description": "The customer wants to buy." },
                { "value": "other", "description": "No other intent applies." },
            ],
        },
        "model": "model-a",
        "name": "Customer intent",
        "status": "enabled",
        "thresholdConfig": null,
    })
}

#[tokio::test]
async fn connections_normalize_the_catalog_and_never_return_the_credential() {
    let f = setup().await;
    let id = connection(&f).await;
    let (status, out) = call(&f, "decisionConnections.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{out}");
    let row = &out["data"][0];
    assert_eq!(row["modelCatalog"], json!(["model-a", "model-b"]));
    assert_eq!(row["credentialConfigured"], true);
    assert!(row.get("credential").is_none());
    assert!(!out.to_string().contains("secret"));
    let stored: String = f
        .state
        .db
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT "credential" FROM "DecisionConnection" WHERE "id" = ?1"#,
                [id.parse::<i64>().unwrap()],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert!(!stored.contains("secret"), "the credential is encrypted");

    // Another workspace sees nothing.
    let (_, other) = call(&f, "decisionConnections.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other["data"], json!([]));
}

#[tokio::test]
async fn connection_validation_follows_the_original_codes() {
    let f = setup().await;
    let (status, out) = call(
        &f,
        "decisionConnections.create",
        WS,
        &[],
        None,
        Some(
            json!({ "name": "D", "providerKind": "typesafe", "modelCatalog": ["model-a"],
                     "defaultModel": "missing", "credential": "secret" }),
        ),
    )
    .await;
    assert_eq!(status, 400, "{out}");
    assert_eq!(out["code"], "invalidDecisionModel");

    let (status, out) = call(
        &f,
        "decisionConnections.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "D", "providerKind": "systemOneCompatible",
                     "modelCatalog": ["model-a"], "credential": "secret" })),
    )
    .await;
    assert_eq!(status, 400, "{out}");
    assert_eq!(out["code"], "invalidDecisionEndpoint");
}

#[tokio::test]
async fn an_edit_without_a_credential_keeps_the_stored_one() {
    let f = setup().await;
    let id = connection(&f).await;
    let read = |f: &Fixture, id: i64| {
        let db = Arc::clone(&f.state.db);
        async move {
            db.read(move |c| {
                Ok(c.query_row(
                    r#"SELECT "credential" FROM "DecisionConnection" WHERE "id" = ?1"#,
                    [id],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .await
            .unwrap()
        }
    };
    let n: i64 = id.parse().unwrap();
    let before = read(&f, n).await;
    let (status, out) = call(
        &f,
        "decisionConnections.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Renamed", "providerKind": "typesafe", "modelCatalog": ["model-a"] })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["name"], "Renamed");
    assert_eq!(read(&f, n).await, before);
}

#[tokio::test]
async fn profiles_round_trip_through_the_public_shape() {
    let f = setup().await;
    let conn = connection(&f).await;
    let (status, created) = call(
        &f,
        "decisionProfiles.create",
        WS,
        &[],
        None,
        Some(form(&conn)),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["decision"], form(&conn)["decision"]);
    assert_eq!(created["globalRouterEnabled"], false);
    let id = created["id"].as_str().unwrap().to_owned();

    let (status, got) = call(&f, "decisionProfiles.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{got}");
    assert_eq!(got, created);

    let (status, list) = call(&f, "decisionProfiles.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["data"].as_array().unwrap().len(), 1);

    let (status, out) = call(
        &f,
        "decisionProfiles.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{out}");

    let mut renamed = form(&conn);
    renamed["name"] = json!("Renamed");
    let (status, out) = call(
        &f,
        "decisionProfiles.update",
        WS,
        &[("id", &id)],
        None,
        Some(renamed),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["name"], "Renamed");

    let (status, _) = call(
        &f,
        "decisionProfiles.deleteMany",
        WS,
        &[],
        None,
        Some(json!({ "ids": [id] })),
    )
    .await;
    assert_eq!(status, 204);
    let (_, list) = call(&f, "decisionProfiles.list", WS, &[], None, None).await;
    assert_eq!(list["data"], json!([]));
}

#[tokio::test]
async fn profiles_reject_a_model_outside_the_connection_catalog() {
    let f = setup().await;
    let conn = connection(&f).await;
    let mut bad = form(&conn);
    bad["model"] = json!("unknown");
    let (status, out) = call(&f, "decisionProfiles.create", WS, &[], None, Some(bad)).await;
    assert_eq!(status, 400, "{out}");
    assert_eq!(out["code"], "invalidDecisionModel");

    let (status, out) = call(
        &f,
        "decisionProfiles.create",
        WS,
        &[],
        None,
        Some(form("999999")),
    )
    .await;
    assert_eq!(status, 404, "{out}");

    let mut extra = form(&conn);
    extra["unknown"] = json!(1);
    // `.strict()`: an unknown key is the 422 `invalidRequestData` here.
    let (status, _) = call(&f, "decisionProfiles.create", WS, &[], None, Some(extra)).await;
    assert_eq!(status, 422);
}
