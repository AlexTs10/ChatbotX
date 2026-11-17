//! `ads-conversion/service.ts` rule CRUD (`list`, `findOrFail`, `create`,
//! `update`, `toggleEnabled`, `remove`) over `AdsConversionRule`.
//! ponytail: no `hasEnabledTriggerRule` cache yet (nothing reads it here).

use chatbotx_db::Db;
use chatbotx_db::repo::{iso, new_id, now_ms};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `adsConversionChannelValues`.
pub const CHANNELS: &[&str] = &["whatsapp", "facebook", "messenger", "instagram"];
/// `adsConversionEventTypeValues`.
pub const EVENT_TYPES: &[&str] = &["lead", "purchase"];

const COLUMNS: &str = "\"id\", \"createdAt\", \"updatedAt\", \"channel\", \"integrationWhatsappId\", \
    \"integrationFacebookAdsId\", \"integrationMessengerId\", \"integrationInstagramId\", \
    \"adAccountId\", \"eventType\", \"trigger\", \"markAs\", \"enabled\"";

/// The four per-channel integration FKs, in column order.
const FK_COLUMNS: [&str; 4] = [
    "integrationWhatsappId",
    "integrationFacebookAdsId",
    "integrationMessengerId",
    "integrationInstagramId",
];

fn id_text(id: Option<i64>) -> Value {
    id.map_or(Value::Null, |id| json!(id.to_string()))
}

/// `adsConversionRulePublicResource` (no `workspaceId`).
fn resource(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<Value> {
    let trigger: String = r.get(10)?;
    Ok(json!({
        "id": r.get::<_, i64>(0)?.to_string(),
        "createdAt": iso(r.get(1)?),
        "updatedAt": iso(r.get(2)?),
        "channel": r.get::<_, String>(3)?,
        "integrationWhatsappId": id_text(r.get(4)?),
        "integrationFacebookAdsId": id_text(r.get(5)?),
        "integrationMessengerId": id_text(r.get(6)?),
        "integrationInstagramId": id_text(r.get(7)?),
        "adAccountId": r.get::<_, Option<String>>(8)?,
        "eventType": r.get::<_, String>(9)?,
        "trigger": serde_json::from_str::<Value>(&trigger).unwrap_or(Value::Null),
        "markAs": r.get::<_, Option<String>>(11)?,
        "enabled": r.get::<_, i64>(12)? != 0,
    }))
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Ads conversion rule not found")
}

fn system(message: impl Into<String>) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

fn find(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Value>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM \"AdsConversionRule\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2"
            ),
            params![id, workspace_id],
            resource,
        )
        .optional()?)
}

/// The integration FKs of a rule, in [`FK_COLUMNS`] order.
pub type Fks = [Option<i64>; 4];

/// `assertIntegrationConsistency`: exactly the channel's own FK is set.
fn assert_consistency(channel: &str, fks: &Fks) -> Result<()> {
    let [wa, fb, msg, ig] = fks.map(|f| f.is_some());
    let ok = match channel {
        "whatsapp" => wa && !fb,
        "facebook" => fb && !wa,
        "messenger" => msg && !wa && !fb && !ig,
        "instagram" => ig && !wa && !fb && !msg,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(system(
            "Ads conversion rule integration must match the selected channel",
        ))
    }
}

/// `assertIntegrationOwnership`: the channel's integration is in the workspace.
fn assert_ownership(conn: &Connection, workspace_id: i64, channel: &str, fks: &Fks) -> Result<()> {
    let (table, id) = match channel {
        "whatsapp" => ("IntegrationWhatsapp", fks[0]),
        "facebook" => ("IntegrationFacebookAds", fks[1]),
        "messenger" => ("IntegrationMessenger", fks[2]),
        _ => ("IntegrationInstagram", fks[3]),
    };
    let found = match id {
        Some(id) => conn
            .query_row(
                &format!("SELECT 1 FROM \"{table}\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2"),
                params![id, workspace_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some(),
        None => false,
    };
    if found {
        Ok(())
    } else {
        Err(system(
            "Ads conversion rule integration was not found in this workspace",
        ))
    }
}

/// `assertSupportedTrigger`: Instagram has no `templateSent`.
fn assert_trigger(trigger: &Value, channel: &str) -> Result<()> {
    let kind = trigger["type"].as_str().unwrap_or_default();
    if !matches!(
        kind,
        "templateSent" | "tagApplied" | "keywordMatched" | "contactReplied"
    ) {
        return Err(system(format!(
            "Ads conversion trigger type \"{kind}\" is not supported yet"
        )));
    }
    if channel == "instagram" && kind == "templateSent" {
        return Err(system(format!(
            "Ads conversion trigger type \"{kind}\" is not supported for channel \"{channel}\""
        )));
    }
    Ok(())
}

/// `listByWorkspace`: oldest first.
pub async fn list(db: &Db, workspace_id: i64, channel: Option<String>) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM \"AdsConversionRule\" WHERE \"workspaceId\" = ?1 \
             AND (?2 IS NULL OR \"channel\" = ?2) ORDER BY \"createdAt\", \"id\""
        ))?;
        let rows = stmt
            .query_map(params![workspace_id, channel], resource)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// `findOrFail`.
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        find(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `createAdsConversionRuleInput` (parsed).
#[derive(Debug, Clone, Default)]
pub struct CreateInput {
    pub channel: String,
    pub fks: Fks,
    pub ad_account_id: Option<String>,
    pub event_type: String,
    pub trigger: Value,
    pub mark_as: Option<String>,
    pub enabled: Option<bool>,
}

/// `create`.
pub async fn create(db: &Db, workspace_id: i64, input: CreateInput) -> Result<Value> {
    assert_trigger(&input.trigger, &input.channel)?;
    assert_consistency(&input.channel, &input.fks)?;
    tx::write(db, move |tx| {
        assert_ownership(tx, workspace_id, &input.channel, &input.fks)?;
        let id = new_id();
        let [wa, fb, msg, ig] = input.fks;
        tx.execute(
            "INSERT INTO \"AdsConversionRule\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"channel\", \
             \"integrationWhatsappId\", \"integrationFacebookAdsId\", \"integrationMessengerId\", \
             \"integrationInstagramId\", \"adAccountId\", \"eventType\", \"trigger\", \"markAs\", \"enabled\") \
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                now_ms(),
                workspace_id,
                input.channel,
                wa,
                fb,
                msg,
                ig,
                input.ad_account_id,
                input.event_type,
                input.trigger.to_string(),
                input.mark_as,
                input.enabled.unwrap_or(true)
            ],
        )?;
        find(tx, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `updateAdsConversionRuleInput` (parsed; `None` = not sent, `Some(None)` = null).
#[derive(Debug, Clone, Default)]
pub struct Patch {
    pub channel: Option<String>,
    pub fks: [Option<Option<i64>>; 4],
    pub ad_account_id: Option<Option<String>>,
    pub event_type: Option<String>,
    pub trigger: Option<Value>,
    pub mark_as: Option<Option<String>>,
    pub enabled: Option<bool>,
}

fn opt_text(value: Option<String>) -> Sql {
    value.map_or(Sql::Null, Sql::Text)
}

fn opt_int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

fn write_patch(
    tx: &Connection,
    workspace_id: i64,
    id: i64,
    sets: Vec<(&str, Sql)>,
) -> Result<Value> {
    let mut sql = String::from("UPDATE \"AdsConversionRule\" SET \"updatedAt\" = ?1");
    let mut args = vec![Sql::Integer(now_ms())];
    for (column, value) in sets {
        args.push(value);
        sql.push_str(&format!(", \"{column}\" = ?{}", args.len()));
    }
    args.push(Sql::Integer(id));
    args.push(Sql::Integer(workspace_id));
    sql.push_str(&format!(
        " WHERE \"id\" = ?{} AND \"workspaceId\" = ?{}",
        args.len() - 1,
        args.len()
    ));
    if tx.execute(&sql, params_from_iter(args))? == 0 {
        return Err(not_found());
    }
    find(tx, workspace_id, id)?.ok_or_else(not_found)
}

/// `update`: merges the FKs with the stored row before the channel checks.
pub async fn update(db: &Db, workspace_id: i64, id: i64, patch: Patch) -> Result<Value> {
    tx::write(db, move |tx| {
        let existing = find(tx, workspace_id, id)?.ok_or_else(not_found)?;
        let channel = patch
            .channel
            .clone()
            .unwrap_or_else(|| existing["channel"].as_str().unwrap_or_default().to_owned());
        let mut merged: Fks = [None; 4];
        for (i, column) in FK_COLUMNS.iter().enumerate() {
            merged[i] = match patch.fks[i] {
                Some(value) => value,
                None => existing[*column].as_str().and_then(|s| s.parse().ok()),
            };
        }
        assert_consistency(&channel, &merged)?;
        assert_ownership(tx, workspace_id, &channel, &merged)?;
        if let Some(trigger) = &patch.trigger {
            assert_trigger(trigger, &channel)?;
        }
        let mut sets: Vec<(&str, Sql)> = Vec::new();
        if let Some(channel) = patch.channel {
            sets.push(("channel", Sql::Text(channel)));
        }
        for (i, column) in FK_COLUMNS.iter().enumerate() {
            if let Some(value) = patch.fks[i] {
                sets.push((column, opt_int(value)));
            }
        }
        if let Some(value) = patch.ad_account_id {
            sets.push(("adAccountId", opt_text(value)));
        }
        if let Some(value) = patch.event_type {
            sets.push(("eventType", Sql::Text(value)));
        }
        if let Some(value) = patch.trigger {
            sets.push(("trigger", Sql::Text(value.to_string())));
        }
        if let Some(value) = patch.mark_as {
            sets.push(("markAs", opt_text(value)));
        }
        if let Some(value) = patch.enabled {
            sets.push(("enabled", Sql::Integer(value.into())));
        }
        write_patch(tx, workspace_id, id, sets)
    })
    .await
}

/// `toggleEnabled`.
pub async fn toggle(db: &Db, workspace_id: i64, id: i64, enabled: bool) -> Result<Value> {
    tx::write(db, move |tx| {
        write_patch(
            tx,
            workspace_id,
            id,
            vec![("enabled", Sql::Integer(enabled.into()))],
        )
    })
    .await
}

/// `remove`.
pub async fn remove(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        let deleted = tx.execute(
            "DELETE FROM \"AdsConversionRule\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
            params![id, workspace_id],
        )?;
        if deleted == 0 {
            return Err(not_found());
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_channel_and_trigger() {
        assert!(assert_consistency("messenger", &[None, None, Some(1), None]).is_ok());
        assert!(assert_consistency("whatsapp", &[None, None, Some(1), None]).is_err());
        assert!(assert_consistency("whatsapp", &[Some(1), None, Some(1), None]).is_ok());
        assert!(assert_trigger(&json!({"type": "templateSent"}), "instagram").is_err());
        assert!(assert_trigger(&json!({"type": "templateSent"}), "messenger").is_ok());
    }
}
