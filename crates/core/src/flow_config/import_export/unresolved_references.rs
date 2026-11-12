//! Port of `packages/flow-config/src/import-export/unresolved-references.ts`.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use crate::flow_config::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Key(String),
    /// `Number.parseInt` of the bracket text; `None` is `NaN` (never matches).
    Index(Option<usize>),
}

/// `nodes[0].data.steps[2].tagIds[1]` -> `["nodes", 0, "data", "steps", 2, "tagIds", 1]`.
fn parse_path(path: &str) -> Vec<Seg> {
    let mut segments = Vec::new();
    for part in path.split('.') {
        let mut pieces = part.split('[');
        if let Some(key) = pieces.next()
            && !key.is_empty()
        {
            segments.push(Seg::Key(key.to_string()));
        }
        for index in pieces {
            let digits: String = index
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            segments.push(Seg::Index(digits.parse().ok()));
        }
    }
    segments
}

fn read_at<'v>(root: &'v Value, segments: &[Seg]) -> Option<&'v Value> {
    let mut current = root;
    for segment in segments {
        current = match (segment, current) {
            (Seg::Index(Some(index)), Value::Array(items)) => items.get(*index)?,
            (Seg::Key(key), Value::Object(map)) => map.get(key)?,
            _ => return None,
        };
    }
    Some(current)
}

fn read_at_mut<'v>(root: &'v mut Value, segments: &[Seg]) -> Option<&'v mut Value> {
    let mut current = root;
    for segment in segments {
        current = match (segment, current) {
            (Seg::Index(Some(index)), Value::Array(items)) => items.get_mut(*index)?,
            (Seg::Key(key), Value::Object(map)) => map.get_mut(key)?,
            _ => return None,
        };
    }
    Some(current)
}

fn has_step_type(value: Option<&Value>) -> bool {
    value.is_some_and(|v| v.get("stepType").is_some_and(Value::is_string))
}

/// The nearest enclosing object with a string `stepType`, and the path relative to it.
fn find_owning_step(root: &Value, segments: &[Seg]) -> Option<(String, String)> {
    for end in (0..segments.len()).rev() {
        let candidate = read_at(root, &segments[..end]);
        if let Some(step_type) = candidate
            .and_then(|c| c.get("stepType"))
            .and_then(Value::as_str)
        {
            let relative: Vec<&str> = segments[end..]
                .iter()
                .filter_map(|s| match s {
                    Seg::Key(key) => Some(key.as_str()),
                    Seg::Index(_) => None,
                })
                .collect();
            return Some((step_type.to_string(), relative.join(".")));
        }
    }
    None
}

fn is_condition_row(container: Option<&Value>) -> bool {
    container.is_some_and(|c| {
        c.get("field").is_some_and(Value::is_string)
            && c.get("operator").is_some_and(Value::is_string)
    })
}

fn shape_of_slot(root: &Value, segments: &[Seg]) -> String {
    let Some(Seg::Key(key)) = segments.last() else {
        return "required".to_string();
    };
    if is_condition_row(read_at(root, &segments[..segments.len() - 1])) {
        // A Condition row's id slots are optional.
        return "optional".to_string();
    }
    let by_step = find_owning_step(root, segments).and_then(|(step_type, relative)| {
        step_reference_slot_shape()
            .get(&step_type)
            .and_then(|shapes| shapes.get(&relative))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    by_step
        .or_else(|| {
            reference_slot_shape_by_key()
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "required".to_string())
}

/// Arrays whose elements are the graph itself.
const GRAPH_LEVEL_ARRAYS: &[&str] = &["nodes", "edges"];

enum SlotOutcome {
    Removed,
    DroppedElement(Vec<Seg>),
    Blocked,
    Skipped,
}

/// The nearest enclosing droppable row (see the TypeScript notes), if any.
fn find_droppable_element(root: &Value, segments: &[Seg]) -> Option<Vec<Seg>> {
    let inside_step = (0..segments.len()).any(|end| has_step_type(read_at(root, &segments[..end])));
    for end in (1..segments.len()).rev() {
        if !matches!(segments[end], Seg::Index(_)) {
            continue;
        }
        if let Seg::Key(array_key) = &segments[end - 1]
            && GRAPH_LEVEL_ARRAYS.contains(&array_key.as_str())
        {
            return None;
        }
        let element = read_at(root, &segments[..=end]);
        if has_step_type(element) {
            return None;
        }
        if !inside_step
            || is_condition_row(element)
            || element.is_some_and(|e| e.get("actionType").is_some_and(Value::is_string))
        {
            return Some(segments[..=end].to_vec());
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequiredSlotMode {
    Clear,
    Repair,
}

fn splice(container: Option<&mut Value>, index: &Seg) -> bool {
    match (container, index) {
        (Some(Value::Array(items)), Seg::Index(index)) => {
            if let Some(index) = index
                && *index < items.len()
            {
                items.remove(*index);
            }
            true
        }
        _ => false,
    }
}

fn remove_at(root: &mut Value, segments: &[Seg], mode: RequiredSlotMode) -> SlotOutcome {
    let Some(last) = segments.last() else {
        return SlotOutcome::Skipped;
    };
    let parent_path = &segments[..segments.len() - 1];
    let parent_is_array = read_at(root, parent_path).is_some_and(Value::is_array);
    if parent_is_array && matches!(last, Seg::Index(_)) {
        splice(read_at_mut(root, parent_path), last);
        return SlotOutcome::Removed;
    }
    let Seg::Key(key) = last else {
        return SlotOutcome::Skipped;
    };
    if !read_at(root, parent_path).is_some_and(Value::is_object) {
        return SlotOutcome::Skipped;
    }
    let shape = shape_of_slot(root, segments);
    let element_path = if shape == "required" && mode == RequiredSlotMode::Repair {
        find_droppable_element(root, segments)
    } else {
        None
    };
    let Some(Value::Object(parent)) = read_at_mut(root, parent_path) else {
        return SlotOutcome::Skipped;
    };
    match shape.as_str() {
        "optional" => {
            parent.remove(key);
            return SlotOutcome::Removed;
        }
        "nullable" => {
            parent.insert(key.clone(), Value::Null);
            return SlotOutcome::Removed;
        }
        "emptiable" => {
            parent.insert(key.clone(), json!(""));
            return SlotOutcome::Removed;
        }
        _ => {}
    }
    let Some(element_path) = element_path else {
        // Never left pointing at another workspace's id, even if the caller keeps the graph.
        parent.insert(key.clone(), json!(""));
        return if mode == RequiredSlotMode::Clear {
            SlotOutcome::Removed
        } else {
            SlotOutcome::Blocked
        };
    };
    let index = element_path.last().cloned().unwrap_or(Seg::Index(None));
    if splice(
        read_at_mut(root, &element_path[..element_path.len() - 1]),
        &index,
    ) {
        return SlotOutcome::DroppedElement(element_path);
    }
    SlotOutcome::Blocked
}

fn seg_string(seg: &Seg) -> String {
    match seg {
        Seg::Key(key) => key.clone(),
        Seg::Index(Some(index)) => index.to_string(),
        Seg::Index(None) => "NaN".to_string(),
    }
}

/// `localeCompare` for ASCII identifiers: case-insensitive first, lower case first on a tie.
fn locale_compare(left: &str, right: &str) -> Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| right.cmp(left))
}

fn compare_paths_descending(a: &[Seg], b: &[Seg]) -> Ordering {
    for (left, right) in a.iter().zip(b) {
        if left == right {
            continue;
        }
        if let (Seg::Index(Some(l)), Seg::Index(Some(r))) = (left, right) {
            return r.cmp(l);
        }
        return locale_compare(&seg_string(right), &seg_string(left));
    }
    b.len().cmp(&a.len())
}

fn path_key(segments: &[Seg]) -> String {
    segments
        .iter()
        .map(|s| match s {
            Seg::Key(key) => serde_json::to_string(key).unwrap_or_default(),
            Seg::Index(Some(index)) => index.to_string(),
            Seg::Index(None) => "null".to_string(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

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

fn remove_unresolved(
    value: &Value,
    references: &[&str],
    mode: RequiredSlotMode,
) -> RemoveUnresolvedResult {
    let mut result = RemoveUnresolvedResult {
        value: value.clone(),
        removed: Vec::new(),
        dropped_elements: Vec::new(),
        blocked: Vec::new(),
    };
    if references.is_empty() {
        return result;
    }
    let mut ordered: Vec<(&str, Vec<Seg>)> = references
        .iter()
        .map(|path| (*path, parse_path(path)))
        .collect();
    ordered.sort_by(|a, b| compare_paths_descending(&a.1, &b.1));
    let mut dropped: BTreeSet<String> = BTreeSet::new();
    for (path, segments) in ordered {
        if (1..=segments.len()).any(|end| dropped.contains(&path_key(&segments[..end]))) {
            continue;
        }
        match remove_at(&mut result.value, &segments, mode) {
            SlotOutcome::Removed => result.removed.push(path.to_string()),
            SlotOutcome::DroppedElement(element_path) => {
                dropped.insert(path_key(&element_path));
                result.dropped_elements.push(path.to_string());
            }
            SlotOutcome::Blocked => result.blocked.push(path.to_string()),
            SlotOutcome::Skipped => {}
        }
    }
    result
}

/// `removeUnresolvedReferencesDetailed(value, references)` (repair mode): each
/// reported reference removed the way its slot allows. Never mutates the input.
pub fn remove_unresolved_references_detailed(
    value: &Value,
    references: &[&str],
) -> RemoveUnresolvedResult {
    remove_unresolved(value, references, RequiredSlotMode::Repair)
}

/// `removeUnresolvedReferences(value, references)` (clear mode).
pub fn remove_unresolved_references(value: &Value, references: &[&str]) -> Value {
    remove_unresolved(value, references, RequiredSlotMode::Clear).value
}
