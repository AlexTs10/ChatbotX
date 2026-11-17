//! `packages/business/src/email-topic/service.ts` (Q.D5): the email topics of
//! a workspace. Private queries on `"EmailTopic"`, `"AnalyticsEmailTopic"`,
//! `"Folder"`. The tracking service (`tracking-service.ts`, `click-url.ts`)
//! goes with the email senders (Q.I3), its only callers.

use chatbotx_db::Db;
use chatbotx_db::repo::models::EmailTopic;
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by, with_id_tie_break};
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, like_contains, new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn name_taken() -> BusinessError {
    BusinessError::exception("Name is already taken.", "nameTaken", 400)
}

/// `list` input (`folderId: Some(0)` = the root).
#[derive(Debug, Clone, Default)]
pub struct ListEmailTopics {
    pub name: Option<String>,
    pub folder_id: Option<i64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct EmailTopicPage {
    pub data: Vec<EmailTopic>,
    pub page_count: i64,
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<EmailTopic>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            EmailTopic::select_sql()
        ),
        params![id, workspace_id],
        EmailTopic::from_row,
    )
    .optional()
    .map_err(repo_err)
}

fn find_or_fail_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<EmailTopic> {
    find_in(conn, workspace_id, id)?
        .ok_or_else(|| BusinessError::not_found("Email topic not found"))
}

/// `findOrFail`: 404 "Email topic not found".
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<EmailTopic> {
    tx::read(db, move |conn| find_or_fail_in(conn, workspace_id, id)).await
}

/// `listLabelsByIds`: `(id, name)` of the existing topics among `ids`.
pub async fn list_labels_by_ids(
    db: &Db,
    workspace_id: i64,
    ids: Vec<i64>,
) -> Result<Vec<(i64, String)>> {
    tx::read(db, move |conn| {
        let mut out = Vec::new();
        for id in ids {
            if let Some(topic) = find_in(conn, workspace_id, id)? {
                out.push((topic.id, topic.name));
            }
        }
        Ok(out)
    })
    .await
}

/// `list`: `sort` columns (default `createdAt` desc, then `id`), `name` an
/// ILIKE substring.
pub async fn list(db: &Db, workspace_id: i64, input: ListEmailTopics) -> Result<EmailTopicPage> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let requested = parse_order_by(EmailTopic::COLUMNS, input.sort.as_deref(), None);
        let order = if requested.is_empty() {
            parse_order_by(
                EmailTopic::COLUMNS,
                Some(&[SortItem {
                    id: "createdAt".into(),
                    desc: true,
                }]),
                None,
            )
        } else {
            requested
        };
        let order = with_id_tie_break(&order);
        let mut filter = String::from(r#""workspaceId" = ?"#);
        let mut binds: Vec<Sql> = vec![Sql::Integer(workspace_id)];
        if let Some(name) = input.name.as_deref().filter(|n| !n.is_empty()) {
            filter.push_str(r#" AND ilike("name", ?)"#);
            binds.push(Sql::Text(like_contains(name)));
        }
        match input.folder_id {
            Some(0) => filter.push_str(r#" AND "folderId" IS NULL"#),
            Some(folder) => {
                filter.push_str(r#" AND "folderId" = ?"#);
                binds.push(Sql::Integer(folder));
            }
            None => {}
        }
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT count(*) FROM "EmailTopic" WHERE {filter}"#),
                params_from_iter(binds.iter()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let sql = format!(
            r#"{} WHERE {filter} {} LIMIT ? OFFSET ?"#,
            EmailTopic::select_sql(),
            order_by_sql(&order, None)
        );
        binds.push(Sql::Integer(pagination.limit));
        binds.push(Sql::Integer(pagination.offset));
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let data = stmt
            .query_map(params_from_iter(binds.iter()), EmailTopic::from_row)
            .map_err(repo_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        let page_count = if pagination.limit > 0 {
            (total + pagination.limit - 1) / pagination.limit
        } else {
            1
        };
        Ok(EmailTopicPage { data, page_count })
    })
    .await
}

fn name_used(
    conn: &Connection,
    workspace_id: i64,
    name: &str,
    except: Option<i64>,
) -> Result<bool> {
    conn.query_row(
        r#"SELECT 1 FROM "EmailTopic" WHERE "name" = ?1 AND "workspaceId" = ?2 AND "id" IS NOT ?3"#,
        params![name, workspace_id, except],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(repo_err)
}

/// `create`: a taken name is `nameTaken` (400); a `folderId` must be an
/// `emailTopic` folder of the workspace (404 "Folder not found").
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: &str,
    folder_id: Option<i64>,
) -> Result<EmailTopic> {
    let name = name.to_string();
    tx::write(db, move |conn| {
        if name_used(conn, workspace_id, &name, None)? {
            return Err(name_taken());
        }
        if let Some(folder_id) = folder_id {
            conn.query_row(
                r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'emailTopic'"#,
                params![folder_id, workspace_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(repo_err)?
            .ok_or_else(|| BusinessError::not_found("Folder not found"))?;
        }
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "EmailTopic" ("id", "createdAt", "updatedAt", "name", "workspaceId", "folderId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5)"#,
            params![id, now, name, workspace_id, folder_id],
        )
        .map_err(repo_err)?;
        find_or_fail_in(conn, workspace_id, id)
    })
    .await
}

/// `update`: renames the topic (404, `nameTaken`).
pub async fn update(db: &Db, workspace_id: i64, id: i64, name: &str) -> Result<EmailTopic> {
    let name = name.to_string();
    tx::write(db, move |conn| {
        find_or_fail_in(conn, workspace_id, id)?;
        if name_used(conn, workspace_id, &name, Some(id))? {
            return Err(name_taken());
        }
        conn.execute(
            r#"UPDATE "EmailTopic" SET "name" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "workspaceId" = ?4"#,
            params![name, now_ms(), id, workspace_id],
        )
        .map_err(repo_err)?;
        find_or_fail_in(conn, workspace_id, id)
    })
    .await
}

/// `delete`: detaches the tracking rows (`topicReferenceId` keeps the topic,
/// `topicId` = null), then deletes the topics, in one transaction. Returns
/// `deletedCount`.
pub async fn delete(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<i64> {
    tx::write(db, move |conn| {
        let mut deleted = 0;
        for id in ids {
            conn.execute(
                r#"UPDATE "AnalyticsEmailTopic"
                   SET "topicReferenceId" = coalesce("topicReferenceId", "topicId"), "topicId" = NULL
                   WHERE "workspaceId" = ?1 AND "topicId" = ?2"#,
                params![workspace_id, id],
            )
            .map_err(repo_err)?;
            deleted += conn
                .execute(
                    r#"DELETE FROM "EmailTopic" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                    params![workspace_id, id],
                )
                .map_err(repo_err)? as i64;
        }
        Ok(deleted)
    })
    .await
}
