//! Port of `packages/flow-config/__tests__/reference-registry-completeness.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use std::collections::{BTreeMap, BTreeSet};

use chatbotx_core::flow_config::z::Kind;

/// `*Id` / `*Ids` keys that are NOT a reference to one of this workspace's
/// entities, and why. A key that is neither registered nor listed here fails
/// the test, so adding a new id-holding step field forces a decision.
const NOT_A_WORKSPACE_REFERENCE: &[(&str, &str)] = &[
    // Third-party ids.
    ("accountId", "third-party account"),
    ("activeCampaignFieldId", "third-party field"),
    ("adAccountId", "Meta ad account"),
    ("automationId", "third-party automation"),
    ("campaignId", "third-party campaign"),
    ("contentIds", "Meta CAPI content ids"),
    ("credentialId", "payment provider saved-card credential"),
    ("conversionActionId", "Google Ads conversion action"),
    ("customAudienceId", "Meta custom audience"),
    ("dedupId", "event dedup key"),
    ("groupId", "third-party group"),
    ("listId", "third-party list"),
    ("listIds", "third-party lists"),
    // Ids inside a WhatsApp / Meta payload.
    ("buttonId", "button within the message"),
    ("flowSourceId", "Meta WhatsApp flow id"),
    ("flowVersionId", "WhatsApp flow token field"),
    ("navigateScreenId", "WhatsApp flow screen"),
    ("startScreenId", "WhatsApp flow screen"),
    ("templateId", "template start-step marker"),
    // Ids inside the flow graph itself.
    ("otherwiseId", "edge target within the same flow"),
    ("stepId", "step within the same flow"),
    // Runtime context carried by an event, never stored configuration.
    ("appointmentId", "runtime event data"),
    ("broadcastId", "runtime event data"),
    ("challengeId", "runtime event data"),
    ("commentAutomationId", "runtime event data"),
    ("commentId", "runtime event data"),
    ("contactInboxId", "runtime event data"),
    ("conversationId", "runtime event data"),
    ("dispatchId", "runtime event data"),
    ("sequenceStepId", "runtime event data"),
    // A constant, not an id.
    ("maxFieldIds", "limit constant"),
];

fn not_a_reference(key: &str) -> bool {
    NOT_A_WORKSPACE_REFERENCE.iter().any(|(k, _)| *k == key)
}

fn keys_of(table: &Value) -> Vec<String> {
    table
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn string_values_of(table: &Value) -> Vec<String> {
    table
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn last_segment(path: &str) -> String {
    path.rsplit('.').next().unwrap_or(path).to_string()
}

fn registered_keys() -> BTreeSet<String> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    keys.extend(keys_of(fc::reference_field_entity_kind()));
    keys.extend(keys_of(fc::reference_array_field_entity_kind()));
    keys.extend(keys_of(fc::discriminated_reference_fields()));
    keys.insert(fc::FLOW_REFERENCE_FIELD.to_string());
    keys.insert(fc::CROSS_FLOW_NODE_FIELD.to_string());
    for overrides in fc::step_reference_overrides().as_object().unwrap().values() {
        for path in keys_of(overrides) {
            keys.insert(last_segment(&path));
        }
    }
    keys
}

/// The TypeScript test scans `src/{steps,nodes}/*.ts` for `  fooId:` keys; the
/// Rust schemas spell keys as `("fooId", …)` tuples and `"fooId": …` JSON keys.
fn id_keys_declared_in_steps() -> BTreeSet<String> {
    let key = regex::Regex::new(r#"(?:\(|\{|, )"(\w+(?:Id|Ids))"(?::|,)"#).unwrap();
    let mut keys = BTreeSet::new();
    for dir in ["steps", "nodes"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/flow_config")
            .join(dir);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for captures in key.captures_iter(&source) {
                keys.insert(captures[1].to_string());
            }
        }
    }
    keys
}

// --- reference registry completeness ---

// adapted: scans the Rust step/node modules (`("fooId", …)` / `"fooId":` keys) instead of the TypeScript sources
#[test]
fn reference_registry_completeness_every_id_holding_step_field_is_registered_or_explicitly_not_a_reference()
 {
    let registered = registered_keys();
    let undecided: Vec<String> = id_keys_declared_in_steps()
        .into_iter()
        .filter(|key| !(registered.contains(key) || not_a_reference(key)))
        .collect();

    assert_eq!(undecided, Vec::<String>::new());
}

#[test]
fn reference_registry_completeness_nothing_is_both_registered_and_listed_as_not_a_reference() {
    let registered = registered_keys();
    let both: Vec<&str> = NOT_A_WORKSPACE_REFERENCE
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| registered.contains(*k))
        .collect();

    assert_eq!(both, Vec::<&str>::new());
}

#[test]
fn reference_registry_completeness_every_entity_kind_any_table_can_produce_has_a_policy() {
    let mut kinds: BTreeSet<String> = BTreeSet::new();
    kinds.extend(string_values_of(fc::reference_field_entity_kind()));
    kinds.extend(string_values_of(fc::reference_array_field_entity_kind()));
    kinds.extend(string_values_of(fc::prefixed_reference_entity_kind()));
    for reference in fc::contact_filter_field_reference()
        .as_object()
        .unwrap()
        .values()
    {
        kinds.insert(reference["kind"].as_str().unwrap().to_string());
    }
    for overrides in fc::step_reference_overrides().as_object().unwrap().values() {
        kinds.extend(string_values_of(overrides));
    }
    for config in fc::discriminated_reference_fields()
        .as_object()
        .unwrap()
        .values()
    {
        kinds.extend(string_values_of(&config["kindByDiscriminator"]));
    }
    kinds.insert("flow".to_string());
    kinds.insert("flowNode".to_string());
    kinds.remove(fc::EXTERNAL_REFERENCE_KIND);

    let policy = fc::reference_kind_policy();
    let without_policy: Vec<String> = kinds
        .into_iter()
        .filter(|kind| policy.get(kind).is_none())
        .collect();

    assert_eq!(without_policy, Vec::<String>::new());
}

#[test]
fn reference_registry_completeness_slot_shape_tables_only_name_registered_keys() {
    let registered = registered_keys();
    let unknown: Vec<String> = keys_of(fc::reference_slot_shape_by_key())
        .into_iter()
        .filter(|key| !registered.contains(key))
        .collect();
    let step_keys: Vec<String> = fc::step_reference_slot_shape()
        .as_object()
        .unwrap()
        .values()
        .flat_map(|shapes| keys_of(shapes).into_iter().map(|path| last_segment(&path)))
        .collect();

    assert_eq!(unknown, Vec::<String>::new());
    assert_eq!(
        step_keys
            .into_iter()
            .filter(|key| !registered.contains(key) && key != "id")
            .collect::<Vec<_>>(),
        Vec::<String>::new()
    );
}

// adapted: the step schemas come from `fc::exported_step_schemas()` (Rust has no `import.meta.glob`)
#[test]
fn reference_registry_completeness_every_registered_slot_s_shape_matches_its_step_s_zod_schema() {
    assert_eq!(slot_shape_mismatches(), Vec::<String>::new());
}

/// Every exported zod object that is a flow step (has a literal `stepType`).
fn step_schemas() -> BTreeMap<String, Schema> {
    let mut schemas = BTreeMap::new();
    for (_, schema) in fc::exported_step_schemas() {
        if !matches!(schema.kind(), Kind::Object { .. }) {
            continue;
        }
        let step_type = schema.field("stepType");
        if let Kind::Literal(values) = step_type.kind() {
            schemas.insert(values[0].as_str().unwrap_or_default().to_string(), schema);
        }
    }
    schemas
}

/// The shape of a slot from its schema, and the schema once the wrappers are peeled.
fn peel(schema: &Schema) -> (&'static str, Schema) {
    let mut current = schema.clone();
    let mut optional = false;
    let mut nullable = false;
    loop {
        match current.kind() {
            Kind::Optional(_) => {
                optional = true;
                current = current.unwrap();
            }
            Kind::Nullable(_) => {
                nullable = true;
                current = current.unwrap();
            }
            Kind::Default(..) => {
                optional = true;
                current = current.unwrap();
            }
            _ => break,
        }
    }
    if matches!(current.kind(), Kind::Array(_)) {
        return ("arrayItem", current);
    }
    if nullable {
        return ("nullable", current);
    }
    (if optional { "optional" } else { "required" }, current)
}

struct FoundSlot {
    step_type: String,
    path: String,
    shape: &'static str,
    /// The slot sits in a Condition row, whose id slots removal always treats as optional.
    in_condition_row: bool,
}

/// Registered slots found in `schema`, with the path relative to the step (array levels are transparent, as in removal).
fn find_slots(
    step_type: &str,
    schema: &Schema,
    registered: &BTreeSet<String>,
    prefix: &str,
    seen: &mut Vec<Schema>,
    out: &mut Vec<FoundSlot>,
) {
    let (_, inner) = peel(schema);
    if seen.iter().any(|s| s.same(&inner)) {
        return;
    }
    seen.push(inner.clone());
    match inner.kind() {
        Kind::Array(element) => find_slots(step_type, element, registered, prefix, seen, out),
        Kind::Union(options) | Kind::DiscriminatedUnion(_, options) => {
            for option in options {
                find_slots(step_type, option, registered, prefix, seen, out);
            }
        }
        Kind::Object { shape, .. } => {
            // A step nested in another (an email element's steps) is its own owner:
            // removal addresses its slots relative to it, and its own schema is walked.
            let nested_step_type = shape.iter().find(|(k, _)| k == "stepType");
            if !prefix.is_empty()
                && nested_step_type.is_some_and(|(_, s)| matches!(s.kind(), Kind::Literal(_)))
            {
                return;
            }
            let in_condition_row = shape.iter().any(|(k, _)| k == "field")
                && shape.iter().any(|(k, _)| k == "operator");
            for (key, child) in shape {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if registered.contains(key) {
                    out.push(FoundSlot {
                        step_type: step_type.to_string(),
                        path: path.clone(),
                        shape: peel(child).0,
                        in_condition_row,
                    });
                }
                find_slots(step_type, child, registered, &path, seen, out);
            }
        }
        _ => {}
    }
}

fn expected_shape(step_type: &str, path: &str, in_condition_row: bool) -> String {
    let key = last_segment(path);
    if in_condition_row {
        return "optional".to_string();
    }
    if fc::reference_array_field_entity_kind().get(&key).is_some() {
        return "arrayItem".to_string();
    }
    fc::step_reference_slot_shape()
        .get(step_type)
        .and_then(|shapes| shapes.get(path))
        .or_else(|| fc::reference_slot_shape_by_key().get(&key))
        .and_then(Value::as_str)
        .unwrap_or("required")
        .to_string()
}

fn slot_shape_mismatches() -> Vec<String> {
    let mut registered: BTreeSet<String> = BTreeSet::new();
    registered.extend(keys_of(fc::reference_field_entity_kind()));
    registered.extend(keys_of(fc::reference_array_field_entity_kind()));
    let mut found = Vec::new();
    for (step_type, schema) in step_schemas() {
        find_slots(
            &step_type,
            &schema,
            &registered,
            "",
            &mut Vec::new(),
            &mut found,
        );
    }
    let mut mismatches: Vec<String> = found
        .iter()
        .filter_map(|slot| {
            let table = expected_shape(&slot.step_type, &slot.path, slot.in_condition_row);
            (slot.shape != table).then(|| {
                format!(
                    "{}.{}: schema says {}, table says {}",
                    slot.step_type, slot.path, slot.shape, table
                )
            })
        })
        .collect();
    mismatches.sort();
    mismatches
}
