//! Port of `packages/flow-config/src/steps/template-start-step.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn template_step_type_aliases() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(
            r#"{"sendWaTemplateMessage":["WA_TM01"],"sendMessengerTemplateMessage":[]}"#,
        )
        .expect("valid JSON")
    });
    &V
}

// HAND-PORT resolveStepTypeMatches:
// stepType=>[stepType,...TEMPLATE_STEP_TYPE_ALIASES[stepType]??[]]

// HAND-PORT findTemplateStartStep:
// function findTemplateStartStep(nodes,stepType){if(!Array.isArray(nodes)){return null}const matches=resolveStepTypeMatches(stepType);for(const node of nodes.filter(isStartNode)){for(const step of nodeSteps(node)){const candidate=step;const candidateType=asString(candidate.stepType);if(!(candidateType&&matches.includes(candidateType))){continue}const templateId=asString(candidate.template?.id);if(templateId){return{stepType:candidateType,templateId}}}}return null}
