//! `getAIToolset` (`packages/ai/src/server/toolset.ts` + `tools/*`): the
//! knowledge-file search, the AI functions and the MCP server tools of an
//! agent / step `tools` list.
//!
//! ponytail: the system tools (`sys:` — handoff, document / image / URL
//! readers, web search) come with the automated response (they need the
//! conversation context getter); the flow steps pass none, as the
//! original runner passes no `systemFunctionContextGetter`.

use std::collections::HashMap;
use std::sync::Arc;

use chatbotx_business::ai::knowledge::{self, Embedder, FileSearchConfig};
use chatbotx_business::ai::mcp_client::{
    EndpointGuard as _, McpAuth, McpClient, normalize_mcp_content,
};
use chatbotx_business::ai_mcp_server::{self, McpEndpointPolicy};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params_from_iter;
use chatbotx_jobs::BoxFuture;
use regex::Regex;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use super::llm::{ToolDef, Tools};
use super::text;

const KNOWLEDGE_BASE_TOOL: &str = "search_knowledge_base";
/// `DEFAULT_FILE_SEARCH_MAX_RESULTS`.
const DEFAULT_FILE_SEARCH_MAX_RESULTS: i64 = 5;

enum Entry {
    Files(Vec<String>),
    Function {
        output_message: String,
    },
    Mcp {
        client: Arc<Mutex<McpClient>>,
        tool: String,
    },
}

/// The loaded tools.
pub struct Toolset {
    db: Arc<Db>,
    embedder: Arc<dyn Embedder>,
    workspace_id: i64,
    defs: Vec<ToolDef>,
    entries: HashMap<String, Entry>,
}

impl Toolset {
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn has(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    fn add(&mut self, def: ToolDef, entry: Entry) {
        // Later sources win, as the object spread of the original.
        self.defs.retain(|d| d.name != def.name);
        self.entries.insert(def.name.clone(), entry);
        self.defs.push(def);
    }
}

/// `parseToolIds`.
fn tool_ids(tools: &[String], prefix: &str) -> Vec<String> {
    tools
        .iter()
        .filter(|t| t.starts_with(prefix))
        .map(|t| t.replacen(&format!("{prefix}:"), "", 1))
        .filter(|id| !id.is_empty())
        .collect()
}

/// `sanitizeKey` of an AI function's `dataCollect.from`.
fn sanitize_key(from: &str) -> String {
    let lowered = from.trim().to_lowercase();
    let underscored = Regex::new(r"\s+")
        .map(|re| re.replace_all(&lowered, "_").into_owned())
        .unwrap_or(lowered);
    underscored
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
        .collect()
}

fn mcp_safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `getAIToolset`: a failing source contributes no tools (logged).
pub async fn load(
    db: &Arc<Db>,
    embedder: &Arc<dyn Embedder>,
    workspace_id: i64,
    tools: &[String],
) -> Toolset {
    let mut set = Toolset {
        db: Arc::clone(db),
        embedder: Arc::clone(embedder),
        workspace_id,
        defs: Vec::new(),
        entries: HashMap::new(),
    };
    let file_ids = tool_ids(tools, "file");
    let fn_ids: Vec<i64> = tool_ids(tools, "fn")
        .iter()
        .filter_map(|id| id.parse().ok())
        .collect();
    let mcp_ids: Vec<i64> = tool_ids(tools, "mcp")
        .iter()
        .filter_map(|id| id.parse().ok())
        .collect();

    // getAIFileTools: the tool exists when one selected file is in the workspace.
    if !file_ids.is_empty() {
        let ids: Vec<i64> = file_ids.iter().filter_map(|id| id.parse().ok()).collect();
        let found = db
            .read(move |conn| {
                if ids.is_empty() {
                    return Ok(false);
                }
                let marks = vec!["?"; ids.len()].join(", ");
                let mut values: Vec<i64> = vec![workspace_id];
                values.extend(&ids);
                Ok(conn
                    .query_row(
                        &format!(r#"SELECT 1 FROM "AIFile" WHERE "workspaceId" = ? AND "id" IN ({marks}) LIMIT 1"#),
                        params_from_iter(values),
                        |_| Ok(()),
                    )
                    .is_ok())
            })
            .await;
        match found {
            Ok(true) => set.add(
                ToolDef {
                    name: KNOWLEDGE_BASE_TOOL.into(),
                    description: text::FILE_SEARCH_DESCRIPTION.into(),
                    parameters: json!({ "type": "object", "properties": {
                        "query": { "type": "string", "description": text::FILE_SEARCH_QUERY_DESCRIPTION } },
                        "required": ["query"] }),
                },
                Entry::Files(file_ids),
            ),
            Ok(false) => {}
            Err(err) => {
                tracing::error!(err = %err, workspace_id, "[ai-package] getAIFileTools failed")
            }
        }
    }

    // getAIFunctionTools (no context getter: the tool returns its output message).
    if !fn_ids.is_empty() {
        let rows = db
            .read(move |conn| {
                let marks = vec!["?"; fn_ids.len()].join(", ");
                let mut values: Vec<i64> = vec![workspace_id];
                values.extend(&fn_ids);
                let mut stmt = conn.prepare(&format!(
                    r#"SELECT "name", "purpose", "outputMessage", "dataCollect" FROM "AIFunction" WHERE "workspaceId" = ? AND "id" IN ({marks}) ORDER BY "id""#
                ))?;
                let rows = stmt
                    .query_map(params_from_iter(values), |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await;
        match rows {
            Ok(rows) => {
                for (name, purpose, output, data_collect) in rows {
                    let mut properties = Map::new();
                    let collect: Value = data_collect
                        .and_then(|d| serde_json::from_str(&d).ok())
                        .unwrap_or(Value::Null);
                    for entry in collect.as_array().into_iter().flatten() {
                        let from = entry.get("from").and_then(Value::as_str).unwrap_or("");
                        let to = entry.get("to").and_then(Value::as_str).unwrap_or("");
                        if from.is_empty() || to.is_empty() {
                            continue;
                        }
                        properties.insert(
                            sanitize_key(from),
                            json!({ "type": "string", "description": format!("The {from} value provided by the user") }),
                        );
                    }
                    set.add(
                        ToolDef {
                            name,
                            description: purpose.unwrap_or_default(),
                            parameters: json!({ "type": "object", "properties": properties, "additionalProperties": {} }),
                        },
                        Entry::Function {
                            output_message: output.unwrap_or_default(),
                        },
                    );
                }
            }
            Err(err) => {
                tracing::error!(err = %err, workspace_id, "[ai-package] getAIFunctionTools failed")
            }
        }
    }

    // getMCPServerTools.
    if !mcp_ids.is_empty() {
        let servers = db
            .read(move |conn| {
                let marks = vec!["?"; mcp_ids.len()].join(", ");
                let mut values: Vec<i64> = vec![workspace_id];
                values.extend(&mcp_ids);
                let mut stmt = conn.prepare(&format!(
                    r#"SELECT "name", "url", "auth", "availableTools", "selectedTools" FROM "AIMCPServer" WHERE "workspaceId" = ? AND "id" IN ({marks}) ORDER BY "id""#
                ))?;
                let rows = stmt
                    .query_map(params_from_iter(values), |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap_or_else(|err| {
                tracing::error!(err = %err, workspace_id, "[ai-package] getMCPServerTools failed");
                Vec::new()
            });
        let json_of = |text: Option<String>| -> Value {
            text.and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null)
        };
        for (name, url, auth, available, selected) in servers {
            let policy = McpEndpointPolicy::from_env();
            if let Err(err) = policy.check(&url).await {
                tracing::error!(err = %err, workspace_id, "[ai-package] Failed to load MCP server tools");
                continue;
            }
            let mut auth = McpAuth::from_json(&json_of(auth));
            if let McpAuth::Token(token) = &auth {
                match ai_mcp_server::resolve_bot_field_variable_text(db, workspace_id, token).await
                {
                    Ok(chatbotx_core::variables::bot_field::BotFieldVariableTextResolution::Resolved { value }) => {
                        auth = McpAuth::Token(value);
                    }
                    _ => continue,
                }
            }
            let client = Arc::new(Mutex::new(McpClient::new(
                &url,
                &auth,
                Some(&name),
                Arc::new(policy),
            )));
            let available = json_of(available);
            let server = mcp_safe_name(&name);
            for tool in json_of(selected)
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let unique = format!("{server}_{}", mcp_safe_name(tool));
                let cached = available.get(tool);
                let schema = cached
                    .and_then(|c| c.get("inputSchema"))
                    .filter(|s| s.is_object())
                    .map(|s| {
                        s.get("jsonSchema")
                            .filter(|j| j.is_object())
                            .unwrap_or(s)
                            .clone()
                    })
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                set.add(
                    ToolDef {
                        name: unique,
                        description: cached
                            .and_then(|c| c.get("description"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        parameters: schema,
                    },
                    Entry::Mcp {
                        client: Arc::clone(&client),
                        tool: tool.to_owned(),
                    },
                );
            }
        }
    }
    set
}

impl Tools for Toolset {
    fn defs(&self) -> Vec<ToolDef> {
        self.defs.clone()
    }

    fn call<'a>(&'a self, name: &'a str, args: Value) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            match self.entries.get(name) {
                None => Err(format!("Model tried to call unavailable tool '{name}'")),
                Some(Entry::Function { output_message }) => Ok(output_message.clone()),
                Some(Entry::Files(file_ids)) => {
                    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
                    let config = FileSearchConfig {
                        workspace_id: self.workspace_id,
                        selected_file_ids: file_ids.clone(),
                        similarity_threshold: None,
                        max_results: DEFAULT_FILE_SEARCH_MAX_RESULTS,
                    };
                    let results =
                        knowledge::perform_file_search(&self.db, &*self.embedder, query, config)
                            .await
                            .map_err(|e| e.to_string())?;
                    if results.is_empty() {
                        tracing::info!(
                            workspace_id = self.workspace_id,
                            query_length = query.len(),
                            "[knowledge-base] tool: no results found"
                        );
                        return Ok(text::FILE_SEARCH_NO_RESULT.into());
                    }
                    let listed = results
                        .iter()
                        .enumerate()
                        .map(|(i, r)| format!("{}. {}", i + 1, r.content))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    Ok(format!(
                        "{}\n\n{listed}",
                        text::file_search_found_prefix(results.len())
                    ))
                }
                Some(Entry::Mcp { client, tool }) => {
                    let args = args.as_object().cloned().unwrap_or_default();
                    let result = client
                        .lock()
                        .await
                        .call_tool(tool, &args)
                        .await
                        .map_err(|e| e.to_string())?;
                    if result.is_error {
                        return Err(match &result.content {
                            Value::String(s) => s.clone(),
                            Value::Array(items) => items
                                .first()
                                .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                                .and_then(|c| c.get("text"))
                                .and_then(Value::as_str)
                                .unwrap_or("MCP tool returned an error")
                                .to_owned(),
                            _ => "MCP tool returned an error".into(),
                        });
                    }
                    Ok(match normalize_mcp_content(&result.content) {
                        Value::String(s) => s,
                        other => other.to_string(),
                    })
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids_and_keys() {
        let tools = vec![
            "file:1".to_string(),
            "fn:2".into(),
            "fn:".into(),
            "mcp:3".into(),
        ];
        assert_eq!(tool_ids(&tools, "fn"), vec!["2"]);
        assert_eq!(tool_ids(&tools, "file"), vec!["1"]);
        assert_eq!(sanitize_key(" Phone  Number! "), "phone_number");
        assert_eq!(mcp_safe_name("My Server.v2"), "My_Server_v2");
    }
}
