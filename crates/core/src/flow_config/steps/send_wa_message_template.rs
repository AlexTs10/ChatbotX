//! Port of `packages/flow-config/src/steps/send-wa-message-template.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn button_sub_types = z::enum_(&["url", "quick_reply", "copy_code", "flow", "catalog", "mpm", "payment_request"]));

schema!(pub fn wa_template_button_param_schema = z::object(vec![("sub_type", button_sub_types().optional().describe("Button kind: url, quick_reply, copy_code, flow, catalog, mpm or payment_request.")),
("index", z::number().optional().describe("Zero-based position of the button in the template.")),
("text", z::string().optional().describe("Variable value for a `url` button (the dynamic URL suffix).")),
("coupon_code", z::string().optional().describe("Coupon code for a `copy_code` button.")),
("payload", z::string().optional().describe("Payload returned when a `quick_reply` button is tapped.")),
("flow_token", z::string().optional().describe("Token that identifies the WhatsApp Flow session (`flow` button).")),
("flow_action_data", z::record(z::string(), z::unknown()).optional().describe("Initial data passed to the first screen of the WhatsApp Flow.")),
("flowSourceId", z::string().optional().describe("Meta id of the WhatsApp Flow, from `whatsappFlows.list`.")),
("navigateScreenId", z::string().optional().describe("Screen id the Flow opens on, from `whatsappFlows.getScreens`.")),
("fieldMappings", z::array(whatsapp_flow_field_mapping_schema()).optional().describe("Maps Flow response fields to custom fields.")),
("thumbnail_product_retailer_id", z::string().optional().describe("Retailer id of the product shown as thumbnail (`catalog` button).")),
("sections", z::array(z::object(vec![("title", z::string().optional().describe("Section title (`mpm` button).")),
("product_items", z::array(z::object(vec![("product_retailer_id", z::string().describe("Retailer id of a catalog product."))])).optional().describe("Products listed in the section."))])).optional().describe("Product sections of a multi-product (`mpm`) button.")),
("payment_request", payment_request_schema().optional().describe("Payment data of a `payment_request` button. `method` must match the template button (Pix, Boleto, payment link or saved card)."))]).super_refine(wa_template_button_param_schema_super_refine));

schema!(pub fn wa_template_carousel_card_schema = z::object(vec![("card_index", z::number().describe("Zero-based position of the card in the carousel.")),
("header", z::array(z::object(vec![("type", z::enum_(&["image", "video"]).describe("Card media kind.")),
("image", z::object(vec![("link", z::string().describe("Public https URL of the card image."))]).optional().describe("Image for an `image` card header.")),
("video", z::object(vec![("link", z::string().describe("Public https URL of the card video."))]).optional().describe("Video for a `video` card header."))])).optional().describe("Card header media.")),
("body", z::array(z::object(vec![("type", z::literal(json!("text")).optional().describe("Always `text`.")),
("text", z::string().describe("Value that replaces the card variable."))])).optional().describe("Card body variables in order.")),
("button", z::array(wa_template_button_param_schema().nullable()).transform(wa_template_carousel_card_schema_button_transform).optional().describe("Parameters for the card's dynamic buttons, by `index`."))]).super_refine(wa_template_carousel_card_schema_super_refine));

schema!(pub fn wa_template_params_schema = z::object(vec![("header", z::array(z::object(vec![("type", z::enum_(&["text", "image", "video", "document", "location"]).describe("Header parameter kind; fill the matching field.")),
("text", z::string().optional().describe("Value for a `text` header variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates ({{order_id}}).")),
("image", z::object(vec![("link", z::string().describe("Public https URL of the image."))]).optional().describe("Media for an `image` header.")),
("video", z::object(vec![("link", z::string().describe("Public https URL of the video."))]).optional().describe("Media for a `video` header.")),
("document", z::object(vec![("link", z::string().describe("Public https URL of the document."))]).optional().describe("Media for a `document` header.")),
("location", z::object(vec![("latitude", z::string().optional().describe("Latitude, e.g. `10.7769`.")),
("longitude", z::string().optional().describe("Longitude, e.g. `106.7009`.")),
("name", z::string().optional().describe("Place name shown to the customer.")),
("address", z::string().optional().describe("Street address."))]).optional().describe("Place for a `location` header."))])).optional().describe("Header parameters; one entry for a templated header.")),
("body", z::array(z::object(vec![("type", z::literal(json!("text")).optional().describe("Always `text` for a body variable.")),
("text", z::string().describe("Value that replaces the variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates; omit for positional `{{1}}`."))])).optional().describe("Body variables in order: {{1}}, {{2}}, ...")),
("button", z::array(wa_template_button_param_schema().nullable()).transform(wa_template_carousel_card_schema_button_transform).optional().describe("Parameters for the template's dynamic buttons, by `index`.")),
("carousel", z::array(wa_template_carousel_card_schema()).optional().describe("Cards of a carousel template, by `card_index`.")),
("limited_time_offer", z::object(vec![("expiration_time_ms", z::number().describe("Offer expiry as a Unix timestamp in milliseconds."))]).optional().describe("Only for a limited-time-offer template."))]));

// HAND-PORT toOptionalString:
// value=>value===null||value===void 0?void 0:String(value)

// HAND-PORT isNamedTemplateToken:
// function isNamedTemplateToken(token){return!POSITIONAL_TOKEN_REGEX.test(token)}

// HAND-PORT extractTemplateParams:
// function extractTemplateParams(components){const params={};if(!components||components.length===0){return params}for(const component of components){if(component.type==="HEADER"){if(component.format==="TEXT"&&component.text){const headerParams=extractTextParams(component.text);if(headerParams.length>0){params.header=headerParams}}else if(component.format==="LOCATION"){params.header=[{type:"location",location:{latitude:"",longitude:"",name:"",address:""}}]}else if(["IMAGE","VIDEO","DOCUMENT"].includes(component.format||"")){const format=component.format?.toLowerCase();params.header=[{type:format,[format]:{link:""}}]}}else if(component.type==="BODY"&&component.text){const bodyParams=extractTextParams(component.text);if(bodyParams.length>0){params.body=bodyParams}}else if(component.type==="BUTTONS"&&component.buttons){const buttonParams=extractButtonParams(component.buttons,{allowPaymentRequest:true});if(buttonParams.length>0){params.button=buttonParams}}else if(component.type==="CAROUSEL"&&component.cards){params.carousel=extractCarouselParams(component.cards)}else if(component.type==="LIMITED_TIME_OFFER"&&component.limited_time_offer?.has_expiration){params.limited_time_offer={expiration_time_ms:0}}}return params}

pub const WA_TEMPLATE_STATUS_BRANCHES: &[&str] = &["delivered", "failed"];

pub fn wa_template_status_button_labels() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"delivered":"Delivered","failed":"Failed"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn send_wa_template_message_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendWaTemplateMessage"))),
("template", z::object(vec![("id", z::string().trim().min(1)),
("name", z::string()),
("language", z::string()),
("inboxId", z::string().regex(r#"^\d+$"#, "").nullable().optional()),
("params", wa_template_params_schema())])),
("buttons", z::array(button_step_schema()).super_refine(refine_no_open_mini_app_buttons).super_refine(refine_reply_only_buttons).default(json!([])).transform(send_wa_template_message_step_schema_buttons_transform))]));

// HAND-PORT sendWaTemplateMessageStepDefaultFn:
// (props={})=>{const{template:templateProps,...restProps}=props;return{template:{id:"",name:"",language:"",inboxId:null,params:{},...templateProps},buttons:[buttonStepDefaultFn({label:WA_TEMPLATE_STATUS_BUTTON_LABELS.delivered}),buttonStepDefaultFn({label:WA_TEMPLATE_STATUS_BUTTON_LABELS.failed})],...restProps,id:createId(),stepType:stepTypes.enum.sendWaTemplateMessage}}

// (button,ctx)=>{const isPaymentButton=button.sub_type==="payment_request";if(isPaymentButton!==(button.payment_request!==void 0)){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplatePaymentRequestMisplaced,path:["payment_request"]})}if(isPaymentButton&&button.index===void 0){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplatePaymentRequestMisplaced,path:["index"]})}}
fn wa_template_button_param_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// buttons=>buttons.filter(button=>button!==null)
fn wa_template_carousel_card_schema_button_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// (card,ctx)=>{if(card.button?.some(button=>button.payment_request!==void 0)){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplatePaymentRequestMisplaced,path:["button"]})}}
fn wa_template_carousel_card_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// buttons=>{if(buttons.length===0){return[buttonStepDefaultFn({label:WA_TEMPLATE_STATUS_BUTTON_LABELS.delivered}),buttonStepDefaultFn({label:WA_TEMPLATE_STATUS_BUTTON_LABELS.failed})]}return buttons}
fn send_wa_template_message_step_schema_buttons_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_wa_template_message_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}

pub fn extract_template_params(components: &[Value]) -> Value {
    todo!("P3.4")
}

pub fn is_named_template_token(token: &str) -> bool {
    todo!("P3.4")
}
