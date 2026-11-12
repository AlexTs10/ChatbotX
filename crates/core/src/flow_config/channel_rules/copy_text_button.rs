//! Port of `packages/flow-config/src/channel-rules/copy-text-button.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT isCopyTextButton:
// button=>button.buttonType===buttonTypes.enum.copyText

// HAND-PORT sendsAsNativeCopyButton:
// (channel,button)=>isCopyTextButton(button)&&isNativeButtonType(channel,buttonTypes.enum.copyText)&&isCopyableText(button.beforeStep.text)
