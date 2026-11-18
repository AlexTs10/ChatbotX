//! `packages/business/src/coupon/service.ts` + `couponRepository`: coupon
//! topics (pools of single-use codes) and their coupons, issued one per
//! contact.

use chatbotx_db::repo::models::{Coupon, CouponTopic};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::utils::like_contains;
use chatbotx_db::repo::{find_one, iso, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

pub const MAX_COUPONS_PER_TOPIC: i64 = 10_000;
pub const MAX_COUPON_IMPORT_BYTES: i64 = 10 * 1024 * 1024;

fn exception(message: &str, code: &str) -> BusinessError {
    BusinessError::exception(message, code, 400)
}

fn topic_not_found() -> BusinessError {
    BusinessError::not_found("Coupon topic not found")
}

fn id_text(id: Option<i64>) -> Value {
    id.map_or(Value::Null, |id| Value::String(id.to_string()))
}

/// `publicCouponTopicResource`.
pub fn topic_json(t: &CouponTopic) -> Value {
    json!({
        "id": t.id.to_string(),
        "workspaceId": t.workspace_id.to_string(),
        "name": t.name,
        "description": t.description,
        "expiresAt": t.expires_at.map(iso),
        "status": t.status,
        "hasEverHadCoupon": t.has_ever_had_coupon,
        "createdAt": iso(t.created_at),
        "updatedAt": iso(t.updated_at),
    })
}

/// `publicIssuedCouponResource` (the bare row).
pub fn coupon_json(c: &Coupon) -> Value {
    json!({
        "id": c.id.to_string(),
        "workspaceId": c.workspace_id.to_string(),
        "topicId": c.topic_id.to_string(),
        "code": c.code,
        "issuedContactId": id_text(c.issued_contact_id),
        "issuedAt": c.issued_at.map(iso),
        "usedAt": c.used_at.map(iso),
        "createdAt": iso(c.created_at),
        "updatedAt": iso(c.updated_at),
    })
}

/// `couponResource`: the row with its topic name and derived statuses.
pub fn listed_json(c: &Coupon, topic_name: &str) -> Value {
    let mut row = coupon_json(c);
    row["topicName"] = json!(topic_name);
    row["issueStatus"] = json!(if c.issued_contact_id.is_some() {
        "published"
    } else {
        "unpublished"
    });
    row["usageStatus"] = json!(if c.used_at.is_some() {
        "used"
    } else {
        "notUsed"
    });
    row
}

fn page_count(total: i64, limit: i64) -> i64 {
    (total + limit - 1) / limit
}

fn trim_nullable(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

// ---- topics ------------------------------------------------------------------

/// `listTopics` (active, or `archived`), by name, with each coupon count.
pub async fn list_topics(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    archived: bool,
    search: Option<String>,
    page: i64,
    per_page: i64,
) -> Result<(Vec<Value>, i64)> {
    tx::read(db, move |conn| {
        let mut where_sql = Sql::raw(r#"WHERE t."workspaceId" = "#);
        where_sql.push_bind(workspace_id);
        where_sql.push(r#" AND t."deletedAt" IS NULL AND t."status" = "#);
        where_sql.push_bind(if archived { "archived" } else { "active" });
        if let Some(search) = search.filter(|s| !s.is_empty()) {
            where_sql.push(r#" AND ilike(t."name", "#);
            where_sql.push_bind(like_contains(&search));
            where_sql.push(")");
        }
        let mut count = Sql::raw(r#"SELECT count(*) FROM "CouponTopic" t "#);
        count.append(where_sql.clone());
        let total: i64 = count.query_map(conn, |r| r.get(0))?.first().copied().unwrap_or(0);
        let mut select = Sql::raw(
            r#"SELECT t."id", (SELECT count(*) FROM "Coupon" c WHERE c."topicId" = t."id" AND c."workspaceId" = t."workspaceId") FROM "CouponTopic" t "#,
        );
        select.append(where_sql);
        select.push(&format!(
            r#" ORDER BY t."name" ASC LIMIT {per_page} OFFSET {}"#,
            (page - 1) * per_page
        ));
        let ids: Vec<(i64, i64)> = select.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut data = Vec::with_capacity(ids.len());
        for (id, coupons) in ids {
            if let Some(topic) = find_one::<CouponTopic>(conn, r#""id" = ?1"#, &[&id])? {
                let mut row = topic_json(&topic);
                row["couponCount"] = json!(coupons);
                data.push(row);
            }
        }
        Ok((data, page_count(total, per_page)))
    })
    .await
}

fn find_topic(conn: &Connection, workspace_id: i64, topic_id: i64) -> Result<Option<CouponTopic>> {
    Ok(find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
        &[&topic_id, &workspace_id],
    )?)
}

/// `getTopic`.
pub async fn get_topic(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    topic_id: i64,
) -> Result<CouponTopic> {
    tx::read(db, move |conn| {
        find_topic(conn, workspace_id, topic_id)?.ok_or_else(topic_not_found)
    })
    .await
}

fn assert_unique_name(
    conn: &Connection,
    workspace_id: i64,
    name: &str,
    except: Option<i64>,
) -> Result<()> {
    let taken: Option<i64> = conn
        .query_row(
            r#"SELECT "id" FROM "CouponTopic" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL
               AND lower("name") = lower(?2) AND (?3 IS NULL OR "id" <> ?3) LIMIT 1"#,
            params![workspace_id, name, except],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))?;
    if taken.is_some() {
        return Err(exception(
            "Coupon topic name already exists",
            "couponTopicNameDuplicated",
        ));
    }
    Ok(())
}

/// `endOfSelectedDayInTimezone`: 23:59:59.999 of that day where the
/// workspace is.
pub fn end_of_day_in(at_ms: i64, timezone: &str) -> Result<i64> {
    let invalid = || exception("Invalid validity date", "invalidValidityDate");
    let tz: chrono_tz::Tz = timezone.parse().map_err(|_| invalid())?;
    let day = Utc
        .timestamp_millis_opt(at_ms)
        .single()
        .ok_or_else(invalid)?
        .with_timezone(&tz)
        .date_naive();
    let end = day.and_hms_milli_opt(23, 59, 59, 999).ok_or_else(invalid)?;
    let zoned = tz
        .from_local_datetime(&end)
        .earliest()
        .ok_or_else(invalid)?;
    Ok(zoned.timestamp_millis())
}

fn normalize_expires_at(
    conn: &Connection,
    workspace_id: i64,
    expires_at: Option<i64>,
    reject_past: bool,
) -> Result<Option<i64>> {
    let timezone: Option<Option<String>> = conn
        .query_row(
            r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            params![workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))?;
    let Some(timezone) = timezone else {
        return Err(BusinessError::not_found("Workspace not found"));
    };
    let Some(at) = expires_at else {
        return Ok(None);
    };
    let end = end_of_day_in(at, timezone.as_deref().unwrap_or("UTC"))?;
    if reject_past && end <= now_ms() {
        return Err(exception(
            "Coupon topic validity cannot be in the past",
            "couponTopicValidityInPast",
        ));
    }
    Ok(Some(end))
}

/// `createTopic` (the route has validated name and description).
pub async fn create_topic(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    name: String,
    description: Option<String>,
    expires_at: Option<i64>,
) -> Result<CouponTopic> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let name = name.trim().to_string();
        let description = trim_nullable(description.as_deref());
        assert_unique_name(conn, workspace_id, &name, None)?;
        let expires_at = normalize_expires_at(conn, workspace_id, expires_at, true)?;
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "CouponTopic" ("id", "createdAt", "updatedAt", "workspaceId", "name", "description", "expiresAt", "status", "hasEverHadCoupon", "createdById")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 'active', 0, NULL)"#,
            params![id, now, workspace_id, name, description, expires_at],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        find_topic(conn, workspace_id, id)?.ok_or_else(topic_not_found)
    })
    .await
}

/// `updateTopic`: `None` keeps a field, `Some(None)` clears it. Name and
/// description stay once the topic ever had a coupon.
pub async fn update_topic(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    topic_id: i64,
    name: Option<String>,
    description: Option<Option<String>>,
    expires_at: Option<Option<i64>>,
) -> Result<CouponTopic> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let existing = find_topic(conn, workspace_id, topic_id)?.ok_or_else(topic_not_found)?;
        let locked = existing.has_ever_had_coupon;
        let name = name
            .filter(|_| !locked)
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        let description = description
            .filter(|_| !locked)
            .map(|d| trim_nullable(d.as_deref()));
        if let Some(name) = &name {
            assert_unique_name(conn, workspace_id, name, Some(topic_id))?;
        }
        let expires_at = match expires_at {
            Some(at) => Some(normalize_expires_at(conn, workspace_id, at, false)?),
            None => None,
        };
        conn.execute(
            r#"UPDATE "CouponTopic" SET
                 "name" = COALESCE(?3, "name"),
                 "description" = CASE WHEN ?4 THEN ?5 ELSE "description" END,
                 "expiresAt" = CASE WHEN ?6 THEN ?7 ELSE "expiresAt" END,
                 "updatedAt" = ?8
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            params![
                topic_id,
                workspace_id,
                name,
                description.is_some(),
                description.flatten(),
                expires_at.is_some(),
                expires_at.flatten(),
                now_ms(),
            ],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        find_topic(conn, workspace_id, topic_id)?.ok_or_else(topic_not_found)
    })
    .await
}

/// `archiveTopic` / `unarchiveTopic`.
pub async fn set_topic_status(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    topic_id: i64,
    status: &'static str,
) -> Result<CouponTopic> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        conn.execute(
            r#"UPDATE "CouponTopic" SET "status" = ?3, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            params![topic_id, workspace_id, status, now_ms()],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        find_topic(conn, workspace_id, topic_id)?.ok_or_else(topic_not_found)
    })
    .await
}

/// `deleteTopic`: a soft delete of an archived topic.
pub async fn delete_topic(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    topic_id: i64,
) -> Result<CouponTopic> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let topic = find_topic(conn, workspace_id, topic_id)?
            .filter(|t| t.status == "archived")
            .ok_or_else(|| BusinessError::not_found("Archived coupon topic not found"))?;
        conn.execute(
            r#"UPDATE "CouponTopic" SET "deletedAt" = ?3, "updatedAt" = ?3 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![topic_id, workspace_id, now_ms()],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(topic)
    })
    .await
}

// ---- coupons -----------------------------------------------------------------

/// `CouponListInput` without the workspace and paging.
#[derive(Debug, Clone, Default)]
pub struct CouponFilter {
    pub topic_id: Option<i64>,
    pub issue_status: Option<String>,
    pub usage_status: Option<String>,
    pub search: Option<String>,
}

/// `couponListWhere` (live, active topics only).
fn coupon_where(workspace_id: i64, filter: &CouponFilter) -> Sql {
    let mut sql = Sql::raw(
        r#"FROM "Coupon" c JOIN "CouponTopic" t ON t."id" = c."topicId" AND t."workspaceId" = "#,
    );
    sql.push_bind(workspace_id);
    sql.push(r#" AND t."status" = 'active' AND t."deletedAt" IS NULL WHERE c."workspaceId" = "#);
    sql.push_bind(workspace_id);
    if let Some(id) = filter.topic_id {
        sql.push(r#" AND c."topicId" = "#);
        sql.push_bind(id);
    }
    if let Some(search) = filter.search.as_deref().filter(|s| !s.is_empty()) {
        sql.push(r#" AND ilike(c."code", "#);
        sql.push_bind(like_contains(search));
        sql.push(")");
    }
    match filter.issue_status.as_deref() {
        Some("published") => sql.push(r#" AND c."issuedContactId" IS NOT NULL"#),
        Some("unpublished") => sql.push(r#" AND c."issuedContactId" IS NULL"#),
        _ => {}
    }
    match filter.usage_status.as_deref() {
        Some("used") => sql.push(r#" AND c."usedAt" IS NOT NULL"#),
        Some("notUsed") => sql.push(r#" AND c."usedAt" IS NULL"#),
        _ => {}
    }
    sql
}

fn coupon_rows(conn: &Connection, sql: Sql) -> Result<Vec<(Coupon, String)>> {
    let mut select = Sql::raw(
        r#"SELECT c."id", c."createdAt", c."updatedAt", c."workspaceId", c."topicId", c."code", c."issuedContactId", c."issuedAt", c."usedAt", t."name" "#,
    );
    select.append(sql);
    Ok(select.query_map(conn, |r| {
        Ok((
            Coupon {
                id: r.get(0)?,
                created_at: r.get(1)?,
                updated_at: r.get(2)?,
                workspace_id: r.get(3)?,
                topic_id: r.get(4)?,
                code: r.get(5)?,
                issued_contact_id: r.get(6)?,
                issued_at: r.get(7)?,
                used_at: r.get(8)?,
            },
            r.get(9)?,
        ))
    })?)
}

/// `listCoupons`: oldest first.
pub async fn list_coupons(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    filter: CouponFilter,
    page: i64,
    per_page: i64,
) -> Result<(Vec<Value>, i64)> {
    tx::read(db, move |conn| {
        let mut count = Sql::raw("SELECT count(*) ");
        count.append(coupon_where(workspace_id, &filter));
        let total: i64 = count
            .query_map(conn, |r| r.get(0))?
            .first()
            .copied()
            .unwrap_or(0);
        let mut rows = coupon_where(workspace_id, &filter);
        rows.push(&format!(
            r#" ORDER BY c."createdAt" ASC, c."id" ASC LIMIT {per_page} OFFSET {}"#,
            (page - 1) * per_page
        ));
        let data = coupon_rows(conn, rows)?
            .iter()
            .map(|(c, name)| listed_json(c, name))
            .collect();
        Ok((data, page_count(total, per_page)))
    })
    .await
}

/// `listCouponsForExport`: every matching coupon, by id.
pub fn list_for_export(
    conn: &Connection,
    workspace_id: i64,
    filter: &CouponFilter,
) -> Result<Vec<(Coupon, String)>> {
    let mut rows = coupon_where(workspace_id, filter);
    rows.push(r#" ORDER BY c."id" ASC"#);
    coupon_rows(conn, rows)
}

/// `importBatch`'s result.
pub fn batch_json(
    processed: usize,
    created: usize,
    existing: usize,
    allowed: i64,
    current: i64,
) -> Value {
    json!({
        "processed": processed,
        "created": created,
        "existing": existing,
        "allowedRemaining": allowed,
        "currentCount": current,
    })
}

/// `importBatch` in a write transaction: the new codes of an active topic,
/// within its limit; codes taken anywhere in the workspace are skipped.
pub fn import_batch_in(
    conn: &Connection,
    workspace_id: i64,
    topic_id: i64,
    codes: &[String],
) -> Result<Value> {
    let mut unique: Vec<String> = Vec::new();
    for code in codes.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        if !unique.iter().any(|u| u == code) {
            unique.push(code.to_string());
        }
    }
    let topic = find_topic(conn, workspace_id, topic_id)?;
    if topic.is_none_or(|t| t.status != "active") {
        return Err(exception(
            "Coupon topic is not active",
            "couponTopicInactive",
        ));
    }
    let current: i64 = conn
        .query_row(
            r#"SELECT count(*) FROM "Coupon" WHERE "workspaceId" = ?1 AND "topicId" = ?2"#,
            params![workspace_id, topic_id],
            |r| r.get(0),
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
    let allowed = (MAX_COUPONS_PER_TOPIC - current).max(0);
    let mut taken = conn
        .prepare(r#"SELECT 1 FROM "Coupon" WHERE "workspaceId" = ?1 AND "code" = ?2"#)
        .map_err(|e| BusinessError::Repo(e.into()))?;
    let mut new_codes = Vec::new();
    for code in &unique {
        if !taken
            .exists(params![workspace_id, code])
            .map_err(|e| BusinessError::Repo(e.into()))?
        {
            new_codes.push(code.clone());
        }
    }
    if new_codes.len() as i64 > allowed {
        return Err(exception(
            "Coupon import exceeds topic limit",
            "couponImportLimitExceeded",
        ));
    }
    let now = now_ms();
    let mut inserted = 0;
    for code in &new_codes {
        inserted += conn
            .execute(
                r#"INSERT INTO "Coupon" ("id", "createdAt", "updatedAt", "workspaceId", "topicId", "code")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING"#,
                params![new_id(), now, workspace_id, topic_id, code],
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
    }
    if inserted > 0 {
        conn.execute(
            r#"UPDATE "CouponTopic" SET "hasEverHadCoupon" = 1 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![topic_id, workspace_id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
    }
    Ok(batch_json(
        unique.len(),
        inserted,
        unique.len() - inserted,
        allowed,
        current,
    ))
}

/// `importBatch`.
pub async fn import_batch(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    topic_id: i64,
    codes: Vec<String>,
) -> Result<Value> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        import_batch_in(conn, workspace_id, topic_id, &codes)
    })
    .await
}

fn find_issued(
    conn: &Connection,
    workspace_id: i64,
    topic_id: i64,
    contact_id: i64,
) -> Result<Option<Coupon>> {
    Ok(find_one(
        conn,
        r#""workspaceId" = ?1 AND "topicId" = ?2 AND "issuedContactId" = ?3"#,
        &[&workspace_id, &topic_id, &contact_id],
    )?)
}

fn issue_error(message: &str, code: &str, status: u16, reason: &str) -> BusinessError {
    BusinessError::Exception {
        message: message.into(),
        code: code.into(),
        status,
        field: None,
        data: Some(json!({ "reason": reason })),
    }
}

/// `issueCoupon`: the contact's coupon of this topic, or the next free one.
/// Refused (`couponIssueUnavailable`, an undeclared 500 as the original) when the topic is not issuable or
/// has no free coupon.
pub async fn issue(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    topic_id: i64,
    contact_id: i64,
) -> Result<(Coupon, &'static str)> {
    let refused = |reason: &str| {
        issue_error(
            "No coupon could be issued for this topic",
            "couponIssueUnavailable",
            500,
            reason,
        )
    };
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        // A token-supplied contact must be this workspace's.
        crate::contact::service::find_or_fail(conn, workspace_id, contact_id)?;
        let now = now_ms();
        let issuable = find_topic(conn, workspace_id, topic_id)?
            .is_some_and(|t| t.status == "active" && t.expires_at.is_none_or(|at| at > now));
        if !issuable {
            return Err(refused("topicUnavailable"));
        }
        if let Some(existing) = find_issued(conn, workspace_id, topic_id, contact_id)? {
            return Ok((existing, "existing"));
        }
        let candidate: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "Coupon" WHERE "workspaceId" = ?1 AND "topicId" = ?2 AND "issuedContactId" IS NULL ORDER BY "id" LIMIT 1"#,
                params![workspace_id, topic_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let Some(id) = candidate else {
            return Err(refused("noAvailableCoupon"));
        };
        conn.execute(
            r#"UPDATE "Coupon" SET "issuedContactId" = ?2, "issuedAt" = ?3, "updatedAt" = ?3 WHERE "id" = ?1"#,
            params![id, contact_id, now],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        let claimed = find_issued(conn, workspace_id, topic_id, contact_id)?
            .ok_or_else(|| refused("noAvailableCoupon"))?;
        Ok((claimed, "issued"))
    })
    .await
}

/// `markCouponUsed`: refused (`couponNotIssued`, an undeclared 500) without an issued
/// coupon; a used one stays as it is.
pub async fn mark_used(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    topic_id: i64,
    contact_id: i64,
) -> Result<Coupon> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        crate::contact::service::find_or_fail(conn, workspace_id, contact_id)?;
        let Some(existing) = find_issued(conn, workspace_id, topic_id, contact_id)? else {
            return Err(issue_error(
                "No coupon has been issued to this contact for this topic",
                "couponNotIssued",
                500,
                "noIssuedCoupon",
            ));
        };
        if existing.used_at.is_some() {
            return Ok(existing);
        }
        let now = now_ms();
        conn.execute(
            r#"UPDATE "Coupon" SET "usedAt" = ?2, "updatedAt" = ?2 WHERE "id" = ?1 AND "usedAt" IS NULL"#,
            params![existing.id, now],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(find_issued(conn, workspace_id, topic_id, contact_id)?.unwrap_or(existing))
    })
    .await
}

/// `listIssuedCouponsForContact`: by topic name, live topics only.
pub async fn list_for_contact(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(
                r#"SELECT c."id", c."topicId", t."name", c."code", c."usedAt", c."issuedAt"
                   FROM "Coupon" c JOIN "CouponTopic" t ON t."id" = c."topicId" AND t."workspaceId" = ?1 AND t."deletedAt" IS NULL
                   WHERE c."workspaceId" = ?1 AND c."issuedContactId" = ?2
                   ORDER BY t."name" ASC"#,
            )
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let rows = stmt
            .query_map(params![workspace_id, contact_id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?.to_string(),
                    "topicId": r.get::<_, i64>(1)?.to_string(),
                    "topicName": r.get::<_, String>(2)?,
                    "code": r.get::<_, String>(3)?,
                    "usedAt": r.get::<_, Option<i64>>(4)?.map(iso),
                    "issuedAt": r.get::<_, Option<i64>>(5)?.map(iso),
                }))
            })
            .map_err(|e| BusinessError::Repo(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(rows)
    })
    .await
}

/// `resolveCouponVariable`: the contact's code of this topic, or "".
pub fn issued_code(
    conn: &Connection,
    workspace_id: i64,
    topic_id: i64,
    contact_id: i64,
) -> Result<String> {
    Ok(find_issued(conn, workspace_id, topic_id, contact_id)?
        .map(|c| c.code)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_of_day_follows_the_time_zone() {
        // 2026-03-10T03:00Z is still 2026-03-09 in New York (EDT, UTC-4).
        let at = chrono::DateTime::parse_from_rfc3339("2026-03-10T03:00:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            iso(end_of_day_in(at, "UTC").unwrap()),
            "2026-03-10T23:59:59.999Z"
        );
        assert_eq!(
            iso(end_of_day_in(at, "America/New_York").unwrap()),
            "2026-03-10T03:59:59.999Z"
        );
        assert!(end_of_day_in(at, "Mars/Base").is_err());
    }
}
