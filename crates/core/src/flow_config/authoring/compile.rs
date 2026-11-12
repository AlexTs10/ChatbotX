// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/authoring/compile.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT compileFlowSpec:
// function compileFlowSpec(spec,ctx){const state=createCompileState(spec,ctx);validateStructure(spec,state);const startNodeId=compileChain(spec.steps,"steps",state);if(state.errors.length>0){throw new FlowAuthoringException(state.errors)}return finalizeGraph(state,startNodeId)}
