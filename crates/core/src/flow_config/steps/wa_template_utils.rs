//! Port of `packages/flow-config/src/steps/wa-template-utils.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT findSendWaTemplateStep:
// (nodes,stepId)=>findStepInNodes(nodes,step=>isSendWaTemplateStep(step)&&hasStepId(step)&&step.id===stepId)

pub const WA_TEMPLATE_STATUS_BUTTON_COUNT: usize = 2;

// HAND-PORT extractTemplateQuickReplyButtons:
// function extractTemplateQuickReplyButtons(components){const buttonsComponent=components?.find(component=>component.type==="BUTTONS"&&component.buttons);return(buttonsComponent?.buttons??[]).flatMap((button,index)=>button.type.toUpperCase()===QUICK_REPLY_BUTTON_TYPE?[{templateButtonIndex:index,text:button.text}]:[])}

// HAND-PORT splitWaTemplateStepButtons:
// function splitWaTemplateStepButtons(buttons){return{statusButtons:buttons.slice(0,WA_TEMPLATE_STATUS_BUTTON_COUNT),quickReplyButtons:buttons.slice(WA_TEMPLATE_STATUS_BUTTON_COUNT)}}

// HAND-PORT seedWaTemplateStepButtons:
// function seedWaTemplateStepButtons(existingButtons,components){const{statusButtons,quickReplyButtons:previousQuickReplies}=splitWaTemplateStepButtons(existingButtons);const templateButtons=extractTemplateQuickReplyButtons(components).map(quickReply=>buttonStepDefaultFn({label:quickReply.text.slice(0,BUTTON_LABEL_MAX)}));return[...statusButtons,...mergeTemplateButtonsWithExisting(templateButtons,previousQuickReplies)]}

// HAND-PORT bindWaTemplateQuickReplyButtons:
// function bindWaTemplateQuickReplyButtons(components,stepButtons){const{quickReplyButtons}=splitWaTemplateStepButtons(stepButtons);if(quickReplyButtons.length===0){return[]}return extractTemplateQuickReplyButtons(components).flatMap((quickReply,position)=>{const stepButton=quickReplyButtons[position];return stepButton?[{templateButtonIndex:quickReply.templateButtonIndex,stepButton}]:[]})}

// HAND-PORT extractParameterInfos:
// function extractParameterInfos(components){const params=[];if(!components||components.length===0){return params}for(const component of components){if(component.type==="HEADER"){if(component.format==="TEXT"&&component.text){const matches=component.text.match(/\{\{(\d+|[a-zA-Z_]+)\}\}/g);if(matches){for(const[idx,match]of matches.entries()){const paramName=match.replace(/\{\{|\}\}/g,"");params.push({type:"header",index:idx,paramName,format:"text"})}}}else if(component.format==="LOCATION"){params.push({type:"header",index:0,paramName:"location",format:"location"})}else if(["IMAGE","VIDEO","DOCUMENT"].includes(component.format||"")){params.push({type:"header",index:0,paramName:"1",format:component.format?.toLowerCase()})}}else if(component.type==="BODY"&&component.text){const matches=component.text.match(/\{\{(\d+|[a-zA-Z_]+)\}\}/g);if(matches){for(const[idx,match]of matches.entries()){const paramName=match.replace(/\{\{|\}\}/g,"");params.push({type:"body",index:idx,paramName})}}}else if(component.type==="BUTTONS"&&component.buttons){params.push(...extractButtonParameterInfos(component.buttons))}else if(component.type==="CAROUSEL"&&component.cards){params.push(...extractCarouselParameterInfos(component.cards))}else if(component.type==="LIMITED_TIME_OFFER"&&component.limited_time_offer?.has_expiration){params.push({type:"limited_time_offer",index:0,paramName:"expiration_time_ms",format:"limited_time_offer"})}}return params}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn bind_wa_template_quick_reply_buttons(
    components: &[Value],
    step_buttons: &[Value],
) -> Vec<Value> {
    todo!("P3.4")
}

pub fn extract_parameter_infos(components: &[Value]) -> Vec<Value> {
    todo!("P3.4")
}

pub fn extract_template_quick_reply_buttons(components: &[Value]) -> Vec<Value> {
    todo!("P3.4")
}

pub fn seed_wa_template_step_buttons(existing: &[Value], components: &[Value]) -> Vec<Value> {
    todo!("P3.4")
}
