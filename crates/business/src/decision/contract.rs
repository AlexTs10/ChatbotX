//! `decision/contracts.ts` + `decision/adapters.ts`: the stored profile
//! contract, the provider request, the provider answer and the thresholds.

use std::collections::BTreeMap;

use chatbotx_core::decision_config::{
    DecisionForm, DecisionProfileForm, compile_noul_instructions,
};
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result};

/// One answer (`decisionAnswerSchema`).
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Choice {
        choice: String,
        confidence: Option<f64>,
        probabilities: Option<BTreeMap<String, f64>>,
    },
    Score {
        score: f64,
        confidence: Option<f64>,
    },
    Noul {
        probability_true: f64,
        confidence: Option<f64>,
    },
}

impl Answer {
    pub fn kind(&self) -> &'static str {
        match self {
            Answer::Choice { .. } => "choice",
            Answer::Score { .. } => "score",
            Answer::Noul { .. } => "noul",
        }
    }

    pub fn confidence(&self) -> Option<f64> {
        match self {
            Answer::Choice { confidence, .. }
            | Answer::Score { confidence, .. }
            | Answer::Noul { confidence, .. } => *confidence,
        }
    }

    /// `outputValue` / `metadataValue`: the mapped diagnostic as text.
    pub fn output_value(&self, value: &str) -> Option<String> {
        match (value, self) {
            ("choice", Answer::Choice { choice, .. }) => Some(choice.clone()),
            ("score", Answer::Score { score, .. }) => Some(js_number(*score)),
            (
                "probabilityTrue",
                Answer::Noul {
                    probability_true, ..
                },
            ) => Some(js_number(*probability_true)),
            ("confidence", answer) => answer.confidence().map(js_number),
            (
                "selectedProbability",
                Answer::Choice {
                    choice,
                    probabilities: Some(p),
                    ..
                },
            ) => p.get(choice).copied().map(js_number),
            _ => None,
        }
    }
}

/// JavaScript `String(number)` for the values a provider returns.
pub fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// A question of a contract, reduced to what an answer is checked against.
#[derive(Debug, Clone, PartialEq)]
pub enum Question {
    Choice { key: String, values: Vec<String> },
    Score { key: String, min: f64, max: f64 },
    Noul { key: String },
}

impl Question {
    pub fn key(&self) -> &str {
        match self {
            Question::Choice { key, .. } | Question::Score { key, .. } | Question::Noul { key } => {
                key
            }
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Question::Choice { .. } => "choice",
            Question::Score { .. } => "score",
            Question::Noul { .. } => "noul",
        }
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// `stableKeySchema`: `^[a-z][a-z0-9_]*$`, 1..=64.
pub fn is_stable_key(text: &str) -> bool {
    let mut chars = text.chars();
    text.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The questions of a contract `Value` (`questions[]`); `None` = malformed.
pub fn questions(contract: &Value) -> Option<Vec<Question>> {
    let items = contract.get("questions")?.as_array()?;
    items
        .iter()
        .map(|q| {
            let key = str_of(q, "key")?.to_owned();
            match str_of(q, "type")? {
                "choice" => Some(Question::Choice {
                    key,
                    values: q
                        .get("options")?
                        .as_array()?
                        .iter()
                        .map(|o| str_of(o, "value").map(str::to_owned))
                        .collect::<Option<_>>()?,
                }),
                "score" => {
                    let levels = q.get("levels")?.as_array()?;
                    let value = |l: Option<&Value>| l?.get("value")?.as_f64();
                    Some(Question::Score {
                        key,
                        min: value(levels.first())?,
                        max: value(levels.last())?,
                    })
                }
                "noul" => Some(Question::Noul { key }),
                _ => None,
            }
        })
        .collect()
}

/// `decisionProfileContractSchema.parse(...).questions[0]` (the stored
/// `result` question, raw). `None` = not a profile contract.
pub fn profile_question(contract: &Value) -> Option<&Value> {
    let inputs = contract.get("inputs")?.as_array()?;
    let items = contract.get("questions")?.as_array()?;
    let ok_input = inputs.len() == 1
        && str_of(&inputs[0], "key") == Some("currentMessage")
        && inputs[0].get("required") == Some(&Value::Bool(true));
    if !ok_input || items.len() != 1 || str_of(&items[0], "key") != Some("result") {
        return None;
    }
    let question = &items[0];
    let text = |k: &str| str_of(question, k).is_some_and(|t| !t.trim().is_empty());
    let ok = text("instructions")
        && match str_of(question, "type")? {
            "choice" => question
                .get("options")
                .and_then(Value::as_array)
                .is_some_and(|o| o.len() >= 2),
            "score" => question
                .get("levels")
                .and_then(Value::as_array)
                .is_some_and(|l| {
                    l.len() >= 2
                        && l.iter().enumerate().all(|(i, level)| {
                            level.get("value").and_then(Value::as_f64) == Some((i + 1) as f64)
                        })
                }),
            "noul" => text("trueCriteria") && text("falseCriteria"),
            _ => false,
        };
    ok.then_some(question)
}

const FIRST_INPUT: fn() -> Value = || json!([{ "key": "currentMessage", "required": true }]);

fn humanize(value: &str) -> String {
    let spaced = value.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => spaced,
    }
}

/// `compileContract` (profile-service): the stored contract of a form.
pub fn compile_contract(form: &DecisionProfileForm) -> Result<Value> {
    let question = match &form.decision {
        DecisionForm::Choice {
            instructions,
            options,
        } => json!({
            "instructions": instructions,
            "key": "result",
            "label": form.name,
            "options": options.iter().map(|o| json!({
                "description": o.description,
                "label": humanize(&o.value),
                "value": o.value,
            })).collect::<Vec<_>>(),
            "type": "choice",
        }),
        DecisionForm::Score {
            instructions,
            levels,
        } => json!({
            "instructions": instructions,
            "key": "result",
            "label": form.name,
            "levels": levels.iter().enumerate().map(|(i, description)| json!({
                "description": description,
                "label": (i + 1).to_string(),
                "value": i + 1,
            })).collect::<Vec<_>>(),
            "type": "score",
        }),
        DecisionForm::Noul {
            false_criteria,
            instructions,
            true_criteria,
        } => {
            let compiled = compile_noul_instructions(instructions, true_criteria, false_criteria);
            if compiled.encode_utf16().count() > 1000 {
                return Err(BusinessError::exception(
                    "Noul instructions exceed the 1,000-character provider limit",
                    "invalidDecisionContract",
                    400,
                ));
            }
            json!({
                "falseCriteria": false_criteria,
                "instructions": instructions,
                "key": "result",
                "label": form.name,
                "trueCriteria": true_criteria,
                "type": "noul",
            })
        }
    };
    Ok(json!({ "inputs": FIRST_INPUT(), "questions": [question] }))
}

/// `toPublicDecisionProfile`'s `decision`: the authoring shape of a stored
/// question.
pub fn public_decision(question: &Value) -> Value {
    match str_of(question, "type") {
        Some("choice") => json!({
            "instructions": question["instructions"],
            "options": question["options"].as_array().into_iter().flatten().map(|o| json!({
                "description": o["description"],
                "value": o["value"],
            })).collect::<Vec<_>>(),
            "type": "choice",
        }),
        Some("score") => json!({
            "instructions": question["instructions"],
            "levels": question["levels"].as_array().into_iter().flatten()
                .map(|l| json!({ "description": l["description"] })).collect::<Vec<_>>(),
            "type": "score",
        }),
        _ => json!({
            "falseCriteria": question["falseCriteria"],
            "instructions": question["instructions"],
            "trueCriteria": question["trueCriteria"],
            "type": "noul",
        }),
    }
}

/// `compileDecisionProfileRequest`: the minimal wire form of the `result`
/// question.
pub fn compile_profile_request(question: &Value) -> Value {
    let instructions = question["instructions"].clone();
    let body = match str_of(question, "type") {
        Some("choice") => json!({
            "criteria": question["options"].as_array().into_iter().flatten()
                .filter_map(|o| Some((str_of(o, "value")?.to_owned(), o["description"].clone())))
                .collect::<Map<_, _>>(),
            "instructions": instructions,
            "type": "choice",
        }),
        Some("score") => json!({
            "criteria": question["levels"].as_array().into_iter().flatten()
                .map(|l| l["description"].clone()).collect::<Vec<_>>(),
            "instructions": instructions,
            "type": "score",
        }),
        _ => json!({
            "instructions": compile_noul_instructions(
                str_of(question, "instructions").unwrap_or(""),
                str_of(question, "trueCriteria").unwrap_or(""),
                str_of(question, "falseCriteria").unwrap_or(""),
            ),
            "type": "noul",
        }),
    };
    json!({ "questions": { "result": body } })
}

/// `formatDecisionRequestBody`: the criteria format of the built-in
/// providers (`options` / `levels` become `criteria`).
pub fn format_request_body(provider_kind: &str, body: &Value) -> Value {
    if provider_kind == "systemOneCompatible" {
        return body.clone();
    }
    let mut out = body.clone();
    let Some(questions) = body.get("questions").and_then(Value::as_object) else {
        return out;
    };
    let converted: Map<String, Value> = questions
        .iter()
        .map(|(key, question)| {
            let mut q = question.as_object().cloned().unwrap_or_default();
            let label_or = |item: &Value| {
                item.get("description")
                    .filter(|d| !d.is_null())
                    .or_else(|| item.get("label"))
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            match (str_of(question, "type"), q.get("options"), q.get("levels")) {
                (Some("choice"), Some(Value::Array(options)), _) => {
                    let criteria: Map<String, Value> = options
                        .iter()
                        .map(|o| (str_of(o, "value").unwrap_or("").to_owned(), label_or(o)))
                        .collect();
                    q.remove("options");
                    q.insert("criteria".into(), Value::Object(criteria));
                }
                (Some("score"), _, Some(Value::Array(levels))) => {
                    let criteria: Vec<Value> = levels.iter().map(label_or).collect();
                    q.remove("levels");
                    q.insert("criteria".into(), Value::Array(criteria));
                }
                _ => {}
            }
            (key.clone(), Value::Object(q))
        })
        .collect();
    out["questions"] = Value::Object(converted);
    out
}

fn opt_unit(map: &Map<String, Value>, key: &str) -> std::result::Result<Option<f64>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| (0.0..=1.0).contains(n))
            .map(Some)
            .ok_or(()),
    }
}

/// `decisionAnswerSchema` (strict).
fn decision_answer(value: &Value) -> Option<Answer> {
    let map = value.as_object()?;
    let allowed: &[&str] = match str_of(value, "type")? {
        "choice" => &["type", "choice", "confidence", "probabilities"],
        "score" => &["type", "score", "confidence"],
        "noul" => &["type", "probabilityTrue", "confidence"],
        _ => return None,
    };
    if map.keys().any(|k| !allowed.contains(&k.as_str())) {
        return None;
    }
    let confidence = opt_unit(map, "confidence").ok()?;
    Some(match str_of(value, "type")? {
        "choice" => {
            let choice = str_of(value, "choice").filter(|c| is_stable_key(c))?;
            let probabilities = match map.get("probabilities") {
                None => None,
                Some(p) => Some(
                    p.as_object()?
                        .iter()
                        .map(|(k, v)| {
                            let n = v.as_f64().filter(|n| (0.0..=1.0).contains(n))?;
                            is_stable_key(k).then(|| (k.clone(), n))
                        })
                        .collect::<Option<BTreeMap<_, _>>>()?,
                ),
            };
            Answer::Choice {
                choice: choice.to_owned(),
                confidence,
                probabilities,
            }
        }
        "score" => Answer::Score {
            score: map.get("score")?.as_f64()?,
            confidence,
        },
        _ => Answer::Noul {
            probability_true: opt_unit(map, "probabilityTrue").ok()??,
            confidence,
        },
    })
}

fn some_num(map: &Map<String, Value>, key: &str) -> std::result::Result<Option<f64>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or(()),
    }
}

/// `providerAnswerSchema` + `normalizeProviderAnswers` for one answer.
fn provider_answer(value: &Value) -> Option<Value> {
    let map = value.as_object()?;
    let confidence = some_num(map, "confidence").ok()?;
    let mut out = Map::new();
    let kind = str_of(value, "type")?;
    out.insert("type".into(), Value::from(kind));
    match kind {
        "choice" => {
            out.insert("choice".into(), Value::from(str_of(value, "choice")?));
            if let Some(p) = map.get("probabilities") {
                if !p.as_object()?.values().all(Value::is_number) {
                    return None;
                }
                out.insert("probabilities".into(), p.clone());
            }
        }
        "score" => {
            out.insert("score".into(), Value::from(map.get("score")?.as_f64()?));
        }
        "noul" => {
            let noul = some_num(map, "noul").ok()?;
            let probability = some_num(map, "probability").ok()?;
            out.insert("probabilityTrue".into(), Value::from(probability.or(noul)?));
        }
        _ => return None,
    }
    if let Some(c) = confidence {
        out.insert("confidence".into(), Value::from(c));
    }
    Some(Value::Object(out))
}

fn answers_of(answers: &Value) -> Option<BTreeMap<String, Answer>> {
    let map = answers.as_object()?;
    let normalized: Option<Vec<Value>> = map.values().map(provider_answer).collect();
    let values: Vec<Value> = normalized.unwrap_or_else(|| map.values().cloned().collect());
    map.keys()
        .zip(values.iter())
        .map(|(k, v)| {
            if is_stable_key(k) {
                decision_answer(v).map(|a| (k.clone(), a))
            } else {
                None
            }
        })
        .collect()
}

/// `nativeResultSchema` → answers.
fn native_answers(raw: &Value) -> Option<BTreeMap<String, Answer>> {
    let map = raw.as_object()?;
    if map.keys().any(|k| k != "answers" && k != "providerModel")
        || !str_of(raw, "providerModel").is_some_and(|m| !m.is_empty())
    {
        return None;
    }
    let mut out = BTreeMap::new();
    for answer in map.get("answers")?.as_array()? {
        let a = answer.as_object()?;
        let allowed: &[&str] = match str_of(answer, "type")? {
            "choice" => &["type", "name", "confidence", "probabilities", "selectedId"],
            "score" => &["type", "name", "confidence", "score"],
            "noul" => &["type", "name", "confidence", "probability"],
            _ => return None,
        };
        if a.keys().any(|k| !allowed.contains(&k.as_str())) {
            return None;
        }
        let mut normalized = Map::new();
        normalized.insert("type".into(), answer["type"].clone());
        if let Some(c) = a.get("confidence") {
            normalized.insert("confidence".into(), c.clone());
        }
        match str_of(answer, "type")? {
            "choice" => {
                normalized.insert("choice".into(), Value::from(str_of(answer, "selectedId")?));
                if let Some(p) = a.get("probabilities") {
                    normalized.insert("probabilities".into(), p.clone());
                }
            }
            "score" => {
                normalized.insert("score".into(), a.get("score")?.clone());
            }
            _ => {
                normalized.insert("probabilityTrue".into(), a.get("probability")?.clone());
            }
        }
        let name = str_of(answer, "name")?;
        if !is_stable_key(name) {
            return None;
        }
        out.insert(
            name.to_owned(),
            decision_answer(&Value::Object(normalized))?,
        );
    }
    Some(out)
}

/// `normalizeDecisionProviderResponse`: the answers, checked against the
/// contract (`score` is shifted to the 1-based Profile scale).
pub fn normalize_provider_response(
    contract: &[Question],
    raw: &Value,
) -> std::result::Result<BTreeMap<String, Answer>, String> {
    let model_ok = |v: &Value| match v.get("model") {
        None => false,
        Some(m) => m
            .as_str()
            .is_some_and(|m| !m.trim().is_empty() && m.len() <= 256),
    };
    let primary = raw
        .as_object()
        .filter(|_| model_ok(raw))
        .and_then(|_| answers_of(raw.get("answers")?));
    let answers = match primary {
        Some(answers) => answers,
        None => native_answers(raw).ok_or("Decision provider returned an invalid result")?,
    };
    if answers.len() != contract.len()
        || answers
            .keys()
            .any(|k| !contract.iter().any(|q| q.key() == k))
    {
        return Err("Decision provider returned a missing or unknown answer".into());
    }
    let mut out = BTreeMap::new();
    for (key, answer) in answers {
        let question = contract.iter().find(|q| q.key() == key).ok_or("unknown")?;
        if question.kind() != answer.kind() {
            return Err("Decision provider answer type does not match Profile".into());
        }
        let answer = match (question, answer) {
            (Question::Choice { values, .. }, Answer::Choice { choice, .. })
                if !values.contains(&choice) =>
            {
                return Err("Decision provider returned an invalid choice".into());
            }
            (Question::Score { min, max, .. }, Answer::Score { score, confidence }) => {
                let shifted = score + 1.0;
                if shifted < *min || shifted > *max {
                    return Err(
                        "Decision provider returned a score outside the Profile range".into(),
                    );
                }
                Answer::Score {
                    score: shifted,
                    confidence,
                }
            }
            (_, answer) => answer,
        };
        out.insert(key, answer);
    }
    Ok(out)
}

/// `evaluateDecisionThreshold` over a stored `thresholdConfig`.
pub fn meets_threshold(answer: &Answer, threshold: Option<&Value>) -> bool {
    let Some(config) = threshold.filter(|v| !v.is_null()) else {
        return true;
    };
    let minimum = config.get("minimum").and_then(Value::as_f64).unwrap_or(0.0);
    match str_of(config, "type") {
        Some("choice_confidence") => match answer {
            Answer::Choice {
                confidence: Some(c),
                ..
            } => *c >= minimum,
            _ => false,
        },
        Some("noul_true_probability") => match answer {
            Answer::Noul {
                probability_true, ..
            } => *probability_true >= minimum,
            _ => false,
        },
        _ => {
            let Answer::Score { score, .. } = answer else {
                return false;
            };
            let value = config.get("value").and_then(Value::as_f64).unwrap_or(0.0);
            match str_of(config, "operator") {
                Some("gt") => *score > value,
                Some("gte") => *score >= value,
                Some("lt") => *score < value,
                _ => *score <= value,
            }
        }
    }
}

/// The three-question contract of a connection test.
pub fn conformance_contract() -> Value {
    json!({
        "inputs": FIRST_INPUT(),
        "questions": [
            {
                "instructions": "Classify the test input.",
                "key": "choice_test",
                "label": "Choice",
                "options": [
                    { "description": "First test choice.", "label": "One", "value": "one" },
                    { "description": "Second test choice.", "label": "Two", "value": "two" },
                ],
                "type": "choice",
            },
            {
                "instructions": "Score the test input.",
                "key": "score_test",
                "label": "Score",
                "levels": [
                    { "description": "Low test score.", "label": "1", "value": 1 },
                    { "description": "High test score.", "label": "2", "value": 2 },
                ],
                "type": "score",
            },
            {
                "instructions": "Evaluate the test input.",
                "key": "noul_test",
                "label": "Noul",
                "type": "noul",
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conformance() -> Vec<Question> {
        questions(&conformance_contract()).unwrap()
    }

    #[test]
    fn provider_answers_are_normalized_and_score_is_shifted() {
        let raw = json!({
            "model": "m",
            "answers": {
                "choice_test": { "type": "choice", "choice": "one", "confidence": 0.9 },
                "score_test": { "type": "score", "score": 1, "confidence": 0.8 },
                "noul_test": { "type": "noul", "probability": 0.25, "confidence": 0.7 },
            },
        });
        let answers = normalize_provider_response(&conformance(), &raw).unwrap();
        assert_eq!(
            answers["score_test"],
            Answer::Score {
                score: 2.0,
                confidence: Some(0.8)
            }
        );
        assert_eq!(
            answers["noul_test"],
            Answer::Noul {
                probability_true: 0.25,
                confidence: Some(0.7)
            }
        );
    }

    #[test]
    fn native_answers_and_failures() {
        let raw = json!({
            "providerModel": "p",
            "answers": [
                { "type": "choice", "name": "choice_test", "selectedId": "two" },
                { "type": "score", "name": "score_test", "score": 0 },
                { "type": "noul", "name": "noul_test", "probability": 1 },
            ],
        });
        assert!(normalize_provider_response(&conformance(), &raw).is_ok());
        let missing = json!({ "model": "m", "answers": { "choice_test": { "type": "choice", "choice": "one" } } });
        assert!(normalize_provider_response(&conformance(), &missing).is_err());
        let bad_choice = json!({ "model": "m", "answers": {
            "choice_test": { "type": "choice", "choice": "three" },
            "score_test": { "type": "score", "score": 0 },
            "noul_test": { "type": "noul", "noul": 0.5 },
        } });
        assert!(normalize_provider_response(&conformance(), &bad_choice).is_err());
        let out_of_range = json!({ "model": "m", "answers": {
            "choice_test": { "type": "choice", "choice": "one" },
            "score_test": { "type": "score", "score": 2 },
            "noul_test": { "type": "noul", "noul": 0.5 },
        } });
        assert!(normalize_provider_response(&conformance(), &out_of_range).is_err());
        assert!(normalize_provider_response(&conformance(), &json!("text")).is_err());
    }

    #[test]
    fn criteria_format_for_builtin_providers() {
        let body = json!({ "model": "m", "questions": {
            "choice_test": conformance_contract()["questions"][0],
            "score_test": conformance_contract()["questions"][1],
        } });
        assert_eq!(format_request_body("systemOneCompatible", &body), body);
        let out = format_request_body("typesafe", &body);
        assert_eq!(
            out["questions"]["choice_test"]["criteria"],
            json!({ "one": "First test choice.", "two": "Second test choice." })
        );
        assert!(out["questions"]["choice_test"].get("options").is_none());
        assert_eq!(
            out["questions"]["score_test"]["criteria"],
            json!(["Low test score.", "High test score."])
        );
    }

    #[test]
    fn thresholds() {
        let choice = Answer::Choice {
            choice: "a".into(),
            confidence: Some(0.5),
            probabilities: None,
        };
        assert!(meets_threshold(&choice, None));
        assert!(!meets_threshold(
            &choice,
            Some(&json!({ "type": "choice_confidence", "minimum": 0.6 }))
        ));
        let score = Answer::Score {
            score: 3.0,
            confidence: None,
        };
        assert!(meets_threshold(
            &score,
            Some(&json!({ "type": "score", "operator": "gte", "value": 3 }))
        ));
        assert!(!meets_threshold(
            &score,
            Some(&json!({ "type": "score", "operator": "lt", "value": 3 }))
        ));
        assert!(!meets_threshold(
            &score,
            Some(&json!({ "type": "noul_true_probability", "minimum": 0.1 }))
        ));
    }

    #[test]
    fn output_values() {
        let choice = Answer::Choice {
            choice: "a".into(),
            confidence: Some(0.5),
            probabilities: Some(BTreeMap::from([("a".to_owned(), 0.75)])),
        };
        assert_eq!(choice.output_value("choice").as_deref(), Some("a"));
        assert_eq!(
            choice.output_value("selectedProbability").as_deref(),
            Some("0.75")
        );
        assert_eq!(choice.output_value("score"), None);
        let score = Answer::Score {
            score: 2.0,
            confidence: None,
        };
        assert_eq!(score.output_value("score").as_deref(), Some("2"));
        assert_eq!(score.output_value("confidence"), None);
    }

    #[test]
    fn humanize_and_stable_keys() {
        assert_eq!(humanize("support_request"), "Support request");
        assert!(is_stable_key("a_1"));
        assert!(!is_stable_key("1a"));
        assert!(!is_stable_key("A"));
    }
}
