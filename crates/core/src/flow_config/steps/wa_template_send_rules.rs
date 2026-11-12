//! Port of `packages/flow-config/src/steps/wa-template-send-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT validateWaTemplateSendParams:
// function validateWaTemplateSendParams(params,ctx,basePath=[]){validateMpmParams(params,ctx,basePath);validateLimitedTimeOfferParams(params,ctx,basePath);validatePaymentRequestParams(params,ctx,basePath)}

// HAND-PORT refineWaTemplateSendStep:
// (step,ctx)=>validateWaTemplateSendParams(step.template.params,ctx,["template","params"])
