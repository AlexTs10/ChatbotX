//! Port of `packages/flow-config/src/import-export/unresolved-references.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT removeUnresolvedReferencesDetailed:
// (value,references,mode="repair")=>{const result={value,removed:[],droppedElements:[],blocked:[]};if(references.length===0){return result}const root=structuredClone(value);const ordered=references.map(reference=>({path:reference.path,segments:parsePath(reference.path)})).sort((a,b)=>comparePathsDescending(a.segments,b.segments));const dropped=new Set;for(const{path,segments}of ordered){if(startsWithDropped(segments,dropped)){continue}const outcome=removeAt(root,segments,mode);if(outcome.kind==="removed"){result.removed.push(path)}else if(outcome.kind==="droppedElement"){dropped.add(pathKey(outcome.elementPath));result.droppedElements.push(path)}else if(outcome.kind==="blocked"){result.blocked.push(path)}}result.value=root;return result}

// HAND-PORT removeUnresolvedReferences:
// (value,references)=>removeUnresolvedReferencesDetailed(value,references,"clear").value
