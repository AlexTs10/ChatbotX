//! Inboxes and inbox teams: `features/inboxes/api/private.ts`
//! (`listAllInboxesAuthenticatedAPI`) and
//! `enterprise/features/inbox-teams/api/private.ts` (`listInboxTeamsAuthenticatedAPI`).
//! Pure reads (no business logic), so they query the tables directly.

use chatbotx_db::repo::models::{
    Inbox, InboxTeam, InboxTeamMember, IntegrationInstagram, IntegrationMessenger, IntegrationSmtp,
    IntegrationTelegram, IntegrationThreads, IntegrationWebchat, IntegrationWhatsapp,
    IntegrationZalo, User,
};
use chatbotx_db::repo::{Model, find_many, id_list};
use chatbotx_db::rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::{Ctx, Outcome, fail, invalid};

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        "inboxesAPI.listAllInboxesAuthenticatedAPI" => list_all_inboxes(ctx, input).await,
        "inboxTeamsAPI.listInboxTeamsAuthenticatedAPI" => list_inbox_teams(ctx).await,
        _ => return None,
    })
}

/// `inboxService.listAllConnectedByWorkspace`: the connected inboxes, with
/// the integration relations when `includes` has `"integration"`.
async fn list_all_inboxes(ctx: &Ctx<'_>, input: &Value) -> Result<Value, crate::orpc::OrpcError> {
    let with_integrations = match input.get("includes") {
        None | Some(Value::Null) => false,
        Some(Value::Array(items)) if items.iter().all(|i| i.as_str() == Some("integration")) => {
            !items.is_empty()
        }
        Some(_) => return Err(invalid()),
    };
    let workspace_id = ctx.workspace_id;
    chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        let inboxes: Vec<Inbox> = find_many(
            conn,
            r#"WHERE "workspaceId" = ?1 AND "status" = 'connected' ORDER BY "id""#,
            &[&workspace_id],
        )?;
        let ids: Vec<i64> = inboxes.iter().map(|i| i.id).collect();
        let mut data: Vec<Value> = inboxes.iter().map(Inbox::to_json).collect();
        if with_integrations {
            attach_integrations(conn, &ids, &mut data)?;
        }
        Ok(json!({ "data": data }))
    })
    .await
    .map_err(fail)
}

/// `inboxWithIntegrationsResource`: each relation keyed by `inboxId`, with
/// the columns of its builder resource (`.pick`); `null` when absent.
fn attach_integrations(
    conn: &Connection,
    ids: &[i64],
    data: &mut [Value],
) -> Result<(), chatbotx_db::repo::Error> {
    fn rows<M: Model>(
        conn: &Connection,
        ids: &[i64],
        inbox_id: impl Fn(&M) -> i64,
        to_json: impl Fn(&M) -> Value,
    ) -> Result<Vec<(i64, Value)>, chatbotx_db::repo::Error> {
        let models: Vec<M> = find_many(
            conn,
            r#"WHERE "inboxId" IN (SELECT value FROM json_each(?1))"#,
            &[&id_list(ids)],
        )?;
        Ok(models.iter().map(|m| (inbox_id(m), to_json(m))).collect())
    }
    fn pick(value: Value, keys: &[&str]) -> Value {
        let Value::Object(object) = value else {
            return value;
        };
        Value::Object(
            object
                .into_iter()
                .filter(|(k, _)| keys.contains(&k.as_str()))
                .collect::<Map<_, _>>(),
        )
    }
    let relations: Vec<(&str, Vec<(i64, Value)>)> = vec![
        (
            "integrationWhatsapp",
            rows::<IntegrationWhatsapp>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| {
                    pick(
                        m.to_json(),
                        &[
                            "id",
                            "name",
                            "inboxId",
                            "displayPhoneNumber",
                            "tokenRefreshError",
                            "phoneNumberId",
                            "wabaId",
                            "hasCapiScope",
                            "capiScopeCheckedAt",
                            "datasetId",
                        ],
                    )
                },
            )?,
        ),
        (
            "integrationWebchat",
            rows::<IntegrationWebchat>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| pick(m.to_json(), &["id", "name"]),
            )?,
        ),
        (
            "integrationMessenger",
            rows::<IntegrationMessenger>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| pick(m.to_json(), &["id", "name", "inboxId"]),
            )?,
        ),
        (
            "integrationZalo",
            rows::<IntegrationZalo>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| pick(m.to_json(), &["id", "name"]),
            )?,
        ),
        (
            // ponytail: the builder resource is every column; `auth` (the bot
            // credential) is left out, the UI never reads it.
            "integrationTelegram",
            rows::<IntegrationTelegram>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| {
                    pick(
                        m.to_json(),
                        &[
                            "id",
                            "createdAt",
                            "updatedAt",
                            "botId",
                            "name",
                            "workspaceId",
                            "inboxId",
                        ],
                    )
                },
            )?,
        ),
        (
            "integrationInstagram",
            rows::<IntegrationInstagram>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| pick(m.to_json(), &["id", "name", "inboxId", "username"]),
            )?,
        ),
        (
            "integrationSmtp",
            rows::<IntegrationSmtp>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| pick(m.to_json(), &["id", "name", "fromAddress"]),
            )?,
        ),
        (
            "integrationThreads",
            rows::<IntegrationThreads>(
                conn,
                ids,
                |m| m.inbox_id,
                |m| pick(m.to_json(), &["id", "name", "inboxId", "username"]),
            )?,
        ),
    ];
    for (inbox, id) in data.iter_mut().zip(ids) {
        for (key, list) in &relations {
            let found = list
                .iter()
                .find(|(inbox_id, _)| inbox_id == id)
                .map_or(Value::Null, |(_, v)| v.clone());
            inbox[*key] = found;
        }
    }
    Ok(())
}

/// `inboxTeamService.listByWorkspace`: teams (oldest first) with their
/// members and each member's user.
async fn list_inbox_teams(ctx: &Ctx<'_>) -> Result<Value, crate::orpc::OrpcError> {
    let workspace_id = ctx.workspace_id;
    chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        let teams: Vec<InboxTeam> = find_many(
            conn,
            r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#,
            &[&workspace_id],
        )?;
        let team_ids: Vec<i64> = teams.iter().map(|t| t.id).collect();
        let members: Vec<InboxTeamMember> = find_many(
            conn,
            r#"WHERE "inboxTeamId" IN (SELECT value FROM json_each(?1)) ORDER BY "id""#,
            &[&id_list(&team_ids)],
        )?;
        let user_ids: Vec<i64> = members.iter().map(|m| m.user_id).collect();
        let users: Vec<User> = find_many(
            conn,
            r#"WHERE "id" IN (SELECT value FROM json_each(?1))"#,
            &[&id_list(&user_ids)],
        )?;
        let data: Vec<Value> = teams
            .iter()
            .map(|team| {
                let mut out = team.to_json();
                out["inboxTeamMembers"] = members
                    .iter()
                    .filter(|m| m.inbox_team_id == team.id)
                    .filter_map(|m| {
                        let user = users.iter().find(|u| u.id == m.user_id)?;
                        let mut member = m.to_json();
                        member["user"] = user.to_json();
                        Some(member)
                    })
                    .collect();
                out
            })
            .collect();
        Ok(json!({ "data": data }))
    })
    .await
    .map_err(fail)
}
