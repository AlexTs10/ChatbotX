//! `bot-field-variable.ts`, `bot-field-variable-resolver.ts` and the
//! `parseFieldReference` grammar of `@chatbotx.io/flow-config`.

use std::sync::OnceLock;

use chatbotx_kv::cache::CacheOpts;
use regex::Regex;

use super::sources::VariableSources;
use super::{BotFieldValue, BotFieldsMap, VariableError, Variables};
use crate::utils::variables::variable_placeholder_re;

/// `BOT_FIELD_REFERENCE_PREFIX`.
pub const BOT_FIELD_REFERENCE_PREFIX: &str = "bot_field";

/// `BOT_FIELDS_CACHE_TTL_SECONDS`.
pub const BOT_FIELDS_CACHE_TTL_SECONDS: u64 = 5 * 60;

fn bot_field_reference_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^bot_field:([0-9]+)$").expect("valid regex"))
}

/// `FieldReference`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldReference {
    /// An id or a name (legacy) of a contact custom field.
    CustomField { key: String },
    /// A workspace Bot Field (`bot_field:<id>`).
    BotField { id: String },
}

/// `parseFieldReference`: a well-formed `bot_field:<digits>` token is a bot
/// field; anything else a custom field key, byte for byte.
pub fn parse_field_reference(raw: &str) -> FieldReference {
    match bot_field_reference_re().captures(raw) {
        Some(caps) => FieldReference::BotField {
            id: caps[1].to_owned(),
        },
        None => FieldReference::CustomField {
            key: raw.to_owned(),
        },
    }
}

/// `formatBotFieldReference`.
pub fn format_bot_field_reference(id: &str) -> String {
    format!("{BOT_FIELD_REFERENCE_PREFIX}:{id}")
}

/// `botFieldWorkspaceCacheTags`.
pub fn bot_field_workspace_cache_tags(workspace_id: &str) -> Vec<String> {
    vec![
        "bot-fields".to_owned(),
        format!("bot-fields:{workspace_id}"),
    ]
}

/// `BotFieldVariableTextParseResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotFieldVariableTextParseResult {
    Malformed,
    Valid { field_ids: Vec<String> },
}

/// `parseBotFieldVariableText`: static text plus `{{bot_field:<id>}}`
/// variables only (client-safe for Builder schemas).
pub fn parse_bot_field_variable_text(text: &str) -> BotFieldVariableTextParseResult {
    let mut field_ids = Vec::new();
    for caps in variable_placeholder_re().captures_iter(text) {
        let variable = &caps[1];
        if variable.is_empty() {
            return BotFieldVariableTextParseResult::Malformed;
        }
        match parse_field_reference(variable) {
            FieldReference::BotField { id } => field_ids.push(id),
            FieldReference::CustomField { .. } => {
                return BotFieldVariableTextParseResult::Malformed;
            }
        }
    }
    let static_text = variable_placeholder_re().replace_all(text, "");
    if static_text.contains('{') || static_text.contains('}') {
        return BotFieldVariableTextParseResult::Malformed;
    }
    BotFieldVariableTextParseResult::Valid { field_ids }
}

/// `BotFieldVariableTextResolution`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotFieldVariableTextResolution {
    Malformed,
    Missing { field_id: String },
    Empty { field_id: String },
    Resolved { value: String },
}

impl<S: VariableSources> Variables<'_, S> {
    /// `loadBotFields`: the workspace's Bot Fields keyed by id, cached for
    /// [`BOT_FIELDS_CACHE_TTL_SECONDS`] under the workspace bot-field tags
    /// (Bot Field writes invalidate them).
    pub async fn load_bot_fields(&self, workspace_id: &str) -> Result<BotFieldsMap, VariableError> {
        let opts = CacheOpts {
            ttl_secs: BOT_FIELDS_CACHE_TTL_SECONDS,
            tags: bot_field_workspace_cache_tags(workspace_id),
            ..CacheOpts::default()
        };
        let map = self
            .cache
            .get_or_load::<BotFieldsMap, VariableError, _>(
                &format!("bot-fields:{workspace_id}:variable-map"),
                opts,
                || async {
                    let rows = self.sources.list_bot_fields(workspace_id).await?;
                    Ok(Some(
                        rows.into_iter()
                            .map(|row| {
                                (
                                    row.id,
                                    BotFieldValue {
                                        kind: row.kind,
                                        value: row.value,
                                    },
                                )
                            })
                            .collect(),
                    ))
                },
            )
            .await?;
        Ok(map.unwrap_or_default())
    }

    /// `resolveBotFieldVariableText`: fails closed for a deleted (missing)
    /// or empty Bot Field.
    pub async fn resolve_bot_field_variable_text(
        &self,
        text: &str,
        workspace_id: &str,
    ) -> Result<BotFieldVariableTextResolution, VariableError> {
        let field_ids = match parse_bot_field_variable_text(text) {
            BotFieldVariableTextParseResult::Malformed => {
                return Ok(BotFieldVariableTextResolution::Malformed);
            }
            BotFieldVariableTextParseResult::Valid { field_ids } => field_ids,
        };
        if field_ids.is_empty() {
            return Ok(BotFieldVariableTextResolution::Resolved {
                value: text.to_owned(),
            });
        }
        let bot_fields = self.load_bot_fields(workspace_id).await?;
        let mut value = text.to_owned();
        for field_id in field_ids {
            let Some(field) = bot_fields.get(&field_id) else {
                return Ok(BotFieldVariableTextResolution::Missing { field_id });
            };
            let Some(field_value) = field.value.as_deref().filter(|v| !v.is_empty()) else {
                return Ok(BotFieldVariableTextResolution::Empty { field_id });
            };
            value = value.replace(&format!("{{{{bot_field:{field_id}}}}}"), field_value);
        }
        Ok(BotFieldVariableTextResolution::Resolved { value })
    }
}
