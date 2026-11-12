//! Port of `packages/flow-config/src/import-export/remap.ts`.

use std::collections::{BTreeMap, BTreeSet};

use crate::flow_config::prelude::*;

/// `UnresolvedReference`: a reference slot whose value has no entry in its kind's id map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedReference {
    pub entity_kind: String,
    pub path: String,
    pub value: String,
}

/// `ReferenceIdMaps`: entity kind -> (source id -> target id).
pub type ReferenceIdMaps = BTreeMap<String, BTreeMap<String, String>>;

/// `RemapOptions`.
#[derive(Default)]
pub struct RemapOptions<'a> {
    /// Restrict remapping (and warning collection) to these entity kinds.
    pub kinds: Option<Vec<String>>,
    /// Called once per reference slot whose value has no entry in its kind's map.
    pub on_unresolved: Option<&'a mut dyn FnMut(&UnresolvedReference)>,
}

/// The step a walked object belongs to (`stepType` and the path of the step object).
#[derive(Clone)]
struct StepContext {
    step_type: String,
    base_path: String,
}

struct Walk<'m, 'o> {
    id_maps: &'m ReferenceIdMaps,
    kinds: Option<Vec<String>>,
    on_unresolved: Option<&'o mut dyn FnMut(&UnresolvedReference)>,
}

enum Scalar {
    Resolved(String),
    Unresolved,
}

fn table_str<'v>(table: &'v Value, key: &str) -> Option<&'v str> {
    table.get(key).and_then(Value::as_str)
}

impl Walk<'_, '_> {
    fn kind_enabled(&self, kind: &str) -> bool {
        self.kinds
            .as_ref()
            .is_none_or(|kinds| kinds.iter().any(|k| k == kind))
    }

    fn lookup(&self, kind: &str, id: &str) -> Option<String> {
        self.id_maps
            .get(kind)
            .and_then(|map| map.get(id))
            .filter(|mapped| !mapped.is_empty())
            .cloned()
    }

    fn report(&mut self, entity_kind: &str, path: &str, value: &str) {
        if let Some(callback) = self.on_unresolved.as_mut() {
            callback(&UnresolvedReference {
                entity_kind: entity_kind.to_string(),
                path: path.to_string(),
                value: value.to_string(),
            });
        }
    }

    fn remap_prefixed_token(&mut self, token: &str, path: &str) -> String {
        let Some((prefix, id)) = split_prefixed_token(token) else {
            return token.to_string();
        };
        let Some(entity_kind) = table_str(prefixed_reference_entity_kind(), prefix) else {
            return token.to_string();
        };
        if entity_kind.is_empty() || !self.kind_enabled(entity_kind) {
            return token.to_string();
        }
        if let Some(mapped) = self.lookup(entity_kind, id) {
            return format!("{prefix}:{mapped}");
        }
        self.report(entity_kind, path, id);
        token.to_string()
    }

    fn remap_prefixed_value(&mut self, value: &Value, path: &str) -> Value {
        match value {
            Value::String(token) => Value::String(self.remap_prefixed_token(token, path)),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| match item {
                        Value::String(token) => Value::String(
                            self.remap_prefixed_token(token, &format!("{path}[{index}]")),
                        ),
                        other => other.clone(),
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn resolve_scalar_ref(&mut self, value: &Value, entity_kind: &str, path: &str) -> Scalar {
        let Some(text) = value.as_str() else {
            return Scalar::Unresolved;
        };
        if crate::js::trim(text).is_empty() {
            return Scalar::Unresolved;
        }
        // A sentinel such as `unassigned` names no member or team: not a reference.
        if STRIP_UNRESOLVED_ARRAY_REFERENCE_KINDS.contains(&entity_kind)
            && !is_workspace_bound_reference_value(entity_kind, text)
        {
            return Scalar::Unresolved;
        }
        if let Some(mapped) = self.lookup(entity_kind, text) {
            return Scalar::Resolved(mapped);
        }
        self.report(entity_kind, path, text);
        Scalar::Unresolved
    }

    /// `Some(value)` when `value` is a well-formed `bot_field:<id>` token (resolved
    /// against `idMaps.botField`); `None` falls through to customField resolution.
    fn resolve_bot_field_reference_slot(&mut self, value: &Value, path: &str) -> Option<Value> {
        let text = value.as_str()?;
        let FieldReference::BotField { id } = parse_field_reference(text) else {
            return None;
        };
        if !self.kind_enabled("botField") {
            return Some(value.clone());
        }
        if let Some(mapped) = self.lookup("botField", &id) {
            return Some(Value::String(format_bot_field_reference(&mapped)));
        }
        self.report("botField", path, &id);
        Some(value.clone())
    }

    fn remap_array_ref(&mut self, value: &Value, entity_kind: &str, path: &str) -> Value {
        let Value::Array(items) = value else {
            return value.clone();
        };
        let strip = STRIP_UNRESOLVED_ARRAY_REFERENCE_KINDS.contains(&entity_kind);
        let mut out = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let item_path = format!("{path}[{index}]");
            match self.resolve_scalar_ref(item, entity_kind, &item_path) {
                Scalar::Resolved(mapped) => out.push(Value::String(mapped)),
                Scalar::Unresolved => {
                    let strippable = item
                        .as_str()
                        .is_some_and(|s| is_workspace_bound_reference_value(entity_kind, s));
                    if !(strip && strippable) {
                        out.push(item.clone());
                    }
                }
            }
        }
        Value::Array(out)
    }

    fn resolve_scalar_or_recurse(
        &mut self,
        child: &Value,
        kind: &str,
        child_path: &str,
        depth: usize,
        step: Option<&StepContext>,
    ) -> Value {
        match self.resolve_scalar_ref(child, kind, child_path) {
            Scalar::Resolved(mapped) => Value::String(mapped),
            Scalar::Unresolved => self.remap_value(child, child_path, depth + 1, step),
        }
    }

    fn remap_value(
        &mut self,
        value: &Value,
        path: &str,
        depth: usize,
        step: Option<&StepContext>,
    ) -> Value {
        if depth > MAX_WALK_DEPTH {
            return value.clone();
        }
        let object = match value {
            Value::Array(items) => {
                return Value::Array(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            self.remap_value(item, &format!("{path}[{index}]"), depth + 1, step)
                        })
                        .collect(),
                );
            }
            Value::Object(object) => object,
            other => return other.clone(),
        };

        let has_cross_flow_jump = self.kind_enabled("flow")
            && object
                .get(FLOW_REFERENCE_FIELD)
                .is_some_and(Value::is_string);
        let scoped = object
            .get("stepType")
            .and_then(Value::as_str)
            .map(|step_type| StepContext {
                step_type: step_type.to_string(),
                base_path: path.to_string(),
            });
        let step = scoped.as_ref().or(step);

        let mut next = Map::new();
        for (key, child) in object {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let value = self.remap_entry(
                key,
                child,
                &child_path,
                object,
                has_cross_flow_jump,
                depth,
                step,
            );
            next.insert(key.clone(), value);
        }
        Value::Object(next)
    }

    fn find_step_override(
        &self,
        child_path: &str,
        step: Option<&StepContext>,
    ) -> Option<&'static str> {
        let step = step?;
        let relative = if step.base_path.is_empty() {
            child_path
        } else {
            child_path.get(step.base_path.len() + 1..)?
        };
        step_reference_overrides()
            .get(&step.step_type)?
            .get(relative)?
            .as_str()
    }

    fn apply_step_override(&mut self, kind: &str, child: &Value, child_path: &str) -> Value {
        if kind == EXTERNAL_REFERENCE_KIND || !self.kind_enabled(kind) {
            return child.clone();
        }
        if child.is_array() {
            return self.remap_array_ref(child, kind, child_path);
        }
        match self.resolve_scalar_ref(child, kind, child_path) {
            Scalar::Resolved(mapped) => Value::String(mapped),
            Scalar::Unresolved => child.clone(),
        }
    }

    fn resolve_discriminated_kind(
        key: &str,
        container: &Map<String, Value>,
    ) -> Option<&'static str> {
        let config = discriminated_reference_fields().get(key)?;
        if let Some(Value::Array(siblings)) = config.get("requireSiblings")
            && siblings.iter().any(|sibling| {
                !container
                    .get(sibling.as_str().unwrap_or_default())
                    .is_some_and(Value::is_string)
            })
        {
            return None;
        }
        let discriminator = container
            .get(config["discriminatorKey"].as_str()?)?
            .as_str()?;
        config["kindByDiscriminator"].get(discriminator)?.as_str()
    }

    #[allow(clippy::too_many_arguments)]
    fn remap_entry(
        &mut self,
        key: &str,
        child: &Value,
        child_path: &str,
        container: &Map<String, Value>,
        has_cross_flow_jump: bool,
        depth: usize,
        step: Option<&StepContext>,
    ) -> Value {
        if let Some(kind) = self
            .find_step_override(child_path, step)
            .filter(|k| !k.is_empty())
        {
            return self.apply_step_override(kind, child, child_path);
        }

        if let Some(scalar_kind) =
            table_str(reference_field_entity_kind(), key).filter(|k| !k.is_empty())
        {
            // A `bot_field:<id>` token in any customField-kind slot goes to `idMaps.botField`.
            if scalar_kind == "customField"
                && let Some(value) = self.resolve_bot_field_reference_slot(child, child_path)
            {
                return value;
            }
            if self.kind_enabled(scalar_kind) {
                return self.resolve_scalar_or_recurse(child, scalar_kind, child_path, depth, step);
            }
        }

        if let Some(array_kind) =
            table_str(reference_array_field_entity_kind(), key).filter(|k| !k.is_empty())
            && self.kind_enabled(array_kind)
            && child.is_array()
        {
            return self.remap_array_ref(child, array_kind, child_path);
        }

        let discriminated =
            Self::resolve_discriminated_kind(key, container).filter(|k| !k.is_empty());
        if discriminated == Some(EXTERNAL_REFERENCE_KIND) {
            return child.clone();
        }
        if let Some(kind) = discriminated
            && self.kind_enabled(kind)
        {
            if child.is_array() {
                return self.remap_array_ref(child, kind, child_path);
            }
            return self.resolve_scalar_or_recurse(child, kind, child_path, depth, step);
        }

        if PREFIXED_REFERENCE_FIELDS.contains(&key) {
            return self.remap_prefixed_value(child, child_path);
        }

        if key == FLOW_REFERENCE_FIELD && self.kind_enabled("flow") {
            return self.resolve_scalar_or_recurse(child, "flow", child_path, depth, step);
        }

        if key == CROSS_FLOW_NODE_FIELD && has_cross_flow_jump && self.kind_enabled("flowNode") {
            // The node id only means something with its sibling flow id; an
            // unresolved flow has been reported already.
            let flow_id = container
                .get(FLOW_REFERENCE_FIELD)
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !self
                .id_maps
                .get("flow")
                .is_some_and(|map| map.contains_key(flow_id))
            {
                return child.clone();
            }
            return self.resolve_scalar_or_recurse(child, "flowNode", child_path, depth, step);
        }

        self.remap_value(child, child_path, depth + 1, step)
    }
}

fn walk<'m, 'o>(id_maps: &'m ReferenceIdMaps, options: RemapOptions<'o>) -> Walk<'m, 'o> {
    Walk {
        id_maps,
        kinds: options.kinds,
        on_unresolved: options.on_unresolved,
    }
}

/// `remapReferences(value, idMaps, options)`: a copy with every known reference
/// slot rewritten through `idMaps`; misses pass through and are reported.
pub fn remap_references(
    value: &Value,
    id_maps: &ReferenceIdMaps,
    options: RemapOptions<'_>,
) -> Value {
    walk(id_maps, options).remap_value(value, "", 0, None)
}

/// `remapFlowGraphReferences(flow, idMaps, options)`: remaps `nodes` and `edges`
/// (paths rooted at `nodes` / `edges`).
pub fn remap_flow_graph_references(
    value: &Value,
    id_maps: &ReferenceIdMaps,
    options: RemapOptions<'_>,
) -> Value {
    let mut walker = walk(id_maps, options);
    let mut out = value.as_object().cloned().unwrap_or_default();
    for key in ["nodes", "edges"] {
        if let Some(child) = value.get(key) {
            let remapped = walker.remap_value(child, key, 0, None);
            out.insert(key.to_string(), remapped);
        }
    }
    Value::Object(out)
}

/// `collectReferencesByKind(value, kinds)`: every reference id found, by entity kind.
pub fn collect_references_by_kind(
    value: &Value,
    kinds: Option<&[&str]>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut by_kind: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut collect = |reference: &UnresolvedReference| {
        by_kind
            .entry(reference.entity_kind.clone())
            .or_default()
            .insert(reference.value.clone());
    };
    remap_references(
        value,
        &ReferenceIdMaps::new(),
        RemapOptions {
            kinds: kinds.map(|kinds| kinds.iter().map(|k| (*k).to_string()).collect()),
            on_unresolved: Some(&mut collect),
        },
    );
    by_kind
}
