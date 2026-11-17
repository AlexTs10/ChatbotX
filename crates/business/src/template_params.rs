//! `template-params/service.ts`: flat `{ key: value }` template parameters
//! into the nested send params of a workspace template.

use chatbotx_core::flow_config::steps::validate_wa_template_send_params;
use chatbotx_core::flow_config::steps::wa_template_params_schema;
use chatbotx_core::flow_config::steps::{
    AppliedTemplateParameters, TemplateParameterValues, apply_messenger_template_parameter_values,
    apply_wa_template_parameter_values,
};
use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

fn describe_problems(applied: &AppliedTemplateParameters) -> Vec<String> {
    [
        ("missing", &applied.missing),
        ("unknown", &applied.unknown),
        ("invalid", &applied.invalid),
        ("send templateData for", &applied.unsupported),
    ]
    .into_iter()
    .filter(|(_, keys)| !keys.is_empty())
    .map(|(label, keys)| format!("{label}: {}", keys.join(", ")))
    .collect()
}

/// The `(components, parameterFormat)` of a template of the workspace.
async fn load(
    db: &Db,
    channel: &str,
    workspace_id: i64,
    template_id: i64,
) -> Result<Option<(Value, String)>> {
    let (template, integration, fk, format) = match channel {
        "whatsapp" => (
            "WhatsappMessageTemplate",
            "IntegrationWhatsapp",
            "integrationWhatsappId",
            "''",
        ),
        _ => (
            "MessengerMessageTemplate",
            "IntegrationMessenger",
            "integrationMessengerId",
            r#"t."parameterFormat""#,
        ),
    };
    let sql = format!(
        r#"SELECT t."components", {format} FROM "{template}" AS t INNER JOIN "{integration}" AS i ON i."id" = t."{fk}"
           WHERE t."id" = ?1 AND i."workspaceId" = ?2"#
    );
    tx::read(db, move |conn| {
        Ok(conn
            .query_row(&sql, params![template_id, workspace_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()
            .map_err(repo::Error::from)?
            .map(|(components, format)| {
                (
                    serde_json::from_str(&components).unwrap_or(Value::Null),
                    format,
                )
            }))
    })
    .await
}

/// `resolveTemplateParams`: 404 for a template outside the workspace, 422 on
/// `field` for keys that do not match the template.
pub async fn resolve_template_params(
    db: &Db,
    workspace_id: i64,
    channel: &str,
    template_id: i64,
    values: &TemplateParameterValues,
    field: &str,
) -> Result<Value> {
    if channel != "whatsapp" && channel != "messenger" {
        return Err(BusinessError::validation(
            "channel",
            format!("Templates are only sent on whatsapp and messenger, not {channel}"),
        ));
    }
    let (components, format) = load(db, channel, workspace_id, template_id)
        .await?
        .ok_or_else(|| BusinessError::not_found("Template not found"))?;
    let components = components.as_array().map_or(&[][..], Vec::as_slice);
    let applied = if channel == "whatsapp" {
        apply_wa_template_parameter_values(components, values)
    } else {
        apply_messenger_template_parameter_values(components, &format, values)
    };
    let problems = describe_problems(&applied);
    if !problems.is_empty() {
        return Err(BusinessError::validation(
            field,
            format!(
                "Template parameters do not match the template ({})",
                problems.join("; ")
            ),
        ));
    }
    if channel == "whatsapp" {
        let checked = wa_template_params_schema()
            .super_refine(|params, ctx| validate_wa_template_send_params(params, ctx, &[]))
            .safe_parse(&applied.params);
        if !checked.success {
            return Err(BusinessError::validation(
                field,
                checked.messages().join("; "),
            ));
        }
    }
    Ok(applied.params)
}
