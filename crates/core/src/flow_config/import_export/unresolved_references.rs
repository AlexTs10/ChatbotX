// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/import-export/unresolved-references.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT removeUnresolvedReferencesDetailed:
// (value,references,mode="repair")=>{const result={value,removed:[],droppedElements:[],blocked:[]};if(references.length===0){return result}const root=structuredClone(value);const ordered=references.map(reference=>({path:reference.path,segments:parsePath(reference.path)})).sort((a,b)=>comparePathsDescending(a.segments,b.segments));const dropped=new Set;for(const{path,segments}of ordered){if(startsWithDropped(segments,dropped)){continue}const outcome=removeAt(root,segments,mode);if(outcome.kind==="removed"){result.removed.push(path)}else if(outcome.kind==="droppedElement"){dropped.add(pathKey(outcome.elementPath));result.droppedElements.push(path)}else if(outcome.kind==="blocked"){result.blocked.push(path)}}result.value=root;return result}

// HAND-PORT removeUnresolvedReferences:
// (value,references)=>removeUnresolvedReferencesDetailed(value,references,"clear").value

/// `RemoveUnresolvedResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoveUnresolvedResult {
    pub value: Value,
    /// Paths whose slot was cleared or removed in place.
    pub removed: Vec<String>,
    /// Paths of array elements dropped because a required slot inside them was unresolved.
    pub dropped_elements: Vec<String>,
    /// Paths of required slots outside any droppable element: the flow cannot be kept.
    pub blocked: Vec<String>,
}

/// `removeUnresolvedReferencesDetailed(value, references)` (repair mode): each
/// reported reference removed the way its slot allows. Never mutates the input.
pub fn remove_unresolved_references_detailed(
    value: &Value,
    references: &[&str],
) -> RemoveUnresolvedResult {
    todo!("P3.4b")
}

/// `removeUnresolvedReferences(value, references)` (clear mode).
pub fn remove_unresolved_references(value: &Value, references: &[&str]) -> Value {
    todo!("P3.4b")
}
