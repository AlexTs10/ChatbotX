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

// HAND-PORT parseFieldReference:
// raw=>{const match=BOT_FIELD_REFERENCE_PATTERN.exec(raw);if(match){const id=match[1];if(id){return{kind:FieldReferenceKind.botField,id}}}return{kind:FieldReferenceKind.customField,key:raw}}

// HAND-PORT formatBotFieldReference:
// id=>`${BOT_FIELD_REFERENCE_PREFIX}${BOT_FIELD_REFERENCE_SEPARATOR}${id}`

// HAND-PORT isBotFieldReference:
// raw=>BOT_FIELD_REFERENCE_PATTERN.test(raw)

// HAND-PORT zodFieldReference:
// (message=`Value starting with "${RESERVED_FIELD_REFERENCE_PREFIX}" must be a valid bot field reference (e.g. "${formatBotFieldReference("123")}").`)=>z.string().trim().min(1).refine(value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value),{message})

// HAND-PORT isReservedFieldName:
// name=>name.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)

// HAND-PORT zodFieldName:
// (message=`Name must not start with the reserved "${RESERVED_FIELD_REFERENCE_PREFIX}" prefix.`)=>z.string().trim().min(1).max(255).refine(name=>!isReservedFieldName(name),{message})

// ---- stage 1 API (hand-ported in stage 2) ----

/// `parseFieldReference` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldReference {
    CustomField { key: String },
    BotField { id: String },
}

pub fn format_bot_field_reference(id: &str) -> String {
    todo!("P3.4")
}

pub fn is_bot_field_reference(value: &str) -> bool {
    todo!("P3.4")
}

pub fn is_reserved_field_name(name: &str) -> bool {
    todo!("P3.4")
}

pub fn parse_field_reference(value: &str) -> FieldReference {
    todo!("P3.4")
}

pub fn zod_field_name() -> Schema {
    todo!("P3.4")
}

pub fn zod_field_reference() -> Schema {
    todo!("P3.4")
}
