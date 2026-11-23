//! The builder reads the flow editor makes for the Q.D13 domains:
//! `decisionProfilesAPI.listActiveForFlow`,
//! `dynamicImagesAPI.getDynamicImageAPI`,
//! `leaderboardsAPI.listLeaderboardOptionsAPI`.

use chatbotx_business::{BusinessError, Result, tx};
use chatbotx_db::repo::leaderboard::{self, LeaderboardFilter};
use chatbotx_db::repo::leaderboard_participant;
use chatbotx_db::repo::models::{DecisionConnection, DecisionProfile, DynamicImage};
use chatbotx_db::repo::utils::PaginationInput;
use chatbotx_db::repo::{self};
use serde_json::{Value, json};

use crate::handlers::ApiState;

/// `listLeaderboardOptionsAPI`: `OPTIONS_PAGE_SIZE`.
const OPTIONS_PAGE_SIZE: i64 = 200;

/// `decisionProfileService.listForFlow` as the procedure's output.
pub async fn decision_profiles_for_flow(api: &ApiState, workspace_id: i64) -> Result<Value> {
    tx::read(&api.db, move |c| {
        let profiles: Vec<DecisionProfile> = repo::find_many(
            c,
            r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt" DESC"#,
            &[&workspace_id],
        )?;
        let mut out = Vec::with_capacity(profiles.len());
        for profile in profiles {
            let connection = repo::find_one::<DecisionConnection>(
                c,
                r#""id" = ?1 AND "workspaceId" = ?2"#,
                &[&profile.connection_id, &workspace_id],
            )?;
            let available = connection.is_some_and(|conn| {
                conn.status == "enabled"
                    && conn
                        .model_catalog
                        .0
                        .as_array()
                        .is_some_and(|m| m.iter().any(|x| x.as_str() == Some(&profile.model)))
            });
            out.push(json!({
                "connectionAvailable": available,
                "contract": profile.contract.0,
                "id": profile.id.to_string(),
                "model": profile.model,
                "name": profile.name,
                "profileEnabled": profile.status == "enabled",
            }));
        }
        Ok(Value::Array(out))
    })
    .await
}

/// `dynamicImageService.find`.
pub async fn dynamic_image(api: &ApiState, workspace_id: i64, id: i64) -> Result<Value> {
    tx::read(&api.db, move |c| {
        repo::find_one::<DynamicImage>(
            c,
            r#""id" = ?1 AND "workspaceId" = ?2"#,
            &[&id, &workspace_id],
        )?
        .map(|row| row.to_json())
        .ok_or_else(|| BusinessError::not_found("Dynamic image not found"))
    })
    .await
}

/// `listLeaderboardOptionsAPI`: the first 200 leaderboards, newest first,
/// with their participant counts (the Postgres stats path).
pub async fn leaderboard_options(api: &ApiState, workspace_id: i64) -> Result<Value> {
    tx::read(&api.db, move |c| {
        let filter = LeaderboardFilter {
            workspace_id,
            status: None,
            platform: None,
            keyword: None,
        };
        let page = PaginationInput {
            page: Some(1),
            per_page: Some(OPTIONS_PAGE_SIZE),
        };
        let rows = leaderboard::list_paginated(c, &filter, page)?;
        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            let lb = row.leaderboard;
            let (participants, _, _) = leaderboard_participant::stats(c, lb.id, i64::MAX)?;
            data.push(json!({
                "id": lb.id.to_string(),
                "name": lb.name,
                "status": lb.status,
                "platform": lb.platform,
                "participants": participants,
            }));
        }
        Ok(json!({ "data": data }))
    })
    .await
}
