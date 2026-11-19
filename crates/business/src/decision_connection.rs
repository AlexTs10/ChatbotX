//! Q.F2-settings: the AI Router (decision) connections
//! (`packages/business/src/decision/connection-service.ts`, its
//! `compatible-url.ts`, the request formatting and pinned transport of
//! `transport.ts` and the response check of `adapters.ts`).
//!
//! The credential is stored with `encryptObject` under the AAD
//! `decision-connection:{workspaceId}:{id}` and never leaves the server. A
//! compatible endpoint must be a canonical public HTTPS URL; the test call
//! re-checks it, dials only the checked address, follows no redirect, and
//! reads at most 256 KiB in 2.5 s. One test per connection per 30 s.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use chatbotx_db::Db;
use chatbotx_db::repo::{iso, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, Row, params};
use chatbotx_kv::DurableKv;
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result};
use crate::meta_catalog::process_encryption;
use crate::net::ssrf_guard::{DohFetch, SsrfCheck, check_ssrf_safety_with};
use crate::tx;

pub const PROVIDER_KINDS: &[&str] = &["typesafe", "systemOneCompatible", "openrouterDecision"];
const TEST_THROTTLE_SECONDS: u64 = 30;
const REQUEST_TIMEOUT: Duration = Duration::from_millis(2500);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const DIRECT_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn aad(workspace_id: i64, id: i64) -> String {
    format!("decision-connection:{workspace_id}:{id}")
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Decision connection not found")
}

fn invalid(code: &str, message: &str) -> BusinessError {
    BusinessError::exception(message, code, 400)
}

/// `decisionConnectionActionSchema` (strict).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveInput {
    pub credential: Option<String>,
    pub default_model: Option<String>,
    pub endpoint: Option<String>,
    pub model_catalog: Vec<String>,
    pub name: String,
    pub provider_kind: String,
}

fn trimmed(
    object: &Map<String, Value>,
    key: &str,
    min: usize,
    max: usize,
) -> Result<Option<Option<String>>> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(text)) => {
            let text = text.trim();
            let count = text.chars().count();
            if count < min || count > max {
                return Err(BusinessError::validation(key, "Invalid input"));
            }
            Ok(Some(Some(text.to_string())))
        }
        Some(_) => Err(BusinessError::validation(key, "Invalid input")),
    }
}

/// Parses the action / public body; `extra` names the keys the caller
/// handles itself (`id`).
pub fn parse_save_input(value: &Value, extra: &[&str]) -> Result<SaveInput> {
    let Some(object) = value.as_object() else {
        return Err(BusinessError::validation("", "Invalid input"));
    };
    const KEYS: &[&str] = &[
        "credential",
        "defaultModel",
        "endpoint",
        "modelCatalog",
        "name",
        "providerKind",
    ];
    if let Some(key) = object
        .keys()
        .find(|k| !KEYS.contains(&k.as_str()) && !extra.contains(&k.as_str()))
    {
        return Err(BusinessError::validation(key, "Unrecognized key"));
    }
    let credential = match trimmed(object, "credential", 0, 2048)? {
        Some(None) => return Err(BusinessError::validation("credential", "Invalid input")),
        other => other.flatten(),
    };
    let default_model = trimmed(object, "defaultModel", 1, 256)?.flatten();
    let endpoint = trimmed(object, "endpoint", 0, 2048)?.flatten();
    let Some(Value::Array(items)) = object.get("modelCatalog") else {
        return Err(BusinessError::validation("modelCatalog", "Invalid input"));
    };
    if items.is_empty() || items.len() > 100 {
        return Err(BusinessError::validation("modelCatalog", "Invalid input"));
    }
    let mut model_catalog = Vec::new();
    for item in items {
        let text = item.as_str().map(str::trim).unwrap_or_default();
        let count = text.chars().count();
        if item.as_str().is_none() || !(1..=256).contains(&count) {
            return Err(BusinessError::validation("modelCatalog", "Invalid input"));
        }
        model_catalog.push(text.to_string());
    }
    let name = trimmed(object, "name", 1, 160)?
        .flatten()
        .ok_or_else(|| BusinessError::validation("name", "Invalid input"))?;
    let provider_kind = object
        .get("providerKind")
        .and_then(Value::as_str)
        .filter(|kind| PROVIDER_KINDS.contains(kind))
        .ok_or_else(|| BusinessError::validation("providerKind", "Invalid input"))?
        .to_string();
    Ok(SaveInput {
        credential,
        default_model,
        endpoint,
        model_catalog,
        name,
        provider_kind,
    })
}

/// `uniqueModels`.
fn unique_models(models: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for model in models.iter().map(|m| m.trim()).filter(|m| !m.is_empty()) {
        if !out.iter().any(|m| m == model) {
            out.push(model.to_string());
        }
    }
    out
}

/// `assertModelConfig`.
fn assert_model_config(default_model: Option<&str>, catalog: &[String]) -> Result<()> {
    if catalog.is_empty() {
        return Err(invalid(
            "invalidDecisionModel",
            "Decision connection must have at least one model",
        ));
    }
    if let Some(model) = default_model.filter(|m| !m.is_empty())
        && !catalog.iter().any(|m| m == model.trim())
    {
        return Err(invalid(
            "invalidDecisionModel",
            "Default decision model must be in the connection catalog",
        ));
    }
    Ok(())
}

/// `normalizeCompatibleEndpoint`: HTTPS, no userinfo / query / fragment,
/// no trailing slash, publicly reachable.
pub async fn normalize_compatible_endpoint(raw: &str, doh: &impl DohFetch) -> Result<String> {
    let Ok(mut endpoint) = url::Url::parse(raw.trim()) else {
        return Err(invalid(
            "invalidDecisionEndpoint",
            "Decision provider endpoint must be a valid HTTPS URL",
        ));
    };
    if endpoint.scheme() != "https"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some_and(|q| !q.is_empty())
        || endpoint.fragment().is_some_and(|f| !f.is_empty())
    {
        return Err(invalid(
            "invalidDecisionEndpoint",
            "Decision provider endpoint must be a canonical HTTPS URL",
        ));
    }
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let path = endpoint.path().trim_end_matches('/').to_string();
    endpoint.set_path(if path.is_empty() { "/" } else { &path });
    let normalized = endpoint.to_string();
    if check_ssrf_safety_with(&normalized, doh).await == SsrfCheck::Unsafe {
        return Err(invalid(
            "unsafeDecisionEndpoint",
            "Decision provider endpoint is not publicly reachable",
        ));
    }
    Ok(normalized)
}

/// `resolveEndpoint`: only the compatible kind keeps an endpoint.
async fn resolve_endpoint(input: &SaveInput, doh: &impl DohFetch) -> Result<Option<String>> {
    if input.provider_kind != "systemOneCompatible" {
        return Ok(None);
    }
    let Some(endpoint) = input.endpoint.as_deref().filter(|e| !e.is_empty()) else {
        return Err(invalid(
            "invalidDecisionEndpoint",
            "System One Compatible requires an endpoint",
        ));
    };
    normalize_compatible_endpoint(endpoint, doh).await.map(Some)
}

fn seal(workspace_id: i64, id: i64, api_key: &str) -> Result<String> {
    // `decisionCredentialSchema`: a trimmed, non-empty key.
    let api_key = api_key.trim();
    if api_key.is_empty() || api_key.chars().count() > 2048 {
        return Err(BusinessError::validation("apiKey", "Invalid input"));
    }
    let sealed = process_encryption()
        .encrypt_object(&json!({ "apiKey": api_key }), Some(&aad(workspace_id, id)))
        .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    serde_json::to_string(&sealed)
        .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))
}

const COLUMNS: &str = r#""id", "defaultModel", "endpoint", "lastTestStatus", "lastTestedAt",
    "modelCatalog", "name", "providerKind", "status""#;

/// `toSafeConnection` (no credential).
fn safe(row: &Row<'_>) -> chatbotx_db::rusqlite::Result<Value> {
    let catalog: String = row.get(5)?;
    Ok(json!({
        "credentialConfigured": true,
        "defaultModel": row.get::<_, Option<String>>(1)?,
        "endpoint": row.get::<_, Option<String>>(2)?,
        "id": row.get::<_, i64>(0)?.to_string(),
        "lastTest": {
            "status": row.get::<_, Option<String>>(3)?,
            "testedAt": row.get::<_, Option<i64>>(4)?.map(iso),
        },
        "modelCatalog": serde_json::from_str::<Value>(&catalog).unwrap_or(json!([])),
        "name": row.get::<_, String>(6)?,
        "providerKind": row.get::<_, String>(7)?,
        "status": row.get::<_, String>(8)?,
    }))
}

fn find_safe(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Value>> {
    conn.query_row(
        &format!(
            r#"SELECT {COLUMNS} FROM "DecisionConnection" WHERE "id" = ?1 AND "workspaceId" = ?2"#
        ),
        params![id, workspace_id],
        safe,
    )
    .optional()
    .map_err(repo_err)
}

/// `listSafe` (newest first).
pub async fn list_safe(db: &Db, workspace_id: i64) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT {COLUMNS} FROM "DecisionConnection" WHERE "workspaceId" = ?1
                   ORDER BY "createdAt" DESC, "id" DESC"#
            ))
            .map_err(repo_err)?;
        let rows = stmt
            .query_map([workspace_id], safe)
            .map_err(repo_err)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await
}

/// `create`.
pub async fn create(
    db: &Db,
    doh: &impl DohFetch,
    workspace_id: i64,
    input: SaveInput,
) -> Result<Value> {
    let catalog = unique_models(&input.model_catalog);
    assert_model_config(input.default_model.as_deref(), &catalog)?;
    let id = new_id();
    let sealed = seal(
        workspace_id,
        id,
        input.credential.as_deref().unwrap_or_default(),
    )?;
    let endpoint = resolve_endpoint(&input, doh).await?;
    let catalog = serde_json::to_string(&catalog).unwrap_or_else(|_| "[]".into());
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "DecisionConnection" ("id", "createdAt", "updatedAt", "workspaceId", "name",
                   "providerKind", "endpoint", "credential", "modelCatalog", "defaultModel")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
            params![
                id,
                now,
                workspace_id,
                input.name.trim(),
                input.provider_kind,
                endpoint,
                sealed,
                catalog,
                input.default_model
            ],
        )
        .map_err(repo_err)?;
        find_safe(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `update`: an omitted / blank credential keeps the stored one.
pub async fn update(
    db: &Db,
    doh: &impl DohFetch,
    workspace_id: i64,
    id: i64,
    input: SaveInput,
) -> Result<Value> {
    let exists = tx::read(db, move |conn| {
        Ok(find_safe(conn, workspace_id, id)?.is_some())
    })
    .await?;
    if !exists {
        return Err(not_found());
    }
    let catalog = unique_models(&input.model_catalog);
    assert_model_config(input.default_model.as_deref(), &catalog)?;
    let endpoint = resolve_endpoint(&input, doh).await?;
    let sealed = match input.credential.as_deref().map(str::trim) {
        Some(key) if !key.is_empty() => Some(seal(workspace_id, id, key)?),
        _ => None,
    };
    let catalog = serde_json::to_string(&catalog).unwrap_or_else(|_| "[]".into());
    tx::write(db, move |conn| {
        let changed = conn
            .execute(
                r#"UPDATE "DecisionConnection" SET "name" = ?3, "providerKind" = ?4, "endpoint" = ?5,
                       "credential" = COALESCE(?6, "credential"), "modelCatalog" = ?7, "defaultModel" = ?8,
                       "updatedAt" = ?9
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![
                    id,
                    workspace_id,
                    input.name.trim(),
                    input.provider_kind,
                    endpoint,
                    sealed,
                    catalog,
                    input.default_model,
                    now_ms()
                ],
            )
            .map_err(repo_err)?;
        if changed == 0 {
            return Err(not_found());
        }
        find_safe(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `setEnabled`.
pub async fn set_enabled(db: &Db, workspace_id: i64, id: i64, enabled: bool) -> Result<Value> {
    tx::write(db, move |conn| {
        let changed = conn
            .execute(
                r#"UPDATE "DecisionConnection" SET "status" = ?3, "updatedAt" = ?4
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![
                    id,
                    workspace_id,
                    if enabled { "enabled" } else { "disabled" },
                    now_ms()
                ],
            )
            .map_err(repo_err)?;
        if changed == 0 {
            return Err(not_found());
        }
        find_safe(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// The provider call of a test (`postDecisionRequest`).
pub trait DecisionTransport: Send + Sync {
    /// `Ok((status, body))`; `Err` = refused or no answer.
    fn post(
        &self,
        endpoint: &str,
        authorization: &str,
        body: &Value,
    ) -> impl Future<Output = std::result::Result<(u16, String), String>> + Send;
}

/// The real transport: the endpoint is re-checked and only its first
/// public address is dialed (TLS keeps the host name).
pub struct HttpDecisionTransport<D>(pub D);

impl<D: DohFetch + Send + Sync> DecisionTransport for HttpDecisionTransport<D> {
    async fn post(
        &self,
        endpoint: &str,
        authorization: &str,
        body: &Value,
    ) -> std::result::Result<(u16, String), String> {
        let SsrfCheck::Safe { resolved_ips } = check_ssrf_safety_with(endpoint, &self.0).await
        else {
            return Err("Decision endpoint failed public-address validation".into());
        };
        let url = url::Url::parse(endpoint).map_err(|e| e.to_string())?;
        let host = url.host_str().ok_or("Decision endpoint has no host")?;
        let ip: IpAddr = resolved_ips
            .first()
            .and_then(|ip| ip.parse().ok())
            .ok_or("Decision endpoint failed public-address validation")?;
        let port = url.port_or_known_default().unwrap_or(443);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .resolve(host, SocketAddr::new(ip, port))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .post(endpoint)
            .header("authorization", format!("Bearer {authorization}"))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err("Decision provider redirect was rejected".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err("Decision provider response exceeded size limit".into());
            }
        }
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }
}

/// providerKind, endpoint, encrypted credential, defaultModel, modelCatalog.
type TestRow = (String, Option<String>, String, Option<String>, String);

/// `resolveProviderEndpoint`.
fn provider_endpoint(kind: &str, endpoint: Option<&str>) -> Option<String> {
    match kind {
        "typesafe" => Some(DIRECT_ENDPOINT.into()),
        "openrouterDecision" => Some(OPENROUTER_ENDPOINT.into()),
        _ => endpoint.map(str::to_string),
    }
}

/// `formatDecisionRequestBody`: the compatible kind gets the body as is;
/// the others get `criteria` in place of `options` / `levels`.
pub fn format_request_body(body: &Value, kind: &str) -> Value {
    if kind == "systemOneCompatible" {
        return body.clone();
    }
    let mut out = body.clone();
    let Some(questions) = body["questions"].as_object() else {
        out["questions"] = json!({});
        return out;
    };
    let mut converted = Map::new();
    for (key, question) in questions {
        let mut q = question.clone();
        let label_of = |item: &Value| {
            item["description"]
                .as_str()
                .or_else(|| item["label"].as_str())
                .map(|s| json!(s))
                .unwrap_or(Value::Null)
        };
        match (question["type"].as_str(), q.as_object_mut()) {
            (Some("choice"), Some(object)) if question["options"].is_array() => {
                let criteria: Map<String, Value> = question["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| {
                        (
                            o["value"].as_str().unwrap_or_default().to_string(),
                            label_of(o),
                        )
                    })
                    .collect();
                object.remove("options");
                object.insert("criteria".into(), Value::Object(criteria));
            }
            (Some("score"), Some(object)) if question["levels"].is_array() => {
                let criteria: Vec<Value> = question["levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(label_of)
                    .collect();
                object.remove("levels");
                object.insert("criteria".into(), Value::Array(criteria));
            }
            _ => {}
        }
        converted.insert(key.clone(), q);
    }
    out["questions"] = Value::Object(converted);
    out
}

fn is_stable_key(key: &str) -> bool {
    let mut chars = key.chars();
    (1..=64).contains(&key.len())
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn unit(value: &Value) -> bool {
    value.as_f64().is_some_and(|v| (0.0..=1.0).contains(&v))
}

fn optional_unit(value: Option<&Value>) -> bool {
    value.is_none_or(|v| v.is_null() || unit(v))
}

/// One `decisionAnswerSchema` answer from the provider wire shape
/// (`normalizeProviderAnswers`, then the strict schema).
fn provider_answer(answer: &Value) -> Option<Value> {
    let confidence = answer.get("confidence").filter(|c| !c.is_null());
    if !optional_unit(confidence) {
        return None;
    }
    match answer["type"].as_str()? {
        "choice" => {
            let choice = answer["choice"].as_str().filter(|c| is_stable_key(c))?;
            if let Some(probabilities) = answer.get("probabilities").filter(|p| !p.is_null()) {
                let ok = probabilities
                    .as_object()?
                    .iter()
                    .all(|(k, v)| is_stable_key(k) && unit(v));
                if !ok {
                    return None;
                }
            }
            let mut out = json!({ "choice": choice, "type": "choice" });
            if let Some(c) = confidence {
                out["confidence"] = c.clone();
            }
            if let Some(p) = answer.get("probabilities").filter(|p| !p.is_null()) {
                out["probabilities"] = p.clone();
            }
            Some(out)
        }
        "score" => {
            let score = answer["score"].as_f64().filter(|s| s.is_finite())?;
            let mut out = json!({ "score": score, "type": "score" });
            if let Some(c) = confidence {
                out["confidence"] = c.clone();
            }
            Some(out)
        }
        "noul" => {
            let probability = answer
                .get("probability")
                .filter(|p| !p.is_null())
                .or_else(|| answer.get("noul").filter(|p| !p.is_null()))?;
            if !unit(probability) {
                return None;
            }
            let mut out = json!({ "probabilityTrue": probability, "type": "noul" });
            if let Some(c) = confidence {
                out["confidence"] = c.clone();
            }
            Some(out)
        }
        _ => None,
    }
}

/// The native (`answers: [...]`, `providerModel`) shape.
fn native_answers(raw: &Value) -> Option<(Map<String, Value>, String)> {
    let object = raw.as_object()?;
    if object
        .keys()
        .any(|k| k != "answers" && k != "providerModel")
    {
        return None;
    }
    let model = raw["providerModel"].as_str().filter(|m| !m.is_empty())?;
    let mut answers = Map::new();
    for answer in raw["answers"].as_array()? {
        let name = answer["name"].as_str()?;
        let wire = match answer["type"].as_str()? {
            "choice" => json!({ "type": "choice", "choice": answer["selectedId"],
                "confidence": answer.get("confidence"), "probabilities": answer.get("probabilities") }),
            "score" => {
                json!({ "type": "score", "score": answer["score"], "confidence": answer.get("confidence") })
            }
            "noul" => {
                json!({ "type": "noul", "probability": answer["probability"], "confidence": answer.get("confidence") })
            }
            _ => return None,
        };
        answers.insert(name.to_string(), provider_answer(&wire)?);
    }
    Some((answers, model.to_string()))
}

/// A question of the contract the answers are checked against.
#[derive(Debug, Clone, PartialEq)]
pub enum Question {
    Choice { key: String, values: Vec<String> },
    Score { key: String, min: i64, max: i64 },
    Noul { key: String },
}

impl Question {
    fn key(&self) -> &str {
        match self {
            Question::Choice { key, .. } | Question::Score { key, .. } | Question::Noul { key } => {
                key
            }
        }
    }
}

/// `normalizeDecisionProviderResponse`: the provider or native shape, then
/// every question answered once with a valid value; a native zero-based
/// score moves to the product scale (`+ 1`).
pub fn normalize_response(
    questions: &[Question],
    raw: &Value,
) -> std::result::Result<Value, String> {
    let wire = raw.as_object().and_then(|object| {
        let model = object
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|m| !m.is_empty() && m.chars().count() <= 256)?;
        let mut answers = Map::new();
        for (key, answer) in object.get("answers")?.as_object()? {
            if !is_stable_key(key) {
                return None;
            }
            answers.insert(key.clone(), provider_answer(answer)?);
        }
        Some((answers, model.to_string()))
    });
    let (answers, model) = wire
        .or_else(|| native_answers(raw))
        .ok_or("Decision provider returned an invalid response")?;
    if answers.len() != questions.len()
        || answers
            .keys()
            .any(|k| !questions.iter().any(|q| q.key() == k))
    {
        return Err("Decision provider returned a missing or unknown answer".into());
    }
    let mut out = Map::new();
    for (key, mut answer) in answers {
        let question = questions
            .iter()
            .find(|q| q.key() == key)
            .ok_or("unknown answer")?;
        match (question, answer["type"].as_str()) {
            (Question::Choice { values, .. }, Some("choice")) => {
                if !values.iter().any(|v| answer["choice"] == json!(v)) {
                    return Err("Decision provider returned an invalid choice".into());
                }
            }
            (Question::Score { min, max, .. }, Some("score")) => {
                let score = answer["score"].as_f64().unwrap_or(f64::NAN) + 1.0;
                if !(score >= *min as f64 && score <= *max as f64) {
                    return Err(
                        "Decision provider returned a score outside the Profile range".into(),
                    );
                }
                answer["score"] = json!(score);
            }
            (Question::Noul { .. }, Some("noul")) => {}
            _ => return Err("Decision provider answer type does not match Profile".into()),
        }
        out.insert(key, answer);
    }
    Ok(json!({ "answers": out, "model": model }))
}

/// The conformance contract of a test.
fn test_contract() -> (Value, Vec<Question>) {
    let questions = json!({
        "choice_test": {
            "instructions": "Classify the test input.",
            "key": "choice_test",
            "label": "Choice",
            "options": [
                { "description": "First test choice.", "label": "One", "value": "one" },
                { "description": "Second test choice.", "label": "Two", "value": "two" },
            ],
            "type": "choice",
        },
        "score_test": {
            "instructions": "Score the test input.",
            "key": "score_test",
            "label": "Score",
            "levels": [
                { "description": "Low test score.", "label": "1", "value": 1 },
                { "description": "High test score.", "label": "2", "value": 2 },
            ],
            "type": "score",
        },
        "noul_test": {
            "instructions": "Evaluate the test input.",
            "key": "noul_test",
            "label": "Noul",
            "type": "noul",
        },
    });
    let contract = vec![
        Question::Choice {
            key: "choice_test".into(),
            values: vec!["one".into(), "two".into()],
        },
        Question::Score {
            key: "score_test".into(),
            min: 1,
            max: 2,
        },
        Question::Noul {
            key: "noul_test".into(),
        },
    ];
    (questions, contract)
}

/// `test`: a live conformance call (one per connection per 30 s); the
/// result is stored as `lastTest`.
pub async fn test(
    db: &Db,
    kv: &DurableKv,
    transport: &impl DecisionTransport,
    workspace_id: i64,
    id: i64,
) -> Result<Value> {
    let row: Option<TestRow> = tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "providerKind", "endpoint", "credential", "defaultModel", "modelCatalog"
                   FROM "DecisionConnection" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(repo_err)
    })
    .await?;
    let Some((kind, endpoint, credential, default_model, catalog)) = row else {
        return Err(not_found());
    };
    let acquired = kv
        .set_number_if_not_exists(
            &format!("decision:connection-test:{workspace_id}:{id}"),
            1.0,
            TEST_THROTTLE_SECONDS,
        )
        .await
        .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    if !acquired {
        return Err(BusinessError::exception(
            "Decision connection test is already running or was run recently",
            "decisionConnectionTestThrottled",
            429,
        ));
    }
    let catalog: Vec<String> = serde_json::from_str(&catalog).unwrap_or_default();
    let Some(model) = default_model.or_else(|| catalog.first().cloned()) else {
        return Err(invalid(
            "invalidDecisionModel",
            "Decision connection has no model",
        ));
    };
    let started = Instant::now();
    let passed = run_test(
        transport,
        workspace_id,
        id,
        &kind,
        endpoint.as_deref(),
        &credential,
        &model,
    )
    .await
    .is_ok();
    let status = if passed { "passed" } else { "failed" };
    let latency = started.elapsed().as_millis() as i64;
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "DecisionConnection" SET "lastTestStatus" = ?3, "lastTestedAt" = ?4, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, status, now],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await?;
    Ok(json!({
        "code": if passed { "ok" } else { "failed" },
        "latencyMs": latency,
        "model": model,
        "status": status,
    }))
}

async fn run_test(
    transport: &impl DecisionTransport,
    workspace_id: i64,
    id: i64,
    kind: &str,
    endpoint: Option<&str>,
    credential: &str,
    model: &str,
) -> std::result::Result<Value, String> {
    let sealed = serde_json::from_str(credential).map_err(|e| e.to_string())?;
    let decrypted: Value = process_encryption()
        .decrypt_object(&sealed, Some(&aad(workspace_id, id)))
        .map_err(|e| e.to_string())?;
    let api_key = decrypted["apiKey"].as_str().ok_or("no api key")?;
    let endpoint =
        provider_endpoint(kind, endpoint).ok_or("System One Compatible endpoint is missing")?;
    let (questions, contract) = test_contract();
    let body = json!({
        "model": model,
        "questions": questions,
        "state": { "currentMessage": "Decision connection conformance test" },
    });
    let (status, text) = transport
        .post(&endpoint, api_key, &format_request_body(&body, kind))
        .await?;
    if !(200..300).contains(&status) {
        return Err(format!("Decision provider returned status {status}"));
    }
    let raw: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    normalize_response(&contract, &raw)
}
