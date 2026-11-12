//! Port of `packages/flow-config/src/steps/wa-template-payment-request-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn wa_payment_setting_type_by_method() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"pix":"pix_dynamic_code","boleto":"boleto","payment_link":"payment_link","card_on_file":"offsite_card_pay"}"#).expect("valid JSON")
    });
    &V
}

// HAND-PORT resolveWaPaymentRequestMethod:
// function resolveWaPaymentRequestMethod(button){const settingType=button.payment_setting?.type;if(settingType===void 0){return findMethodByLabel(button.text)}const method=findMethodBySettingType(settingType);if(method===null){return null}const labelMethod=findMethodByLabel(button.text);return labelMethod===null||labelMethod===method?method:null}

// HAND-PORT validatePaymentRequestParams:
// function validatePaymentRequestParams(params,ctx,basePath=[]){for(const[position,button]of(params.button??[]).entries()){if(!button?.payment_request){continue}const issues=collectPaymentRequestIssues(button.payment_request,{allowVariables:true});for(const issue of issues){ctx.addIssue({code:"custom",message:issueCodeByKind[issue.kind],path:[...basePath,"button",position,"payment_request",issue.field]})}}}

// HAND-PORT findTemplatePaymentButtons:
// function findTemplatePaymentButtons(components){return components.filter(component=>component.type==="BUTTONS").flatMap(component=>component.buttons??[]).flatMap((button,index)=>button.type.toUpperCase()==="PAYMENT_REQUEST"?[{index,method:resolveWaPaymentRequestMethod(button)}]:[])}

// HAND-PORT findPaymentRequestMismatches:
// function findPaymentRequestMismatches(params,components){const templateButtons=findTemplatePaymentButtons(components);const sent=(params.button??[]).filter(isPaymentRequestParam);const unmatchedButtons=templateButtons.flatMap(templateButton=>{const reason=mismatchReasonOf(templateButton,sent.filter(entry=>entry.index===templateButton.index));return reason?[{index:templateButton.index,reason}]:[]});const templateIndexes=new Set(templateButtons.map(button=>button.index));const strayEntries=sent.filter(entry=>!templateIndexes.has(entry.index)).map(entry=>({index:entry.index,reason:"extra"}));return[...unmatchedButtons,...strayEntries]}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn find_payment_request_mismatches(params: &Value, components: &[Value]) -> Vec<Value> {
    todo!("P3.4")
}

pub fn resolve_wa_payment_request_method(button: &Value) -> Option<&'static str> {
    todo!("P3.4")
}
