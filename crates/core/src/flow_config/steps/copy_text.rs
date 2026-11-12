//! Port of `packages/flow-config/src/steps/copy-text.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const COPY_TEXT_MAX: usize = 256;

pub const COPY_TEXT_EXPRESSION_MAX: usize = 1000;

schema!(pub fn copy_text_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("copyText")).describe("Step type discriminator: \"copyText\".")),
("text", z::string().trim().min(1).refine(is_valid_authored_copy_text, Some("constraintExceeded"), None).describe("Text the button copies to the contact's clipboard (max 256 characters once variables are resolved), e.g. a PIX \"copia e cola\" code. May embed {{variables}} (max 1000 characters authored)."))]));

default_fn!(pub fn copy_text_step_default_fn = json!({"id": create_id(), "stepType": "copyText", "text": ""}));

/// `isCopyableText`: 1..=COPY_TEXT_MAX characters.
pub fn is_copyable_text(value: &str) -> bool {
    let length = count_message_characters(Some(value));
    length > 0 && length <= COPY_TEXT_MAX
}

fn is_valid_authored_copy_text(value: &Value) -> bool {
    let text = value.as_str().unwrap_or("");
    if crate::utils::variables::contains_variable_placeholder(text) {
        crate::js::utf16_len(text) <= COPY_TEXT_EXPRESSION_MAX
    } else {
        count_message_characters(Some(text)) <= COPY_TEXT_MAX
    }
}
