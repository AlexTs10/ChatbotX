//! `packages/business/src/audit/log-query.ts` `listAuditLogs` (workspace
//! scope) and `parseAuditLogsDateRange` of the audit-logs page schema.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{AuditLog, User};
use chatbotx_db::repo::utils::{SortDir, order_by_sql, parse_order_by};
use chatbotx_db::repo::{
    Error as RepoError, Model, PaginationInput, SortItem, get_pagination_with_defaults,
    like_contains,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension as _, params_from_iter};
use chrono::{Days, NaiveDate};

use crate::errors::Result;
use crate::tx;

const MAX_RANGE_DAYS: u64 = 90;

/// Input of [`list_logs`]; `from` / `to` are `YYYY-MM-DD` keys.
#[derive(Debug, Clone, Default)]
pub struct ListAuditLogsInput {
    pub workspace_id: i64,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
    pub keyword: Option<String>,
    pub user_id: Option<i64>,
    pub from: String,
    pub to: String,
}

pub struct AuditLogRow {
    pub log: AuditLog,
    pub user: Option<User>,
}

pub struct AuditLogPage {
    pub data: Vec<AuditLogRow>,
    pub page_count: i64,
}

/// `parseAuditLogsDateRange`: `(start, end)` in ms, at most 90 days ending
/// today (UTC) at the latest.
pub fn date_range(from: &str, to: &str, today: NaiveDate) -> (i64, i64) {
    let parse = |s: &str| {
        (s.len() == 10)
            .then(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .flatten()
    };
    let window = Days::new(MAX_RANGE_DAYS - 1);
    let fallback_from = today - window;
    let mut from = parse(from).unwrap_or(fallback_from);
    let mut to = parse(to).unwrap_or(today).min(today);
    if from > to {
        from = fallback_from;
        to = today;
    }
    from = from.max(to - window);
    let ms = |d: NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .map_or(0, |t| t.and_utc().timestamp_millis())
    };
    (ms(from), ms(to) + 86_400_000 - 1)
}

/// `listAuditLogs`: always scoped by the workspace; order by the sort, then
/// `id desc`.
pub async fn list_logs(db: &Db, input: ListAuditLogsInput) -> Result<AuditLogPage> {
    let today = chrono::DateTime::from_timestamp_millis(chatbotx_db::repo::now_ms())
        .map(|now| now.date_naive())
        .unwrap_or_default();
    let (start, end) = date_range(&input.from, &input.to, today);
    tx::read(db, move |conn| {
        let mut filter =
            String::from(r#""workspaceId" = ? AND "createdAt" >= ? AND "createdAt" <= ?"#);
        let mut binds = vec![
            Sql::Integer(input.workspace_id),
            Sql::Integer(start),
            Sql::Integer(end),
        ];
        if let Some(user_id) = input.user_id {
            filter.push_str(r#" AND "userId" = ?"#);
            binds.push(Sql::Integer(user_id));
        }
        if let Some(keyword) = input.keyword.as_deref().filter(|k| !k.is_empty()) {
            filter.push_str(
                r#" AND (ilike("action", ?) OR ilike("detail", ?) OR ilike("ipAddress", ?))"#,
            );
            let pattern = like_contains(keyword);
            binds.extend(std::iter::repeat_n(Sql::Text(pattern), 3));
        }
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let mut order = parse_order_by(AuditLog::COLUMNS, input.sort.as_deref(), None);
        order.retain(|(column, _)| column != "id");
        order.push(("id".to_string(), SortDir::Desc));
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "AuditLog" WHERE {filter}"#),
                params_from_iter(binds.iter()),
                |row| row.get(0),
            )
            .map_err(RepoError::from)?;
        binds.push(Sql::Integer(pagination.limit));
        binds.push(Sql::Integer(pagination.offset));
        let logs = conn
            .prepare(&format!(
                "{} WHERE {filter} {} LIMIT ? OFFSET ?",
                AuditLog::select_sql(),
                order_by_sql(&order, None)
            ))
            .map_err(RepoError::from)?
            .query_map(params_from_iter(binds.iter()), AuditLog::from_row)
            .map_err(RepoError::from)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(RepoError::from)?;
        let mut data = Vec::with_capacity(logs.len());
        for log in logs {
            let user = match log.user_id {
                Some(id) => conn
                    .query_row(
                        &format!(r#"{} WHERE "id" = ?1"#, User::select_sql()),
                        [id],
                        User::from_row,
                    )
                    .optional()
                    .map_err(RepoError::from)?,
                None => None,
            };
            data.push(AuditLogRow { log, user });
        }
        let limit = pagination.limit.max(1);
        Ok(AuditLogPage {
            data,
            page_count: (total + limit - 1) / limit,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_range_clamps_like_the_page_schema() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let day = |s: &str| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis()
        };
        // Valid range kept; end is the last ms of `to`.
        assert_eq!(
            date_range("2026-11-01", "2026-11-10", today),
            (day("2026-11-01"), day("2026-11-11") - 1)
        );
        // `to` in the future → today; invalid `from` → today - 89 days.
        assert_eq!(
            date_range("nope", "2027-01-01", today),
            (day("2026-08-20"), day("2026-11-18") - 1)
        );
        // Longer than 90 days → `from` moves up.
        assert_eq!(date_range("2025-01-01", "2026-11-17", today).0, day("2026-08-20"));
        // `from` after `to` → the default range.
        assert_eq!(date_range("2026-11-10", "2026-11-01", today).0, day("2026-08-20"));
    }
}
