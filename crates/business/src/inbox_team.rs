//! `enterprise/inbox-team/service.ts` (Q.D11 slice 10): the inbox teams of a
//! workspace and their members. No list cache (no Redis caches in Rust).
//! The mutations return the user ids whose realtime connections the caller
//! revokes (`enqueueWorkspaceMemberRealtimeConnectionRevocationRetry`,
//! reason `reauth`) and the audit detail it records.

use std::collections::HashSet;

use chatbotx_db::Db;
use chatbotx_db::repo::models::InboxTeam;
use chatbotx_db::repo::{self, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};

use crate::{BusinessError, Result, tx};

/// One `User` row (`userResource`).
#[derive(Debug, Clone, PartialEq)]
pub struct TeamUser {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub name: Option<String>,
    pub email: String,
    pub email_verified: bool,
    pub image: Option<String>,
    pub is_anonymous: bool,
    pub must_change_password: bool,
    pub tenant_id: i64,
}

/// One `InboxTeamMember` row with its user.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamMember {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub inbox_team_id: i64,
    pub user_id: i64,
    pub user: TeamUser,
}

/// `InboxTeamWithMembers`.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamWithMembers {
    pub team: InboxTeam,
    pub members: Vec<TeamMember>,
}

/// A mutation's result: the team, the users to revoke, the audit
/// `(action, detail)` (`None` = no audit, as the original skips it).
#[derive(Debug, Clone, PartialEq)]
pub struct Mutation {
    pub team: InboxTeam,
    pub revoke_user_ids: Vec<i64>,
    pub audit: Option<(&'static str, String)>,
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Inbox team not found")
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> repo::Result<Option<InboxTeam>> {
    repo::find_one::<InboxTeam>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `listByWorkspace`: by `createdAt` (then id), members with their users.
pub async fn list_by_workspace(db: &Db, workspace_id: i64) -> Result<Vec<TeamWithMembers>> {
    tx::read(db, move |c| {
            let teams = repo::find_many::<InboxTeam>(
                c,
                "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" ASC, \"id\" ASC",
                &[&workspace_id],
            )?;
            let mut stmt = c.prepare_cached(
                r#"SELECT m."id", m."createdAt", m."updatedAt", m."inboxTeamId", m."userId",
                          u."createdAt", u."updatedAt", u."name", u."email", u."emailVerified", u."image",
                          u."isAnonymous", u."mustChangePassword", u."tenantId"
                   FROM "InboxTeamMember" m JOIN "InboxTeam" t ON t."id" = m."inboxTeamId"
                   JOIN "User" u ON u."id" = m."userId"
                   WHERE t."workspaceId" = ?1 ORDER BY m."id""#,
            )?;
            let members = stmt
                .query_map([workspace_id], |r| {
                    Ok(TeamMember {
                        id: r.get(0)?,
                        created_at: r.get(1)?,
                        updated_at: r.get(2)?,
                        inbox_team_id: r.get(3)?,
                        user_id: r.get(4)?,
                        user: TeamUser {
                            id: r.get(4)?,
                            created_at: r.get(5)?,
                            updated_at: r.get(6)?,
                            name: r.get(7)?,
                            email: r.get(8)?,
                            email_verified: r.get(9)?,
                            image: r.get(10)?,
                            is_anonymous: r.get(11)?,
                            must_change_password: r.get(12)?,
                            tenant_id: r.get(13)?,
                        },
                    })
                })?
                .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
            Ok(teams
                .into_iter()
                .map(|team| TeamWithMembers {
                    members: members
                        .iter()
                        .filter(|m| m.inbox_team_id == team.id)
                        .cloned()
                        .collect(),
                    team,
                })
                .collect())
    })
    .await
}

/// `findByIdOrFail`.
pub async fn find_by_id_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<InboxTeam> {
    tx::read(db, move |c| Ok(find_in(c, workspace_id, id)?))
        .await?
        .ok_or_else(not_found)
}

/// `assertAllAreWorkspaceMembers`.
fn assert_members(conn: &Connection, workspace_id: i64, user_ids: &[i64]) -> Result<()> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let wanted: HashSet<i64> = user_ids.iter().copied().collect();
    let found: i64 = conn.query_row(
        r#"SELECT COUNT(DISTINCT "userId") FROM "WorkspaceMember"
               WHERE "workspaceId" = ?1 AND "userId" IN (SELECT value FROM json_each(?2))"#,
        params![workspace_id, repo::id_list(user_ids)],
        |r| r.get(0),
    )?;
    if found as usize != wanted.len() {
        return Err(BusinessError::exception(
            "One or more userIds are not members of this workspace",
            "invalidTeamMember",
            400,
        ));
    }
    Ok(())
}

fn insert_members(conn: &Connection, team_id: i64, user_ids: &[i64]) -> repo::Result<()> {
    let now = now_ms();
    let mut stmt = conn.prepare_cached(
        r#"INSERT INTO "InboxTeamMember" ("id", "createdAt", "updatedAt", "inboxTeamId", "userId") VALUES (?1, ?2, ?2, ?3, ?4)"#,
    )?;
    for user_id in user_ids {
        stmt.execute(params![new_id(), now, team_id, user_id])?;
    }
    Ok(())
}

/// `create({ workspaceId, data: { name, userIds } })`.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: String,
    user_ids: Vec<i64>,
) -> Result<Mutation> {
    let team = tx::write(db, move |c| {
        assert_members(c, workspace_id, &user_ids)?;
        let id = new_id();
        let now = now_ms();
        c.execute(
            r#"INSERT INTO "InboxTeam" ("id", "createdAt", "updatedAt", "workspaceId", "name") VALUES (?1, ?2, ?2, ?3, ?4)"#,
            params![id, now, workspace_id, name],
        )
        ?;
        insert_members(c, id, &user_ids)?;
        let team = find_in(c, workspace_id, id)?.ok_or_else(not_found)?;
        Ok((team, user_ids))
    })
    .await?;
    let (team, user_ids) = team;
    let detail = format!("created a new team (#{})", team.id);
    Ok(Mutation {
        team,
        revoke_user_ids: dedup(user_ids),
        audit: Some(("create", detail)),
    })
}

fn dedup(ids: Vec<i64>) -> Vec<i64> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

/// `update(ctx, { name? })`: audited only when the name changes.
pub async fn update(db: &Db, workspace_id: i64, id: i64, name: Option<String>) -> Result<Mutation> {
    let (before, after) = tx::write(db, move |c| {
        let team = find_in(c, workspace_id, id)?.ok_or_else(not_found)?;
        if let Some(name) = &name {
            c.execute(
                r#"UPDATE "InboxTeam" SET "name" = ?2, "updatedAt" = ?3 WHERE "id" = ?1"#,
                params![id, name, now_ms()],
            )?;
        }
        let updated = find_in(c, workspace_id, id)?.unwrap_or_else(|| team.clone());
        Ok((team, updated))
    })
    .await?;
    let audit =
        (after.name != before.name).then(|| ("update", format!("updated a team (#{})", before.id)));
    Ok(Mutation {
        team: after,
        revoke_user_ids: Vec::new(),
        audit,
    })
}

/// `delete({ workspaceId, ids })`: 404 when none of `ids` is a team of the
/// workspace. The revoke list and the audit detail.
pub async fn delete(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<(Vec<i64>, String)> {
    let (teams, users) = tx::write(db, move |c| {
        let list = repo::id_list(&ids);
        let teams = {
            let mut stmt = c
                .prepare_cached(
                    r#"SELECT "id" FROM "InboxTeam" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#,
                )
                ?;
            stmt.query_map(params![workspace_id, list], |r| r.get::<_, i64>(0))
                .and_then(Iterator::collect::<chatbotx_db::rusqlite::Result<Vec<i64>>>)
                ?
        };
        if teams.is_empty() {
            return Err(not_found());
        }
        let users = repo::inbox_team_member::list_user_ids_by_team_ids(c, workspace_id, &teams)?;
        c.execute(
            r#"DELETE FROM "InboxTeam" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
            params![workspace_id, list],
        )
        ?;
        Ok((teams, users))
    })
    .await?;
    let plural = if teams.len() > 1 { "s" } else { "" };
    let list = teams
        .iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok((dedup(users), format!("deleted team{plural} ({list})")))
}

fn display_names(conn: &Connection, user_ids: &[i64]) -> repo::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT COALESCE("name", "email") FROM "User" WHERE "id" IN (SELECT value FROM json_each(?1)) ORDER BY "id""#,
    )?;
    let names = stmt
        .query_map([repo::id_list(user_ids)], |r| r.get(0))?
        .collect::<chatbotx_db::rusqlite::Result<Vec<String>>>()?;
    Ok(names)
}

/// `addMembers(ctx, userIds)`: adds the users not yet in the team; audited
/// when one was added.
pub async fn add_members(
    db: &Db,
    workspace_id: i64,
    id: i64,
    user_ids: Vec<i64>,
) -> Result<Mutation> {
    let (team, added, names) = tx::write(db, move |c| {
        let team = find_in(c, workspace_id, id)?.ok_or_else(not_found)?;
        assert_members(c, workspace_id, &user_ids)?;
        let existing: HashSet<i64> =
            repo::inbox_team_member::list_user_ids_by_team_id(c, workspace_id, id)?
                .into_iter()
                .collect();
        let added = dedup(
            user_ids
                .into_iter()
                .filter(|u| !existing.contains(u))
                .collect(),
        );
        insert_members(c, id, &added)?;
        let names = if added.is_empty() {
            Vec::new()
        } else {
            display_names(c, &added)?
        };
        Ok((team, added, names))
    })
    .await?;
    let audit = (!names.is_empty()).then(|| {
        (
            "update",
            format!("added {} to the team (#{})", names.join(", "), team.id),
        )
    });
    Ok(Mutation {
        team,
        revoke_user_ids: added,
        audit,
    })
}

/// `removeMembers(ctx, userIds)`: audited when a row was deleted.
pub async fn remove_members(
    db: &Db,
    workspace_id: i64,
    id: i64,
    user_ids: Vec<i64>,
) -> Result<Mutation> {
    let (team, removed, names) = tx::write(db, move |c| {
        let team = find_in(c, workspace_id, id)?.ok_or_else(not_found)?;
        let list = repo::id_list(&user_ids);
        let removed = {
            let mut stmt = c
                .prepare_cached(
                    r#"DELETE FROM "InboxTeamMember" WHERE "inboxTeamId" = ?1 AND "userId" IN (SELECT value FROM json_each(?2)) RETURNING "userId""#,
                )
                ?;
            stmt.query_map(params![id, list], |r| r.get::<_, i64>(0))
                .and_then(Iterator::collect::<chatbotx_db::rusqlite::Result<Vec<i64>>>)
                ?
        };
        let removed = dedup(removed);
        let names = if removed.is_empty() {
            Vec::new()
        } else {
            display_names(c, &removed)?
        };
        Ok((team, removed, names))
    })
    .await?;
    let audit = (!names.is_empty()).then(|| {
        (
            "update",
            format!("removed {} from the team (#{})", names.join(", "), team.id),
        )
    });
    Ok(Mutation {
        team,
        revoke_user_ids: removed,
        audit,
    })
}

/// `exists({ workspaceId, id })`.
pub fn exists(conn: &Connection, workspace_id: i64, id: i64) -> repo::Result<bool> {
    Ok(conn
        .query_row(
            r#"SELECT 1 FROM "InboxTeam" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
