//! Shared helpers for the `packages/flow-config/__tests__/authoring` compiler test ports.
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;

use chatbotx_core::flow_config::z;
pub use chatbotx_core::flow_config::{
    self as fc, AuthoringCustomField, AuthoringFlowRef, AuthoringTemplate, CompiledFlow,
    FlowAuthoringContext, FlowAuthoringError, FlowAuthoringPipeline, PipelineFieldRef,
    PipelineStageRef,
};
use serde_json::Value;

pub fn template(id: &str, language: &str, status: &str) -> AuthoringTemplate {
    AuthoringTemplate {
        id: id.to_string(),
        language: language.to_string(),
        status: status.to_string(),
    }
}

pub fn custom_field(id: &str, field_type: &str) -> AuthoringCustomField {
    AuthoringCustomField {
        id: id.to_string(),
        field_type: field_type.to_string(),
    }
}

pub fn flow_ref(id: &str) -> AuthoringFlowRef {
    AuthoringFlowRef { id: id.to_string() }
}

/// `new Map([[name, value], …])`.
pub fn map_of<T>(entries: Vec<(&str, T)>) -> BTreeMap<String, T> {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

/// `z.array(flowVersionSchema).superRefine(refineStepsByChannel).safeParse(nodes)`.
pub fn parse_publish_nodes(nodes: &[Value]) -> z::SafeParse {
    z::array(fc::flow_version_schema())
        .super_refine(|value, ctx| {
            fc::refine_steps_by_channel(value.as_array().map(Vec::as_slice).unwrap_or(&[]), ctx)
        })
        .safe_parse(&Value::Array(nodes.to_vec()))
}

/// Asserts the compiled graph is schema-valid exactly as publish requires.
pub fn expect_publishable(nodes: &[Value], edges: &[Value]) {
    let nodes_result = parse_publish_nodes(nodes);
    assert!(
        nodes_result.success,
        "{:?}",
        nodes_result.error.map(|e| e.issues)
    );
    let edges_result = z::array(fc::edge_schema()).safe_parse(&Value::Array(edges.to_vec()));
    assert!(
        edges_result.success,
        "{:?}",
        edges_result.error.map(|e| e.issues)
    );
}

/// `try { compileFlowSpec(spec, ctx) } catch (error) { … error.errors }`; fails when it compiles.
pub fn compile_errors(spec: &Value, ctx: &FlowAuthoringContext) -> Vec<FlowAuthoringError> {
    match fc::compile_flow_spec(spec, ctx) {
        Ok(_) => panic!("expected compileFlowSpec to throw"),
        Err(exception) => exception.errors,
    }
}

/// `compileFlowSpec(spec, ctx)`; fails on a `FlowAuthoringException`.
pub fn compile_ok(spec: &Value, ctx: &FlowAuthoringContext) -> CompiledFlow {
    match fc::compile_flow_spec(spec, ctx) {
        Ok(compiled) => compiled,
        Err(exception) => panic!("compileFlowSpec threw: {}", exception.message()),
    }
}

/// The first edge matching `predicate`.
pub fn find_edge(edges: &[Value], predicate: impl Fn(&Value) -> bool) -> Option<&Value> {
    edges.iter().find(|edge| predicate(edge))
}
