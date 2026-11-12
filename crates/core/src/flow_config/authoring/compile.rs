// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/authoring/compile.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT compileFlowSpec:
// function compileFlowSpec(spec,ctx){const state=createCompileState(spec,ctx);validateStructure(spec,state);const startNodeId=compileChain(spec.steps,"steps",state);if(state.errors.length>0){throw new FlowAuthoringException(state.errors)}return finalizeGraph(state,startNodeId)}

use std::collections::BTreeMap;

/// A template the compiler resolves by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringTemplate {
    pub id: String,
    pub language: String,
    pub status: String,
}

/// A custom field the compiler resolves by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringCustomField {
    pub id: String,
    pub field_type: String,
}

/// A flow the compiler resolves by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringFlowRef {
    pub id: String,
}

/// A pipeline stage (`FlowAuthoringPipeline.stages[]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStageRef {
    pub id: String,
    pub name: String,
}

/// A pipeline field (`FlowAuthoringPipeline.fields[]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineFieldRef {
    pub id: String,
    pub name: String,
    pub reference_name: String,
}

/// `FlowAuthoringPipeline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowAuthoringPipeline {
    pub id: String,
    pub name: String,
    pub reference_name: String,
    pub stages: Vec<PipelineStageRef>,
    pub fields: Vec<PipelineFieldRef>,
}

/// `FlowAuthoringContext`: reference data the compiler resolves DSL names against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowAuthoringContext {
    pub templates_by_name: BTreeMap<String, AuthoringTemplate>,
    pub custom_fields_by_name: BTreeMap<String, AuthoringCustomField>,
    pub flows_by_name: BTreeMap<String, AuthoringFlowRef>,
    /// Keyed by pipeline `referenceName`. `None` means the workspace has none.
    pub pipelines_by_reference_name: Option<BTreeMap<String, FlowAuthoringPipeline>>,
}

/// `CompiledFlow`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledFlow {
    pub start_node_id: String,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    /// Compiled node id -> the spec-relative path (e.g. `steps[2]`) that produced it.
    pub spec_path_by_node_id: BTreeMap<String, String>,
}

/// `compileFlowSpec(spec, ctx)`; `Err` is the thrown `FlowAuthoringException`.
pub fn compile_flow_spec(
    spec: &Value,
    ctx: &FlowAuthoringContext,
) -> Result<CompiledFlow, FlowAuthoringException> {
    todo!("P3.4b")
}
