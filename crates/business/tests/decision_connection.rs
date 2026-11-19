//! Q.F2-settings: the AI Router connections
//! (`packages/business/src/decision/connection-service.ts`, the request
//! formatting of `transport.ts`, the response check of `adapters.ts`) over
//! a migrated database, a fake DoH and a scripted transport.

mod common;

use std::sync::{Arc, Mutex};

use chatbotx_business::decision_connection::{
    DecisionTransport, Question, SaveInput, create, format_request_body, list_safe,
    normalize_response, set_enabled, test, update,
};
use chatbotx_business::net::ssrf_guard::{DohFetch, DohReply};
use chatbotx_db::Db;
use chatbotx_kv::{DurableKv, SystemClock};
use common::{OTHER_WS, WS, two_workspaces};
use serde_json::{Value, json};

/// Every host resolves to a public address.
struct PublicDoh;

impl DohFetch for PublicDoh {
    async fn get(&self, _url: &str) -> Result<DohReply, String> {
        Ok(DohReply {
            status: 200,
            body: json!({ "Status": 0, "Answer": [{ "type": 1, "data": "93.184.216.34" }] })
                .to_string(),
        })
    }
}

/// Answers each test call with the next scripted reply and records it.
#[derive(Default)]
struct Script {
    replies: Mutex<Vec<Result<(u16, String), String>>>,
    seen: Mutex<Vec<(String, String, Value)>>,
}

impl DecisionTransport for Script {
    async fn post(
        &self,
        endpoint: &str,
        authorization: &str,
        body: &Value,
    ) -> Result<(u16, String), String> {
        self.seen
            .lock()
            .expect("lock")
            .push((endpoint.into(), authorization.into(), body.clone()));
        self.replies
            .lock()
            .expect("lock")
            .pop()
            .unwrap_or_else(|| Err("no scripted reply".into()))
    }
}

async fn kv() -> DurableKv {
    let db = Db::open_in_memory().expect("kv db");
    db.migrate().await.expect("kv migrate");
    DurableKv::new(Arc::new(db), Arc::new(SystemClock))
}

fn input(kind: &str) -> SaveInput {
    SaveInput {
        credential: Some("secret".into()),
        default_model: Some("model-a".into()),
        endpoint: None,
        model_catalog: vec![" model-a ".into(), "model-a".into(), "model-b".into()],
        name: " Decision ".into(),
        provider_kind: kind.into(),
    }
}

async fn stored_credential(db: &common::TestDb) -> String {
    db.read(|conn| {
        Ok(conn.query_row(
            r#"SELECT "credential" FROM "DecisionConnection""#,
            [],
            |r| r.get::<_, String>(0),
        )?)
    })
    .await
    .expect("credential")
}

/// Port of: packages/business/src/decision/connection-service.test.ts:33 "DecisionConnectionService > normalizes a catalog and encrypts credentials before persisting"
#[tokio::test]
async fn decision_connection_service_normalizes_a_catalog_and_encrypts_credentials_before_persisting()
 {
    let db = two_workspaces().await;
    let created = create(&db, &PublicDoh, WS, input("typesafe"))
        .await
        .expect("create");
    assert_eq!(created["modelCatalog"], json!(["model-a", "model-b"]));
    assert_eq!(created["name"], json!("Decision"));
    assert_eq!(created["credentialConfigured"], json!(true));
    assert_eq!(created["endpoint"], Value::Null);
    assert!(created.get("credential").is_none());
    let raw = stored_credential(&db).await;
    assert!(!raw.contains("secret"), "stored encrypted");
    assert_eq!(list_safe(&db, WS).await.expect("list").len(), 1);
    assert!(list_safe(&db, OTHER_WS).await.expect("other").is_empty());
}

/// Port of: packages/business/src/decision/connection-service.test.ts:61 "DecisionConnectionService > keeps the encrypted credential when an edit omits it"
#[tokio::test]
async fn decision_connection_service_keeps_the_encrypted_credential_when_an_edit_omits_it() {
    let db = two_workspaces().await;
    let created = create(&db, &PublicDoh, WS, input("typesafe"))
        .await
        .expect("create");
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();
    let before = stored_credential(&db).await;
    let edited = update(
        &db,
        &PublicDoh,
        WS,
        id,
        SaveInput {
            credential: None,
            name: "Renamed".into(),
            ..input("typesafe")
        },
    )
    .await
    .expect("update");
    assert_eq!(edited["name"], json!("Renamed"));
    assert_eq!(stored_credential(&db).await, before);
    let missing = update(&db, &PublicDoh, OTHER_WS, id, input("typesafe"))
        .await
        .expect_err("scoped");
    assert_eq!(missing.message(), "Decision connection not found");
}

/// Port of: packages/business/src/decision/connection-service.test.ts:84 "DecisionConnectionService > rejects a default model outside the normalized catalog"
#[tokio::test]
async fn decision_connection_service_rejects_a_default_model_outside_the_normalized_catalog() {
    let db = two_workspaces().await;
    let error = create(
        &db,
        &PublicDoh,
        WS,
        SaveInput {
            default_model: Some("missing".into()),
            ..input("typesafe")
        },
    )
    .await
    .expect_err("invalid");
    assert_eq!(error.code(), Some("invalidDecisionModel"));
}

/// Port of: packages/business/src/decision/connection-service.test.ts:97 "DecisionConnectionService > requires an endpoint only for compatible providers"
#[tokio::test]
async fn decision_connection_service_requires_an_endpoint_only_for_compatible_providers() {
    let db = two_workspaces().await;
    let error = create(&db, &PublicDoh, WS, input("systemOneCompatible"))
        .await
        .expect_err("no endpoint");
    assert_eq!(error.code(), Some("invalidDecisionEndpoint"));
    // A canonical public HTTPS endpoint is stored without its trailing slash;
    // a private address or a query is refused.
    let created = create(
        &db,
        &PublicDoh,
        WS,
        SaveInput {
            endpoint: Some("https://decide.example.com/v1/".into()),
            ..input("systemOneCompatible")
        },
    )
    .await
    .expect("compatible");
    assert_eq!(created["endpoint"], json!("https://decide.example.com/v1"));
    for (endpoint, code) in [
        ("https://10.0.0.1/v1", "unsafeDecisionEndpoint"),
        (
            "https://decide.example.com/v1?x=1",
            "invalidDecisionEndpoint",
        ),
        ("http://decide.example.com/v1", "invalidDecisionEndpoint"),
    ] {
        let error = create(
            &db,
            &PublicDoh,
            WS,
            SaveInput {
                endpoint: Some(endpoint.into()),
                name: format!("n-{code}-{}", endpoint.len()),
                ..input("systemOneCompatible")
            },
        )
        .await
        .expect_err(endpoint);
        assert_eq!(error.code(), Some(code), "{endpoint}");
    }
}

fn request() -> Value {
    json!({
        "questions": {
            "choice": {
                "instructions": "Choose one.",
                "label": "Choice",
                "options": [
                    { "description": "The first option.", "label": "One", "value": "one" },
                    { "label": "Two", "value": "two" },
                ],
                "type": "choice",
            },
            "score": {
                "instructions": "Score it.",
                "label": "Score",
                "levels": [
                    { "description": "The low level.", "label": "Low", "value": 0 },
                    { "label": "High", "value": 1 },
                ],
                "type": "score",
            },
        },
    })
}

/// Port of: packages/business/src/decision/transport.test.ts:28 "formatDecisionRequestBody > converts %s question criteria"
#[test]
fn format_decision_request_body_converts_question_criteria() {
    for kind in ["typesafe", "openrouterDecision"] {
        assert_eq!(
            format_request_body(&request(), kind),
            json!({
                "questions": {
                    "choice": {
                        "criteria": { "one": "The first option.", "two": "Two" },
                        "instructions": "Choose one.",
                        "label": "Choice",
                        "type": "choice",
                    },
                    "score": {
                        "criteria": ["The low level.", "High"],
                        "instructions": "Score it.",
                        "label": "Score",
                        "type": "score",
                    },
                },
            }),
            "{kind}"
        );
    }
}

/// Port of: packages/business/src/decision/transport.test.ts:52 "formatDecisionRequestBody > preserves the System One Compatible request body"
#[test]
fn format_decision_request_body_preserves_the_system_one_compatible_request_body() {
    assert_eq!(
        format_request_body(&request(), "systemOneCompatible"),
        request()
    );
}

/// Port of: packages/business/src/decision/adapters.test.ts:66 "Decision Profile adapter > normalizes native zero-based scores to the product scale"
#[test]
fn decision_profile_adapter_normalizes_native_zero_based_scores_to_the_product_scale() {
    let questions = [Question::Score {
        key: "result".into(),
        min: 1,
        max: 2,
    }];
    assert_eq!(
        normalize_response(
            &questions,
            &json!({ "answers": { "result": { "score": 0, "type": "score" } }, "model": "model" }),
        ),
        Ok(json!({ "answers": { "result": { "score": 1.0, "type": "score" } }, "model": "model" }))
    );
    assert!(
        normalize_response(
            &questions,
            &json!({ "answers": { "result": { "score": 5, "type": "score" } }, "model": "model" }),
        )
        .is_err()
    );
}

fn conformant() -> String {
    json!({
        "model": "model-a",
        "answers": {
            "choice_test": { "type": "choice", "choice": "one", "confidence": 0.9 },
            "score_test": { "type": "score", "score": 1 },
            "noul_test": { "type": "noul", "probability": 0.4 },
        },
    })
    .to_string()
}

/// Own (`decisionConnectionService.test`): a conformant answer passes, is
/// stored as `lastTest`, sends the decrypted key and the criteria shape; a
/// second test within 30 s is throttled; a bad answer fails; a disabled
/// toggle round-trips.
#[tokio::test]
async fn test_calls_the_provider_once_per_window_and_stores_the_result() {
    let db = two_workspaces().await;
    let kv = kv().await;
    let created = create(&db, &PublicDoh, WS, input("typesafe"))
        .await
        .expect("create");
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();
    let transport = Script::default();
    transport
        .replies
        .lock()
        .expect("lock")
        .push(Ok((200, conformant())));
    let result = test(&db, &kv, &transport, WS, id).await.expect("test");
    assert_eq!(result["status"], json!("passed"));
    assert_eq!(result["code"], json!("ok"));
    assert_eq!(result["model"], json!("model-a"));
    {
        let seen = transport.seen.lock().expect("lock");
        assert_eq!(seen[0].0, "https://api.typesafe.ai/v1/systemone");
        assert_eq!(seen[0].1, "secret");
        assert!(seen[0].2["questions"]["choice_test"]["criteria"].is_object());
        assert!(
            seen[0].2["questions"]["choice_test"]
                .get("options")
                .is_none()
        );
    }
    let listed = list_safe(&db, WS).await.expect("list");
    assert_eq!(listed[0]["lastTest"]["status"], json!("passed"));
    assert!(listed[0]["lastTest"]["testedAt"].is_string());

    let throttled = test(&db, &kv, &transport, WS, id)
        .await
        .expect_err("throttled");
    assert_eq!(throttled.code(), Some("decisionConnectionTestThrottled"));

    let other_kv = self::kv().await;
    transport.replies.lock().expect("lock").push(Ok((
        200,
        json!({ "model": "m", "answers": {} }).to_string(),
    )));
    let failed = test(&db, &other_kv, &transport, WS, id).await.expect("ran");
    assert_eq!(failed["status"], json!("failed"));
    assert_eq!(failed["code"], json!("failed"));
    assert!(
        test(&db, &other_kv, &transport, OTHER_WS, id)
            .await
            .is_err()
    );

    let disabled = set_enabled(&db, WS, id, false).await.expect("toggle");
    assert_eq!(disabled["status"], json!("disabled"));
    assert!(set_enabled(&db, OTHER_WS, id, true).await.is_err());
}
