//! Port of `packages/flow-config/src/steps/whatsapp-flow-utils.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT findWhatsappFlowStepByButtonId:
// (nodes,buttonId)=>{for(const node of nodes){const details=node.data.details;if("steps"in details&&Array.isArray(details.steps)){const found=findWhatsappFlowStepInList(details.steps,buttonId);if(found){return found}}if("beforeStep"in details){const found=getWhatsappFlowStepByButtonId(details.beforeStep,buttonId);if(found){return found}}}return null}
