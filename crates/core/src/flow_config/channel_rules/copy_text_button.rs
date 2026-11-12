//! Port of `packages/flow-config/src/channel-rules/copy-text-button.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `isCopyTextButton`.
pub fn is_copy_text_button(button: &Value) -> bool {
    button["buttonType"] == "copyText"
}

/// `sendsAsNativeCopyButton(channel, button)`.
pub fn sends_as_native_copy_button(channel: Option<&str>, button: &Value) -> bool {
    is_copy_text_button(button)
        && is_native_button_type(channel, "copyText")
        && is_copyable_text(str_field(&button["beforeStep"], "text"))
}
