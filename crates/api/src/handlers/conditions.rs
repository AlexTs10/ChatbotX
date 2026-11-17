//! `features/conditions/schema` (`conditionSchema`, the union of
//! `allConditions`): one trigger / webhook event condition. Only the keys a
//! variant declares are kept (zod strips the rest).
//!
//! ponytail: a condition that fails every variant is one `invalid_union`
//! issue at its path (zod also nests each variant's issues under `errors`).

use chatbotx_business::webhook::ConditionInput;
use serde_json::{Map, Value};

use super::input::{Input, Seg, is_bigint_id};

const SIMPLE: &[&str] = &[
    "conversationTransferredToHuman",
    "conversationTransferredToBot",
    "newContact",
    "contactUnsubscribedFormBroadcast",
    "archived",
    "followUp",
    "conversationAssigned",
    "conversationUnassigned",
    "contactReferredANewContact",
    "contactReferredExistingContact",
    "contactBlockedBot",
    "contactUnblockedBot",
    "incomingCall",
    "missedAudioCall",
    "callEnded",
    "callRecorded",
    "callTranscribed",
    "newOrder",
    "orderAccepted",
    "orderShipped",
    "orderConcluded",
    "orderCancelled",
    "orderPaid",
    "orderRefunded",
    "cartAbandoned",
];
const WITH_SOURCE: &[&str] = &["subscribedToSequence", "unsubscribedFromSequence"];
/// `sourceId: union([zodBigintIdAsString(), literal("")]).nullish()` → `null`.
const OPTIONAL_REF: &[&str] = &[
    "shoppingCartSent",
    "userAskedAboutProduct",
    "productOrdered",
    "productAddedToCart",
    "productRemovedFromCart",
    "categoryAddedToCart",
    "pipelineItemCreated",
    "pipelineItemMovedToStage",
    "pipelineItemValueChanged",
    "pipelineItemStatusChanged",
    "pipelineItemPriorityChanged",
];

fn digits(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
}

/// `id`: `zodBigintAsString().optional()` (`digits`) or `z.string().optional()`.
fn id_of(item: &Map<String, Value>, digits_only: bool) -> Result<Option<i64>, ()> {
    match item.get("id") {
        None => Ok(None),
        Some(v) if digits_only && !digits(v) => Err(()),
        Some(Value::String(t)) => Ok(Some(t.parse().unwrap_or(-1))),
        Some(_) => Err(()),
    }
}

fn opt_string(item: &Map<String, Value>, key: &str) -> Result<Option<String>, ()> {
    match item.get(key) {
        None => Ok(None),
        Some(Value::String(t)) => Ok(Some(t.clone())),
        Some(_) => Err(()),
    }
}

fn string(item: &Map<String, Value>, key: &str, min: usize) -> Result<String, ()> {
    opt_string(item, key)?
        .filter(|t| t.chars().count() >= min)
        .ok_or(())
}

/// `union([zodBigintIdAsString(), literal("")]).nullish()` → id or `null`.
fn optional_ref(value: Option<&Value>) -> Result<Option<String>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(t)) if t.is_empty() => Ok(None),
        Some(Value::String(t)) if is_bigint_id(t) => Ok(Some(t.clone())),
        Some(_) => Err(()),
    }
}

/// `dateTimeBasedTrigger.value` (+ its `superRefine`).
fn date_time_value(value: Option<&Value>) -> Result<Value, ()> {
    let Some(Value::Object(v)) = value else {
        return Err(());
    };
    let trigger = v.get("triggerType").and_then(Value::as_str).ok_or(())?;
    if !["atTheDayOf", "before", "after"].contains(&trigger) {
        return Err(());
    }
    let mut out = Map::new();
    out.insert("triggerType".into(), trigger.into());
    // `z.coerce.number().min(1).optional()`.
    let time_value = match v.get("timeValue") {
        None => None,
        Some(raw) => {
            let n = match raw {
                Value::Number(n) => n.as_f64(),
                Value::String(t) => Some(if t.trim().is_empty() { 0.0 } else { t.trim().parse().map_err(|_| ())? }),
                Value::Bool(b) => Some(f64::from(u8::from(*b))),
                Value::Null => Some(0.0),
                _ => None,
            }
            .ok_or(())?;
            if !(n.is_finite() && n >= 1.0) {
                return Err(());
            }
            out.insert("timeValue".into(), crate::handlers::number(n));
            Some(n)
        }
    };
    let time_type = match v.get("timeType") {
        None => None,
        Some(Value::String(t)) if ["minutes", "hours", "days"].contains(&t.as_str()) => {
            out.insert("timeType".into(), t.clone().into());
            Some(t.clone())
        }
        Some(_) => return Err(()),
    };
    let at = opt_string(v, "at")?;
    if let Some(at) = &at {
        out.insert("at".into(), at.clone().into());
    }
    if let Some(tz) = opt_string(v, "timezone")? {
        if tz.chars().count() > 64 {
            return Err(());
        }
        out.insert("timezone".into(), tz.into());
    }
    let ok = if trigger == "atTheDayOf" {
        at.is_some_and(|a| !a.is_empty())
    } else {
        time_value.is_some() && time_type.is_some()
    };
    ok.then_some(Value::Object(out)).ok_or(())
}

fn parse(value: &Value) -> Result<ConditionInput, ()> {
    let item = value.as_object().ok_or(())?;
    let kind = item.get("type").and_then(Value::as_str).ok_or(())?;
    let mut out = ConditionInput {
        r#type: kind.to_string(),
        ..ConditionInput::default()
    };
    if SIMPLE.contains(&kind) {
        out.id = id_of(item, true)?;
    } else if WITH_SOURCE.contains(&kind) {
        out.id = id_of(item, true)?;
        out.source_id = Some(string(item, "sourceId", 1)?);
    } else if OPTIONAL_REF.contains(&kind) {
        out.id = id_of(item, true)?;
        out.source_id = optional_ref(item.get("sourceId"))?;
        if kind == "pipelineItemMovedToStage" {
            out.value = match item.get("value") {
                None => None,
                Some(Value::Null) => Some(Value::Null),
                Some(Value::Object(v)) => {
                    let stage = optional_ref(v.get("stageId"))?;
                    Some(serde_json::json!({ "stageId": stage }))
                }
                Some(_) => return Err(()),
            };
        }
    } else {
        out.id = id_of(item, false)?;
        match kind {
            "tagApplied" | "tagRemoved" => out.source_id = Some(string(item, "sourceId", 0)?),
            "customFieldValueChanged" => {
                out.source_id = Some(string(item, "sourceId", 1)?);
                out.operator = Some(string(item, "operator", 0)?);
                out.value = item.get("value").cloned();
            }
            "contactInfoUpdated" => {
                let source = string(item, "sourceId", 0)?;
                if source != "phone" && source != "email" {
                    return Err(());
                }
                out.source_id = Some(source);
            }
            "dateTimeBasedTrigger" => {
                out.source_id = Some(string(item, "sourceId", 1)?);
                out.operator = Some(string(item, "operator", 0)?);
                out.value = Some(date_time_value(item.get("value"))?);
            }
            _ => return Err(()),
        }
    }
    Ok(out)
}

/// One `conditionSchema` item at `path`; `None` = an issue was recorded.
pub fn condition_at(input: &mut Input, path: &[Seg], value: &Value) -> Option<ConditionInput> {
    match parse(value) {
        Ok(condition) => Some(condition),
        Err(()) => {
            input.issue(path, "invalid_union", "Invalid input");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    #[test]
    fn variants_keep_only_their_keys() {
        let c = parse(&json!({ "type": "tagApplied", "sourceId": "5", "operator": "x" })).unwrap();
        assert_eq!((c.source_id.as_deref(), c.operator), (Some("5"), None));
        let c = parse(&json!({ "type": "productOrdered", "sourceId": "" })).unwrap();
        assert_eq!(c.source_id, None);
        assert!(parse(&json!({ "type": "subscribedToSequence", "sourceId": "" })).is_err());
        assert!(parse(&json!({ "type": "newContact", "id": "x" })).is_err());
        assert!(parse(&json!({ "type": "unknown" })).is_err());
        assert!(parse(&json!({ "type": "contactInfoUpdated", "sourceId": "fax" })).is_err());
    }

    #[test]
    fn date_time_refinement() {
        let base = |value| {
            json!({ "type": "dateTimeBasedTrigger", "sourceId": "9", "operator": "eq", "value": value })
        };
        assert!(parse(&base(json!({ "triggerType": "before", "timeValue": "2", "timeType": "hours" }))).is_ok());
        assert!(parse(&base(json!({ "triggerType": "before", "timeValue": 2 }))).is_err());
        assert!(parse(&base(json!({ "triggerType": "atTheDayOf", "at": "" }))).is_err());
        assert!(parse(&base(json!({ "triggerType": "atTheDayOf", "at": "09:00" }))).is_ok());
    }
}
