//! Port of `packages/flow-config/src/channel-rules/product-message-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const PRODUCT_MESSAGE_CONDITIONS: &[&str] =
    &["always", "never", "multipleProducts", "multipleSections"];

// HAND-PORT productMessageConditionHolds (object): {"always":"<fn always>","never":"<fn never>","multipleProducts":"<fn multipleProducts>","multipleSections":"<fn multipleSections>"}

// HAND-PORT evaluateProductMessage:
// (capability,shape)=>{const rule=capability.displays[shape.display];if(!rule){return[{code:flowValidationCodes.sendProductsDisplayUnsupported,path:["display"]}]}const context={productCount:shape.sections.reduce((total,section)=>total+section.productCount,0),sectionCount:shape.sections.length};const issues=[];if(context.productCount<rule.minProducts||context.productCount>rule.maxProducts){issues.push({code:flowValidationCodes.sendProductsProductCountOutOfRange,path:["sections"]})}if(context.sectionCount>rule.maxSections){issues.push({code:flowValidationCodes.sendProductsSectionCountOutOfRange,path:["sections"]})}for(const field of textFields){issues.push(...evaluateText({value:shape[field],rule:rule[field],context,path:[field]}))}shape.sections.forEach((section,sectionIndex)=>{issues.push(...evaluateText({value:section.title,rule:rule.sectionTitle,context,path:["sections",sectionIndex,"title"]}))});return issues}

// HAND-PORT toProductMessageShape:
// content=>({display:content.display,header:content.header,body:content.body,footer:content.footer,sections:content.sections.map(section=>({title:section.title,productCount:section.productIds.length}))})

// HAND-PORT refineProductMessage:
// capability=>(content,ctx)=>{for(const issue of evaluateProductMessage(capability,toProductMessageShape(content))){ctx.addIssue({code:"custom",path:[...issue.path],message:issue.code})}}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn evaluate_product_message(capability: &Value, shape: &Value) -> Vec<Value> {
    todo!("P3.4")
}
