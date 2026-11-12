//! Port of `packages/flow-config/src/template-structure.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT buildTemplateStructure:
// function buildTemplateStructure(source){return{name:source.name,language:source.language,category:source.category.toUpperCase(),parameterFormat:(source.parameterFormat??DEFAULT_PARAMETER_FORMAT).toUpperCase(),components:asRecordArray(source.components).map(normalizeComponent)}}

// HAND-PORT templateStructureKey:
// function templateStructureKey(source){return canonicalJson(buildTemplateStructure(source))}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn build_template_structure(template: &Value) -> Value {
    todo!("P3.4")
}

pub fn template_structure_key(template: &Value) -> String {
    todo!("P3.4")
}
