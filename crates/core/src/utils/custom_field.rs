//! `packages/utils/src/custom-field.ts`.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::js;

string_enum! {
    /// `customFieldTypes`.
    pub enum CustomFieldType {
        ShortText = "shortText",
        Email = "email",
        PhoneNumber = "phoneNumber",
        Number = "number",
        Date = "date",
        Datetime = "datetime",
        Boolean = "boolean",
        LongText = "longText",
    }
}

string_enum! {
    /// `operatorTypes` (contact-filter comparison operators).
    pub enum OperatorType {
        In = "in",
        NotIn = "notIn",
        IsEmpty = "isEmpty",
        IsNotEmpty = "isNotEmpty",
        Eq = "eq",
        Ne = "ne",
        StartsWith = "startsWith",
        EndsWith = "endsWith",
        Contains = "contains",
        NotContains = "notContains",
        Lt = "lt",
        Lte = "lte",
        Gt = "gt",
        Gte = "gte",
        IsBetween = "isBetween",
        NotBetween = "notBetween",
        Used = "used",
    }
}

/// A custom-field row (or request) as the resolution helpers see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFieldRow {
    pub id: String,
    pub name: String,
    pub kind: CustomFieldType,
}

/// A requested field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFieldRequest {
    pub name: String,
    pub kind: CustomFieldType,
}

/// `customFieldResolutionKey`: `<type>:<trimmed lower-case name>`.
pub fn custom_field_resolution_key(name: &str, kind: CustomFieldType) -> String {
    format!("{}:{}", kind.as_str(), js::trim(name).to_lowercase())
}

/// `pickCustomFieldRow`: an exact trimmed-name match beats a case-only one,
/// then the lowest id (string order) wins.
pub fn pick_custom_field_row<'a>(
    current: Option<&'a CustomFieldRow>,
    candidate: &'a CustomFieldRow,
    requested_name: Option<&str>,
) -> &'a CustomFieldRow {
    let Some(current) = current else {
        return candidate;
    };
    if let Some(requested) = requested_name {
        let current_exact = js::trim(&current.name) == requested;
        let candidate_exact = js::trim(&candidate.name) == requested;
        if current_exact != candidate_exact {
            return if candidate_exact { candidate } else { current };
        }
    }
    if candidate.id < current.id {
        candidate
    } else {
        current
    }
}

/// `normalizeCustomFieldName`.
pub fn normalize_custom_field_name(name: &str) -> String {
    js::trim(name).to_lowercase()
}

/// The longest custom-field name.
pub const MAX_CUSTOM_FIELD_NAME_LENGTH: usize = 255;

/// `allocateRenamedCustomFieldName`: `<name>_<n>` for the smallest free
/// `n`, the base shortened (UTF-16 units) to stay within `max_length`.
pub fn allocate_renamed_custom_field_name(
    name: &str,
    taken: &HashSet<String>,
    max_length: usize,
) -> String {
    let base = js::trim(name);
    let mut suffix = 1usize;
    loop {
        let tail = format!("_{suffix}");
        let keep = max_length.saturating_sub(tail.len()).max(1);
        let candidate = format!("{}{tail}", js::slice_utf16(base, 0, keep));
        if !taken.contains(&normalize_custom_field_name(&candidate)) {
            return candidate;
        }
        suffix += 1;
    }
}

/// `CustomFieldResolutionPlan`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomFieldResolutionPlan {
    Reuse(CustomFieldRow),
    Create { planned_name: String },
    Rename { planned_name: String },
}

/// `planCustomFieldResolution`'s result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFieldResolution {
    /// In the order the install creates them.
    pub requests: Vec<CustomFieldRequest>,
    pub plans: BTreeMap<String, CustomFieldResolutionPlan>,
    /// The winning existing row per key.
    pub winners: HashMap<String, CustomFieldRow>,
}

/// `planCustomFieldResolution`.
pub fn plan_custom_field_resolution(
    existing: &[CustomFieldRow],
    fields: &[CustomFieldRequest],
) -> CustomFieldResolution {
    // `new Map(fields.map(…)).values()`: first-seen key order, last value.
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, CustomFieldRequest> = HashMap::new();
    for field in fields {
        let key = custom_field_resolution_key(&field.name, field.kind);
        if !by_key.contains_key(&key) {
            order.push(key.clone());
        }
        by_key.insert(key, field.clone());
    }
    let requests: Vec<CustomFieldRequest> = order.iter().map(|k| by_key[k].clone()).collect();
    let requested_name_by_key: HashMap<String, String> = requests
        .iter()
        .map(|f| {
            (
                custom_field_resolution_key(&f.name, f.kind),
                js::trim(&f.name).to_owned(),
            )
        })
        .collect();
    let mut winners: HashMap<String, CustomFieldRow> = HashMap::new();
    for row in existing {
        let key = custom_field_resolution_key(&row.name, row.kind);
        let winner = pick_custom_field_row(
            winners.get(&key),
            row,
            requested_name_by_key.get(&key).map(String::as_str),
        )
        .clone();
        winners.insert(key, winner);
    }
    let mut taken: HashSet<String> = existing
        .iter()
        .map(|r| normalize_custom_field_name(&r.name))
        .collect();
    let mut plans = BTreeMap::new();
    for field in &requests {
        let key = custom_field_resolution_key(&field.name, field.kind);
        if let Some(winner) = winners.get(&key) {
            plans.insert(key, CustomFieldResolutionPlan::Reuse(winner.clone()));
            continue;
        }
        let is_taken = taken.contains(&normalize_custom_field_name(&field.name));
        let planned_name = if is_taken {
            allocate_renamed_custom_field_name(&field.name, &taken, MAX_CUSTOM_FIELD_NAME_LENGTH)
        } else {
            field.name.clone()
        };
        taken.insert(normalize_custom_field_name(&planned_name));
        plans.insert(
            key,
            if is_taken {
                CustomFieldResolutionPlan::Rename { planned_name }
            } else {
                CustomFieldResolutionPlan::Create { planned_name }
            },
        );
    }
    CustomFieldResolution {
        requests,
        plans,
        winners,
    }
}

/// `BOOLEAN_TRUTHY_LITERALS`.
pub const BOOLEAN_TRUTHY_LITERALS: &[&str] = &["t", "true", "y", "yes", "on", "1"];
/// `BOOLEAN_FALSY_LITERALS`.
pub const BOOLEAN_FALSY_LITERALS: &[&str] = &["f", "false", "n", "no", "off", "0"];
/// `BOOLEAN_LITERAL_PATTERN_SOURCE`.
pub const BOOLEAN_LITERAL_PATTERN_SOURCE: &str = "^(t|true|y|yes|on|1|f|false|n|no|off|0)$";

/// `canonicalBooleanLiteral`: `"true"` / `"false"`, or `None`.
pub fn canonical_boolean_literal(raw: &str) -> Option<&'static str> {
    let literal = js::trim(raw).to_lowercase();
    if BOOLEAN_TRUTHY_LITERALS.contains(&literal.as_str()) {
        Some("true")
    } else if BOOLEAN_FALSY_LITERALS.contains(&literal.as_str()) {
        Some("false")
    } else {
        None
    }
}

/// `coerceBooleanLiteral`: blank or a falsy literal is `"false"`, anything
/// else `"true"`.
pub fn coerce_boolean_literal(raw: &str) -> &'static str {
    let literal = js::trim(raw).to_lowercase();
    if literal.is_empty() || BOOLEAN_FALSY_LITERALS.contains(&literal.as_str()) {
        "false"
    } else {
        "true"
    }
}

/// `canonicalNumberLiteral`: whatever `Number()` accepts, as
/// `String(number)`; blank or non-finite → `None`.
pub fn canonical_number_literal(raw: &str) -> Option<String> {
    let trimmed = js::trim(raw);
    if trimmed.is_empty() {
        return None;
    }
    let parsed = js::to_number(trimmed);
    parsed.is_finite().then(|| js::number_to_string(parsed))
}
