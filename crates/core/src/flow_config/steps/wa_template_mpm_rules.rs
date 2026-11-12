//! Port of `packages/flow-config/src/steps/wa-template-mpm-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn wa_template_mpm_limits() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"maxSections":10,"maxProductsTotal":30}"#).expect("valid JSON")
    });
    &V
}

// HAND-PORT countMpmProducts:
// function countMpmProducts(sections){return(sections??[]).reduce((total,section)=>total+(section.product_items??[]).filter(item=>isConfiguredRetailerId(item.product_retailer_id)).length,0)}

// HAND-PORT validateMpmParams:
// function validateMpmParams(params,ctx,basePath=[]){for(const target of collectMpmTargets(params,basePath)){const sections=target.button.sections??[];const productCount=countMpmProducts(sections);if(productCount===0){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplateMpmNoProducts,path:target.path});continue}if(sections.some(sectionHasBlankOrNoProducts)){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplateMpmIncompleteProducts,path:target.path})}if(sections.length>waTemplateMpmLimits.maxSections){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplateMpmTooManySections,path:target.path})}if(productCount>waTemplateMpmLimits.maxProductsTotal){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplateMpmTooManyProducts,path:target.path})}}}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn count_mpm_products(sections: Option<&Value>) -> usize {
    todo!("P3.4")
}
