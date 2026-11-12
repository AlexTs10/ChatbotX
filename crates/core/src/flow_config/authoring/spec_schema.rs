//! Port of `packages/flow-config/src/authoring/spec-schema.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn flow_spec_step_types() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"[{"type":"send","description":"Sends one message (text, image, or file), optionally with quick-reply buttons."},{"type":"sendTemplate","description":"Sends an existing WhatsApp message template."},{"type":"sendProducts","description":"Sends products from the workspace catalog (single product, product list or carousel; use sendViewCatalog to open the whole catalog). Only on channels that support product messages."},{"type":"sendViewCatalog","description":"Shares the product catalog: a message with a button that opens it. Publish rejects it on channels that cannot deliver it (see `capabilities.get`'s `unsupportedSteps`); in an omnichannel flow (`channel` omitted) it is accepted but only sent to WhatsApp contacts, so set `channel` to check it at publish."},{"type":"wait","description":"Pauses the flow for a fixed duration before continuing."},{"type":"branch","description":"Splits the flow by contact-filter-style conditions. Terminal within its step list — nothing may follow a `branch` at the same level; continue inside `cases[].then` / `otherwise` instead."},{"type":"action","description":"Performs a workspace side-effect: tag, custom field, or conversation action."},{"type":"pipeline","description":"Runs a Pipelines (CRM kanban) action for the contact: addToPipeline, moveToStage, changeValue, assign, changeStatus, changePriority, changeDueDate, changeTitle, setCustomField, clearCustomField, changeDescription, removeFromPipeline, addComment or transfer. Pipelines, stages and fields are referenced by name/referenceName (see `pipelines.list` / `pipelines.get`). Success continues to the next step; `onError` / `onSkip` hold the steps for those outcomes."},{"type":"startFlow","description":"Starts another flow for the contact; this flow keeps running afterward (an ordinary step may follow)."},{"type":"addNote","description":"Adds an internal note to the conversation."},{"type":"goto","description":"Terminal — routes to an already-defined step instead of continuing. Must be the last step in its list."}]"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn flow_spec_schema = z::object(vec![("formatVersion", z::literal(json!(1)).describe("DSL format version. Always 1.")),
("name", z::string().trim().min(1).max(255).describe("Flow name (max 255 characters).")),
("channel", deps::channel_types().optional().describe("Channel this flow targets. Omit for any/omnichannel. sendTemplate steps always send over WhatsApp regardless of this value.")),
("steps", z::array(z::lazy(flow_spec_schema_steps_rec)).min(1).describe("Ordered steps executed from the flow's start node."))]).describe("A complete agent-authored flow DSL document."));

schema!(fn flow_spec_schema_steps_rec = z::discriminated_union("type", vec![z::object(vec![("type", z::literal(json!("send")).describe("Step type: \"send\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("text", z::string().trim().min(1).refine(flow_spec_schema_steps_rec_0_text_refine, Some("constraintExceeded"), None).optional().describe("Text message body (max 1000 characters). Exactly one of text/imageUrl/fileUrl is required.").meta(json!({"maxLength":1000}))),
("imageUrl", z::string().url().optional().describe("Image URL to send. Exactly one of text/imageUrl/fileUrl is required.")),
("fileUrl", z::string().url().optional().describe("File URL to send. Exactly one of text/imageUrl/fileUrl is required.")),
("buttons", z::array(z::lazy(|| z::object(vec![("text", z::string().trim().min(1).refine(flow_spec_schema_steps_rec_0_buttons_text_refine, Some("constraintExceeded"), None).describe("Button label shown to the contact (max 20 characters).").meta(json!({"maxLength":20}))),
("then", z::array(z::lazy(flow_spec_schema_steps_rec)).optional().describe("Steps to run when the contact taps this button. Omitted or empty means the button has no follow-up.")),
("call", z::string().trim().refine(is_valid_authored_phone_number, Some("invalidPhoneNumber"), None).describe("Number the button dials, in international format (E.164, e.g. +14155552671). May embed {{variables}} (max 100 characters) that resolve to such a number when the message is sent.").optional().describe("Turns the button into a Call Phone Number button: the number to dial, in international format (e.g. +14155552671), optionally embedding {{variables}}. Cannot be combined with `then`.")),
("copy", z::string().trim().min(1).refine(is_valid_authored_copy_text, Some("constraintExceeded"), None).describe("Text the button copies to the contact's clipboard (max 256 characters once variables are resolved), e.g. a PIX \"copia e cola\" code. May embed {{variables}} (max 1000 characters authored).").optional().describe("Turns the button into a Copy Text button: the text a tap copies to the contact's clipboard (max 256 characters once {{variables}} are resolved), e.g. a PIX code. Native on Telegram; other channels send it as a reply button that answers with the text. Cannot be combined with `then` or `call`."))]).refine(flow_spec_schema_steps_rec_0_buttons_refine, Some("A call button cannot have `then` steps."), Some(&["then"])).refine(flow_spec_schema_steps_rec_0_buttons_refine_2, Some("A copy button cannot have `then` steps."), Some(&["then"])).refine(flow_spec_schema_steps_rec_0_buttons_refine_3, Some("A button cannot both `call` and `copy`."), Some(&["copy"])).describe("A quick-reply button attached to a send step."))).max(3).optional().describe("Up to 3 quick-reply buttons attached to this message."))]).super_refine(flow_spec_schema_steps_rec_0_super_refine).describe("Sends one message (text, image, or file), optionally with quick-reply buttons."),
z::object(vec![("type", z::literal(json!("sendTemplate")).describe("Step type: \"sendTemplate\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("templateName", z::string().trim().min(1).describe("Name of an existing WhatsApp message template whose status is `APPROVED` (see `capabilities.get`'s `templates` list). Sent as-is, without dynamic variables."))]).describe("Sends an existing WhatsApp message template."),
z::object(vec![("type", z::literal(json!("sendProducts")).describe("Step type: \"sendProducts\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("display", send_products_displays().describe("How the products are shown. \"products\": one product, or several grouped in `sections`. \"carousel\": 2+ products as scrollable cards (one section, `body` only). Which displays and text limits a channel supports is in `capabilities.get`.")),
("header", z::string().trim().optional().describe("Header text.")),
("body", z::string().trim().optional().describe("Message text.")),
("footer", z::string().trim().optional().describe("Footer text.")),
("sections", z::array(z::object(vec![("title", z::string().trim().optional().describe("Section title.")),
("productIds", z::array(z::string().regex(r#"^\d+$"#, "")).describe("Product ids, from `products.listSendable`. A product may appear only once in the whole message."))])).optional().describe("Product sections. Needed for \"products\" and \"carousel\" (exactly one section)."))]).describe("Sends products from the workspace catalog (single product, product list or carousel; use sendViewCatalog to open the whole catalog). Only on channels that support product messages."),
z::object(vec![("type", z::literal(json!("sendViewCatalog")).describe("Step type: \"sendViewCatalog\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("text", z::string().trim().min(1).refine(flow_spec_schema_steps_rec_3_text_refine, Some("sendViewCatalogBodyTooLong"), None).describe("Message text shown above the catalog button (max 1024 characters, counted in UTF-16 units so an emoji may count as 2). May include `{{variable}}` placeholders.").meta(json!({"maxLength":1024}))),
("footer", z::string().trim().refine(flow_spec_schema_steps_rec_3_footer_refine, Some("sendViewCatalogFooterTooLong"), None).default(json!("")).describe("Optional small text under the message (max 60 characters, UTF-16 units; use an empty string for none). May include `{{variable}}` placeholders.").meta(json!({"maxLength":60}))),
("thumbnailProductId", z::string().trim().default(json!("")).describe("Optional catalog product whose image is shown on the message: its Meta catalog `retailerId` (the Content ID in Commerce Manager), as returned by `products.searchMetaCatalogItems`. Use an empty string to let the channel pick one."))]).describe("Shares the product catalog: a message with a button that opens it. Publish rejects it on channels that cannot deliver it (see `capabilities.get`'s `unsupportedSteps`); in an omnichannel flow (`channel` omitted) it is accepted but only sent to WhatsApp contacts, so set `channel` to check it at publish."),
z::object(vec![("type", z::literal(json!("wait")).describe("Step type: \"wait\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("duration", z::number().int().gt(0 as f64).describe("How long to wait, in `unit`s.")),
("unit", wait_step_delay_units().describe("Unit for `duration`."))]).describe("Pauses the flow for a fixed duration before continuing."),
z::object(vec![("type", z::literal(json!("branch")).describe("Step type: \"branch\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("cases", z::array(z::lazy(|| z::object(vec![("match", z::enum_(&["and", "or"]).default(json!("and")).describe("Whether every ('and') or any ('or') condition in `when` must match.")),
("when", z::array(z::object(vec![("field", z::string().min(1).describe("A static field name from `GET /v1/contacts/filter-fields`, or `customField:<name>` to reference a workspace custom field by name (resolved automatically — use the exact name from `contacts.listFilterFields`). `botField:` prefixed names are rejected by the compiler with error code `invalidSpec`; bot fields are reference-only data.")),
("operator", z::string().min(1).describe("One of the operators `GET /v1/contacts/filter-fields` lists for this field.")),
("value", z::union(vec![z::string(),
z::array(z::string()),
z::tuple(vec![z::string(),
z::string()])]).optional().describe("Comparison value. Omit for valueless operators (e.g. isEmpty/isNotEmpty). A two-element tuple is a between-range."))]).describe("One contact-filter-style condition for a branch case.")).min(1).describe("Conditions evaluated to determine whether this case matches.")),
("then", z::array(z::lazy(flow_spec_schema_steps_rec)).min(1).describe("Steps to run when this case matches."))]).describe("One ordered case in a branch step."))).min(1).describe("Ordered cases evaluated until one matches.")),
("otherwise", z::array(z::lazy(flow_spec_schema_steps_rec)).optional().describe("Steps to run when no case matches."))]).describe("Splits the flow by contact-filter-style conditions. Terminal within its step list — nothing may follow a `branch` at the same level; continue inside `cases[].then` / `otherwise` instead."),
z::object(vec![("type", z::literal(json!("action")).describe("Step type: \"action\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("action", z::enum_(&["addTags", "removeTags", "setCustomField", "assignConversation", "archiveConversation"]).describe("Which workspace action to perform.")),
("tagNames", z::array(z::string().trim().min(1)).optional().describe("Tag names. Required for `addTags`/`removeTags`.")),
("customFieldName", z::string().trim().min(1).optional().describe("Custom field name. Required for `setCustomField`.")),
("operation", z::enum_(&["set", "append", "prepend", "increase", "decrease", "clear", "formula", "setNow", "dateExpression", "addMinutes", "addHours", "addDays"]).optional().describe("How `setCustomField` applies `value` (default `set`): `set`, `append`/`prepend` (text), `increase`/`decrease`/`formula` (number), `setNow`/`dateExpression`/`addMinutes`/`addHours`/`addDays` (date, datetime) or `clear`. The field type must support it.")),
("value", z::string().optional().describe("Value to set. Required for `setCustomField`, except for the `clear` and `setNow` operations.")),
("assigneeId", z::string().optional().describe("Workspace member id to assign the conversation to. Optional for `assignConversation`; omit to unassign."))]).super_refine(flow_spec_schema_steps_rec_6_super_refine).describe("Performs a workspace side-effect: tag, custom field, or conversation action."),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Step type: \"pipeline\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("action", pipeline_step_actions().describe("Which pipeline action to run. Every action except `addToPipeline` acts on the contact's most recently created OPEN item in `pipeline`; with no open item the step takes the `onSkip` path.")),
("pipeline", z::string().trim().min(1).describe("The pipeline, by `referenceName` (or id). Get both from `pipelines.list` / `pipelines.get`.")),
("title", z::string().max(200).optional().describe("Item title; accepts {{variables}}. `addToPipeline` (default `{{full_name}}`), `changeTitle` (required).")),
("description", z::string().max(5000).optional().describe("Item description. `addToPipeline` (optional), `changeDescription` (required; empty clears it).")),
("status", z::enum_(&["open", "won", "lost"]).optional().describe("Item status. `addToPipeline` (default open), `changeStatus` (required).")),
("priority", z::enum_(&["low", "medium", "high"]).optional().describe("Item priority. Required for `changePriority`.")),
("stage", z::string().trim().min(1).optional().describe("Stage NAME within `pipeline` (from `pipelines.get`). Required for `moveToStage`; optional for `addToPipeline` (default: the first stage). Rejected as ambiguous when two stages share the name.")),
("assigneeMemberIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).optional().describe("Workspace member ids to assign (from `workspaceMembers.list`). `addToPipeline`, `assign` (replaces the assignees).")),
("assigneeInboxTeamIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).optional().describe("Inbox team ids to assign (from `inboxTeams.list`). `addToPipeline`, `assign` (replaces the teams).")),
("dueDate", z::discriminated_union("mode", vec![z::object(vec![("mode", z::literal(json!("none")).describe("No due date."))]),
z::object(vec![("mode", z::literal(json!("fixed")).describe("A fixed instant.")),
("at", z::string().iso_datetime(true, false, None).describe("Fixed ISO-8601 instant."))]),
z::object(vec![("mode", z::literal(json!("customField")).describe("Read from a contact date custom field.")),
("customFieldName", z::string().trim().min(1).describe("Name of a contact custom field of type date or datetime (see `contacts.listCustomFields`); a blank value clears the due date."))])]).describe("Due date: `{ mode: \"none\" }` leaves/clears it, `{ mode: \"fixed\", at }` sets an instant, `{ mode: \"customField\", customFieldName }` reads it from a contact date custom field.").optional().describe("Due date. `changeDueDate` (required), `addToPipeline` (default none).")),
("fieldValues", z::array(z::object(vec![("field", z::string().trim().min(1).describe("Pipeline custom field `referenceName`.")),
("value", z::string().describe("Value; accepts {{variables}}."))])).max(50).optional().describe("Pipeline custom field values set on the new item (`addToPipeline`).")),
("value", z::string().optional().describe("`changeValue`: opportunity value, a number >= 0 ({{variables}} allowed). `setCustomField`: the field value.")),
("field", z::string().trim().min(1).optional().describe("Pipeline custom field `referenceName` (from `pipelines.get`). Required for `setCustomField`.")),
("fields", z::array(z::string().trim().min(1)).max(50).optional().describe("Pipeline custom field `referenceName`s to clear. Required (non-empty) for `clearCustomField`.")),
("text", z::string().max(2000).optional().describe("Comment text; accepts {{variables}}. Required for `addComment`.")),
("destinationPipeline", z::string().trim().min(1).optional().describe("Target pipeline, by `referenceName` (or id). Required for `transfer`.")),
("destinationStage", z::string().trim().min(1).optional().describe("Stage NAME in `destinationPipeline`. Optional for `transfer` (default: its first stage).")),
("onError", z::array(z::lazy(flow_spec_schema_steps_rec)).optional().describe("Steps to run when the action fails (invalid input, unknown pipeline/stage, conflict). Omitted: the flow continues to the next step.")),
("onSkip", z::array(z::lazy(flow_spec_schema_steps_rec)).optional().describe("Steps to run when the contact has no open item in `pipeline`. Not allowed for `addToPipeline`, which never skips."))]).super_refine(flow_spec_schema_steps_rec_7_super_refine).describe("Runs a Pipelines (CRM kanban) action for the contact: addToPipeline, moveToStage, changeValue, assign, changeStatus, changePriority, changeDueDate, changeTitle, setCustomField, clearCustomField, changeDescription, removeFromPipeline, addComment or transfer. Pipelines, stages and fields are referenced by name/referenceName (see `pipelines.list` / `pipelines.get`). Success continues to the next step; `onError` / `onSkip` hold the steps for those outcomes."),
z::object(vec![("type", z::literal(json!("startFlow")).describe("Step type: \"startFlow\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("flowName", z::string().trim().min(1).describe("Name of another existing flow in this workspace (see `flows.list`)."))]).describe("Starts another flow for the contact; this flow keeps running afterward (an ordinary step may follow)."),
z::object(vec![("type", z::literal(json!("addNote")).describe("Step type: \"addNote\".")),
("id", z::string().min(1).optional().describe("Stable id for this step. Omit to auto-generate one. Set it explicitly when a `goto` elsewhere needs to jump to this exact step.")),
("note", z::string().trim().min(1).refine(flow_spec_schema_steps_rec_9_note_refine, Some("constraintExceeded"), None).describe("Internal note text (max 1000 characters) — never shown to the contact.").meta(json!({"maxLength":1000})))]).describe("Adds an internal note to the conversation."),
z::object(vec![("type", z::literal(json!("goto")).describe("Step type: \"goto\".")),
("targetId", z::string().min(1).describe("The `id` of an earlier step in this flow spec to jump to, instead of continuing linearly."))]).describe("Terminal — routes to an already-defined step instead of continuing. Must be the last step in its list.")]).describe("One flow step. The `type` field selects the step kind: send, sendTemplate, sendProducts, sendViewCatalog, wait, branch, action, pipeline, startFlow, addNote or goto."));

// value=>countMessageCharacters(value)<=MAX_SPEC_TEXT_LENGTH
fn flow_spec_schema_steps_rec_0_text_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>countMessageCharacters(value)<=BUTTON_LABEL_MAX
fn flow_spec_schema_steps_rec_0_buttons_text_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>containsVariablePlaceholder(value)?value.length<=PHONE_NUMBER_EXPRESSION_MAX:isE164PhoneNumber(value)
fn is_valid_authored_phone_number(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>containsVariablePlaceholder(value)?value.length<=COPY_TEXT_EXPRESSION_MAX:countMessageCharacters(value)<=COPY_TEXT_MAX
fn is_valid_authored_copy_text(value: &Value) -> bool {
    todo!("P3.4")
}

// button=>button.call===void 0||!button.then?.length
fn flow_spec_schema_steps_rec_0_buttons_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// button=>button.copy===void 0||!button.then?.length
fn flow_spec_schema_steps_rec_0_buttons_refine_2(value: &Value) -> bool {
    todo!("P3.4")
}

// button=>button.copy===void 0||button.call===void 0
fn flow_spec_schema_steps_rec_0_buttons_refine_3(value: &Value) -> bool {
    todo!("P3.4")
}

// (data,ctx)=>{const kinds=[data.text,data.imageUrl,data.fileUrl].filter(value=>value!==void 0);if(kinds.length!==1){ctx.addIssue({code:"custom",message:"Exactly one of text, imageUrl, or fileUrl is required.",path:[]})}}
fn flow_spec_schema_steps_rec_0_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// value=>value.length<=SEND_VIEW_CATALOG_TEXT_MAX
fn flow_spec_schema_steps_rec_3_text_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>value.length<=SEND_VIEW_CATALOG_FOOTER_MAX
fn flow_spec_schema_steps_rec_3_footer_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// (data,ctx)=>{if((data.action==="addTags"||data.action==="removeTags")&&(!data.tagNames||data.tagNames.length===0)){ctx.addIssue({code:"custom",message:`action "${data.action}" requires a non-empty tagNames`,path:["tagNames"]})}if(data.action==="setCustomField"){if(!data.customFieldName){ctx.addIssue({code:"custom",message:'action "setCustomField" requires customFieldName',path:["customFieldName"]})}const operation=data.operation?fieldOperationByApiName(data.operation):void 0;const valueMissing=!operation||operation===FieldOperationType.set?data.value===void 0:isOperandBlank(operation,data.value);if(valueMissing){ctx.addIssue({code:"custom",message:'action "setCustomField" requires value',path:["value"]})}}}
fn flow_spec_schema_steps_rec_6_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// (data,ctx)=>{const rules=PIPELINE_ACTION_FIELDS[data.action];const allowed=new Set([...rules.required,...rules.optional]);for(const key of PIPELINE_ACTION_SPECIFIC_FIELDS){if(data[key]!==void 0&&!allowed.has(key)){ctx.addIssue({code:"custom",message:`action "${data.action}" does not accept ${key}`,path:[key]})}}for(const key of rules.required){const value=data[key];const isEmptyList=Array.isArray(value)&&value.length===0;if(value===void 0||isEmptyList){ctx.addIssue({code:"custom",message:`action "${data.action}" requires ${key}${isEmptyList?" to be non-empty":""}`,path:[key]})}}const oneOf="requireOneOf"in rules?rules.requireOneOf:[];const hasValue=__name(key=>{const value=data[key];return Array.isArray(value)?value.length>0:value!==void 0},"hasValue");if(oneOf.length>0&&!oneOf.some(hasValue)){ctx.addIssue({code:"custom",message:`action "${data.action}" requires at least one of ${oneOf.join(", ")}`,path:[oneOf[0]??"action"]})}if(data.onSkip!==void 0&&!pipelineActionCanSkip(data.action)){ctx.addIssue({code:"custom",message:`action "${data.action}" never skips, so onSkip would never run`,path:["onSkip"]})}}
fn flow_spec_schema_steps_rec_7_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// value=>countMessageCharacters(value)<=MAX_SPEC_TEXT_LENGTH
fn flow_spec_schema_steps_rec_9_note_refine(value: &Value) -> bool {
    todo!("P3.4")
}
