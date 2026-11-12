//! Port of `packages/flow-config/src/import-export/json-walk.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT forEachStringLeaf:
// (value,visit,depth=0)=>{if(depth>MAX_WALK_DEPTH){return}if(typeof value==="string"){visit(value);return}if(Array.isArray(value)){for(const item of value){forEachStringLeaf(item,visit,depth+1)}return}if(typeof value==="object"&&value!==null){for(const child of Object.values(value)){forEachStringLeaf(child,visit,depth+1)}}}

// HAND-PORT mapStringLeaves:
// (value,rewrite,depth=0)=>{if(depth>MAX_WALK_DEPTH){return value}if(typeof value==="string"){return rewrite(value)}if(Array.isArray(value)){return value.map(item=>mapStringLeaves(item,rewrite,depth+1))}if(typeof value==="object"&&value!==null){return Object.fromEntries(Object.entries(value).map(([key,child])=>[key,mapStringLeaves(child,rewrite,depth+1)]))}return value}

// HAND-PORT splitPrefixedToken:
// token=>{const separatorIndex=token.indexOf(":");if(separatorIndex<=0){return null}return{prefix:token.slice(0,separatorIndex),id:token.slice(separatorIndex+1)}}
