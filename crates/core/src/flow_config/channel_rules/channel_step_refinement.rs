//! Port of `packages/flow-config/src/channel-rules/channel-step-refinement.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT refineStepsByChannel:
// (nodes,ctx)=>{nodes.forEach((node,nodeIndex)=>{if(node.type!==nodeTypeSchema.enum.sendMessage){return}const channel=getSendMessageChannel(node);const policy=CHANNEL_FLOW_POLICIES[channel];const quickReplyCount=node.data.details.quickReplies.length;let quickReplyOverflowReported=false;node.data.details.steps.forEach((step,stepIndex)=>{const addStepIssue=__name((message,path,capability)=>{ctx.addIssue({code:"custom",message,path:[nodeIndex,"data","details","steps",stepIndex,...path],params:capability?{capability}:void 0})},"addStepIssue");if(policy.steps[step.stepType]===stepSupport.unsupported){addStepIssue(`The ${channel} channel does not support ${step.stepType}.`,[],{block:step.stepType,channel,code:flowValidationCodes.unsupportedBlock,policyVersion:CHANNEL_POLICY_VERSION});return}const buttonCount=checkButtonCount(step,quickReplyCount,policy.quickRepliesShareButtonSlots,policy.limits.buttonCount);if(buttonCount.total>policy.limits.buttonCount){const capability={actual:buttonCount.total,allowed:policy.limits.buttonCount,block:step.stepType,channel,code:flowValidationCodes.constraintExceeded,constraintId:"maxButtonCount",policyVersion:CHANNEL_POLICY_VERSION,unit:"buttons"};const message=`${step.stepType} exceeds the ${channel} maximum of ${capability.allowed} ${capability.unit}.`;if(buttonCount.quickRepliesCauseOverflow&&!quickReplyOverflowReported){quickReplyOverflowReported=true;ctx.addIssue({code:"custom",message,params:{capability},path:[nodeIndex,"data","details","quickReplies"]})}else if(!buttonCount.quickRepliesCauseOverflow){addStepIssue(message,["buttons"],capability)}}if(channel===channelTypes.enum.tiktok&&step.stepType===stepTypes.enum.sendText&&isTiktokQuickReplyCardTitleTooLong({step,quickReplyCount})){addStepIssue(flowValidationCodes.tiktokCardTitleTooLong,["text"])}const validator=channelAwareStepValidators[step.stepType];if(!validator){return}const result=resolveStepValidator(validator,channel).safeParse(step);if(result.success){return}for(const issue of result.error.issues){addStepIssue(issue.message,issue.path)}})})}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn refine_steps_by_channel(nodes: &[Value], ctx: &mut RefineCtx) {
    todo!("P3.4")
}
