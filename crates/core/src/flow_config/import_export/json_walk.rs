//! Port of `packages/flow-config/src/import-export/json-walk.ts`.

use crate::flow_config::prelude::*;

/// `forEachStringLeaf`: calls `visit` with every string leaf; past
/// `MAX_WALK_DEPTH` the rest of the subtree is skipped.
pub fn for_each_string_leaf(value: &Value, visit: &mut dyn FnMut(&str)) {
    walk_leaves(value, visit, 0);
}

fn walk_leaves(value: &Value, visit: &mut dyn FnMut(&str), depth: usize) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    match value {
        Value::String(text) => visit(text),
        Value::Array(items) => items
            .iter()
            .for_each(|item| walk_leaves(item, visit, depth + 1)),
        Value::Object(map) => map
            .values()
            .for_each(|child| walk_leaves(child, visit, depth + 1)),
        _ => {}
    }
}

/// `mapStringLeaves`: a copy with every string leaf rewritten; past
/// `MAX_WALK_DEPTH` the subtree is returned unchanged.
pub fn map_string_leaves(value: &Value, rewrite: &dyn Fn(&str) -> String) -> Value {
    map_leaves(value, rewrite, 0)
}

fn map_leaves(value: &Value, rewrite: &dyn Fn(&str) -> String, depth: usize) -> Value {
    if depth > MAX_WALK_DEPTH {
        return value.clone();
    }
    match value {
        Value::String(text) => Value::String(rewrite(text)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| map_leaves(item, rewrite, depth + 1))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| (key.clone(), map_leaves(child, rewrite, depth + 1)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `splitPrefixedToken`: `prefix:id` split at the first colon; `None` without a prefix.
pub fn split_prefixed_token(token: &str) -> Option<(&str, &str)> {
    match token.find(':') {
        Some(index) if index > 0 => Some((&token[..index], &token[index + 1..])),
        _ => None,
    }
}
