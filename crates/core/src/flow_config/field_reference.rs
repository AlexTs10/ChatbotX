//! Port of `packages/flow-config/src/field-reference.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn field_reference_kind() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"customField":"customField","botField":"botField"}"#)
            .expect("valid JSON")
    });
    &V
}

pub const BOT_FIELD_REFERENCE_PREFIX: &str = "bot_field";

/// `parseFieldReference` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldReference {
    CustomField { key: String },
    BotField { id: String },
}

const RESERVED_FIELD_REFERENCE_PREFIX: &str = "bot_field:";

fn bot_field_id(raw: &str) -> Option<&str> {
    let id = raw.strip_prefix(RESERVED_FIELD_REFERENCE_PREFIX)?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// `parseFieldReference`: `bot_field:<digits>` is a bot field, anything else
/// a custom field key.
pub fn parse_field_reference(value: &str) -> FieldReference {
    match bot_field_id(value) {
        Some(id) => FieldReference::BotField { id: id.to_owned() },
        None => FieldReference::CustomField {
            key: value.to_owned(),
        },
    }
}

/// `formatBotFieldReference(id)`.
pub fn format_bot_field_reference(id: &str) -> String {
    format!("{RESERVED_FIELD_REFERENCE_PREFIX}{id}")
}

/// `isBotFieldReference`.
pub fn is_bot_field_reference(value: &str) -> bool {
    bot_field_id(value).is_some()
}

/// The `zodFieldReference` check: a reserved-prefix value must be a valid
/// bot field token.
pub fn field_reference_ok(value: &Value) -> bool {
    let text = value.as_str().unwrap_or("");
    !text.starts_with(RESERVED_FIELD_REFERENCE_PREFIX) || is_bot_field_reference(text)
}

/// `zodFieldReference()`.
pub fn zod_field_reference() -> Schema {
    z::string().trim().min(1).refine(
        field_reference_ok,
        Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."),
        None,
    )
}

/// `isReservedFieldName`.
pub fn is_reserved_field_name(name: &str) -> bool {
    name.starts_with(RESERVED_FIELD_REFERENCE_PREFIX)
}

/// `zodFieldName()`.
pub fn zod_field_name() -> Schema {
    z::string().trim().min(1).max(255).refine(
        |name| !is_reserved_field_name(name.as_str().unwrap_or("")),
        Some("Name must not start with the reserved \"bot_field:\" prefix."),
        None,
    )
}
