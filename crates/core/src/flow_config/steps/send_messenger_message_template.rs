//! Port of `packages/flow-config/src/steps/send-messenger-message-template.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn messenger_template_button_param_schema = z::object(vec![("sub_type", z::enum_(&["url", "phone_number"]).describe("Button kind: `url` or `phone_number`.")),
("index", z::number().optional().describe("Zero-based position of the button in the template.")),
("text", z::string().optional().describe("Variable value for a `url` button (the dynamic URL suffix). For a Mini App or Minigame link, leave it blank to send each recipient's own token (`{{mini_app_token}}` / `{{minigame_play_token}}`); a fixed value would not identify the contact.")),
("payload", z::string().optional().describe("Legacy; ignored by the sender. POSTBACK payloads come from the step's `buttons` configuration, so configure buttons there."))]));

schema!(pub fn messenger_template_params_schema = z::object(vec![("header", z::array(z::object(vec![("type", z::enum_(&["text", "image"]).describe("Header parameter kind; fill the matching field.")),
("text", z::string().optional().describe("Value for a `text` header variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates.")),
("image", z::object(vec![("link", z::string().describe("Public https URL of the image."))]).optional().describe("Image for an `image` header."))])).optional().describe("Header parameters; one entry for a templated header.")),
("body", z::array(z::object(vec![("text", z::string().describe("Value that replaces the variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates."))])).optional().describe("Body variables in order: {{1}}, {{2}}, ...")),
("button", z::array(messenger_template_button_param_schema()).optional().describe("Parameters for the template's dynamic buttons, by `index`."))]));

schema!(pub fn send_messenger_template_message_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendMessengerTemplateMessage")).describe("Step type discriminator: \"sendMessengerTemplateMessage\".")),
("template", z::object(vec![("id", z::string().trim().min(1)),
("name", z::string()),
("language", z::string()),
("parameterFormat", z::enum_(&["POSITIONAL", "NAMED"]).default(json!("POSITIONAL"))),
("params", messenger_template_params_schema()),
("inboxId", z::string().optional()),
("integrationMessengerId", z::string().optional())])),
("buttons", z::array(button_step_schema()).super_refine(refine_reply_only_buttons).default(json!([])))]));

// HAND-PORT sendMessengerTemplateMessageStepDefaultFn:
// (props={})=>{const{template:templateProps,...restProps}=props;return{template:{id:"",name:"",language:"",parameterFormat:"POSITIONAL",params:{},...templateProps},buttons:[],...restProps,id:createId(),stepType:stepTypes.enum.sendMessengerTemplateMessage}}

// HAND-PORT findMessengerTokenVariable:
// function findMessengerTokenVariable(text){const match=text?.match(TOKEN_VARIABLE_PATTERN);return match?.[1]??null}

// HAND-PORT normalizeMessengerTokenUrl:
// function normalizeMessengerTokenUrl(url){const match=TOKEN_URL_SUFFIXES.find(entry=>entry.pastedPattern.test(url));if(!match){return null}return{url:url.replace(TRAILING_PLACEHOLDER_PATTERN,"{{1}}"),variable:match.variable}}

// HAND-PORT readMessengerMiniAppButtons:
// function readMessengerMiniAppButtons(components){const miniAppButtons=[];for(const component of components??[]){if(component.type!=="BUTTONS"||!component.buttons){continue}for(const[index,button]of component.buttons.entries()){const url=button.url??"";const miniAppId=url.match(MINI_APP_ID_PATTERN)?.[1];if(button.type.toUpperCase()==="URL"&&miniAppId&&MINI_APP_URL_PATTERN.test(url)){miniAppButtons.push({index,label:button.text??"",miniAppId})}}}return miniAppButtons}

// HAND-PORT suggestMessengerUrlSuffixVariable:
// function suggestMessengerUrlSuffixVariable(url){const match=TOKEN_URL_SUFFIXES.find(entry=>entry.savedPattern.test(url??""));return match?`{{${match.variable}}}`:""}

// HAND-PORT extractMessengerTemplateParams:
// function extractMessengerTemplateParams(components,parameterFormat){const params={};if(!components||components.length===0){return params}for(const component of components){if(component.type==="HEADER"){const names=extractPlaceholderNames(component.text);if(names.length>0){params.header=names.map(name=>({type:"text",text:"",...toParameterName(name,parameterFormat)}))}}else if(component.type==="BODY"){const names=extractPlaceholderNames(component.text);if(names.length>0){params.body=names.map(name=>({text:"",...toParameterName(name,parameterFormat)}))}}else if(component.type==="BUTTONS"&&component.buttons){const buttonParams=[];for(const[idx,button]of component.buttons.entries()){const buttonType=button.type.toUpperCase();if(buttonType==="URL"&&button.url){buttonParams.push({sub_type:"url",index:idx,text:hasPlaceholder(button.url)?suggestMessengerUrlSuffixVariable(button.url):button.url})}if(buttonType==="PHONE_NUMBER"){buttonParams.push({sub_type:"phone_number",index:idx})}}if(buttonParams.length>0){params.button=buttonParams}}}return params}

// HAND-PORT extractMessengerParameterInfos:
// function extractMessengerParameterInfos(components,parameterFormat){const params=[];if(!components||components.length===0){return params}const toDisplayName=__name((name,idx)=>parameterFormat==="NAMED"?name:String(idx+1),"toDisplayName");for(const component of components){if(component.type==="HEADER"){for(const[idx,name]of extractPlaceholderNames(component.text).entries()){params.push({type:"header",index:idx,paramName:toDisplayName(name,idx),format:"text"})}}else if(component.type==="BODY"){for(const[idx,name]of extractPlaceholderNames(component.text).entries()){params.push({type:"body",index:idx,paramName:toDisplayName(name,idx)})}}else if(component.type==="BUTTONS"&&component.buttons){for(const[buttonIdx,button]of component.buttons.entries()){const buttonType=button.type.toUpperCase();const[urlSuffixName]=extractPlaceholderNames(button.url);if(buttonType==="URL"&&urlSuffixName){params.push({type:"button",index:0,paramName:toDisplayName(urlSuffixName,0),buttonIndex:buttonIdx,buttonSubType:"url"})}}}}return params}

// HAND-PORT extractMessengerFlowButtons:
// function extractMessengerFlowButtons(components,options={}){const buttons=[];if(!components||components.length===0){return buttons}const miniAppButtonsByIndex=new Map(options.includeMiniApp?readMessengerMiniAppButtons(components).map(button=>[button.index,button]):[]);for(const component of components){if(component.type==="BUTTONS"&&component.buttons){for(const[index,button]of component.buttons.entries()){const buttonType=button.type.toUpperCase();const miniAppButton=miniAppButtonsByIndex.get(index);if(miniAppButton){buttons.push({...buttonStepDefaultFn({label:miniAppButton.label}),buttonType:buttonTypes.enum.openMiniApp,beforeStep:openMiniAppStepDefaultFn({miniAppId:miniAppButton.miniAppId})})}else if((buttonType==="POSTBACK"||buttonType==="QUICK_REPLY")&&button.payload?.includes("{{")){buttons.push(buttonStepDefaultFn({label:button.text??""}))}}}}return buttons}

// HAND-PORT mergeMessengerFlowButtonsWithExisting:
// function mergeMessengerFlowButtonsWithExisting(templateButtons,existingButtons=[]){const merged=mergeTemplateButtonsWithExisting(templateButtons,existingButtons);return merged.map((button,index)=>{const templateButton=templateButtons[index];if(!templateButton||button===templateButton){return button}if(isOpenMiniApp(button)!==isOpenMiniApp(templateButton)){return templateButton}if(button.buttonType===buttonTypes.enum.openMiniApp&&templateButton.buttonType===buttonTypes.enum.openMiniApp&&button.beforeStep.miniAppId!==templateButton.beforeStep.miniAppId){return{...button,beforeStep:{...button.beforeStep,miniAppId:templateButton.beforeStep.miniAppId}}}return button})}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_messenger_template_message_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}

pub fn extract_messenger_flow_buttons(components: &[Value], include_mini_app: bool) -> Vec<Value> {
    todo!("P3.4")
}

pub fn merge_messenger_flow_buttons_with_existing(
    template_buttons: &[Value],
    existing: &[Value],
) -> Vec<Value> {
    todo!("P3.4")
}

pub fn read_messenger_mini_app_buttons(components: &[Value]) -> Vec<Value> {
    todo!("P3.4")
}
