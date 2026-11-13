//! Port of `repositories/order/repository.ts` (`orderRepository`; no TS tests, smoke test
//! `tests/repo_p33_smoke.rs`). Money columns are x 10,000 integers. `findForUpdate` /
//! `findByExternalIdForUpdate` are plain reads under the one SQLite writer. `update` sets
//! `updatedAt` itself (TS); the other UPDATEs set it as Drizzle `$onUpdate` does.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{Order, OrderItem, OrderPayment, OrderStatusHistory};
use super::patch::{self, Patch};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortDir, SortItem, like_contains, order_by_sql, parse_order_by,
    parse_pagination,
};
use super::{Model, Result, find_many, find_one, new_id, now_ms, returning, text_list};

/// `OrderListInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderListInput {
    pub workspace_id: i64,
    pub status: Vec<String>,
    pub source: Vec<String>,
    pub payment_status: Vec<String>,
    pub fulfillment_status: Vec<String>,
    pub contact_id: Option<i64>,
    /// `createdAt >= from` (unix ms).
    pub from: Option<i64>,
    /// `createdAt < to` (unix ms).
    pub to: Option<i64>,
    pub keyword: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// One row of `list`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderListRow {
    pub order: Order,
    /// Sum of the item quantities.
    pub item_count: i64,
}

/// Result of `list`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderListPage {
    pub data: Vec<OrderListRow>,
    pub page_count: i64,
}

/// One history row of `findDetail` with its actor user (`{ id, name }`).
#[derive(Debug, Clone, PartialEq)]
pub struct OrderHistoryWithActor {
    pub history: OrderStatusHistory,
    pub actor_user: Option<(i64, Option<String>)>,
}

/// Result of `findDetail`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderDetail {
    pub order: Order,
    pub items: Vec<OrderItem>,
    pub history: Vec<OrderHistoryWithActor>,
    pub payments: Vec<OrderPayment>,
}

/// Result of `findByExternalId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderIdStatus {
    pub id: i64,
    pub status: String,
}

fn in_texts(column: &str, items: &[String]) -> Option<Sql> {
    (!items.is_empty()).then(|| {
        Sql::new(
            format!("\"{column}\" IN (SELECT value FROM json_each(?))"),
            vec![Value::Text(text_list(items))],
        )
    })
}

fn one(text: &str, value: Value) -> Sql {
    Sql::new(text, vec![value])
}

/// `buildListWhere`.
fn list_where(input: &OrderListInput) -> Sql {
    let keyword = input
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    let mut parts = vec![
        Some(one(
            "\"workspaceId\" = ?",
            Value::Integer(input.workspace_id),
        )),
        in_texts("status", &input.status),
        in_texts("source", &input.source),
        in_texts("paymentStatus", &input.payment_status),
        in_texts("fulfillmentStatus", &input.fulfillment_status),
        input
            .contact_id
            .map(|c| one("\"contactId\" = ?", Value::Integer(c))),
        input
            .from
            .map(|f| one("\"createdAt\" >= ?", Value::Integer(f))),
        input
            .to
            .map(|t| one("\"createdAt\" < ?", Value::Integer(t))),
    ];
    if let Some(keyword) = keyword {
        let digits = keyword.strip_prefix('#').unwrap_or(keyword);
        let number = (!digits.is_empty()
            && digits.len() <= 15
            && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse::<i64>().ok())
        .flatten();
        let pattern = like_contains(keyword);
        parts.push(Some(Sql::or([
            number.map(|n| one("\"number\" = ?", Value::Integer(n))),
            Some(one("\"externalName\" = ?", Value::Text(keyword.to_owned()))),
            Some(one(
                "ilike(\"customerName\", ?)",
                Value::Text(pattern.clone()),
            )),
            Some(one(
                "ilike(\"customerPhone\", ?)",
                Value::Text(pattern.clone()),
            )),
            Some(one("ilike(\"customerEmail\", ?)", Value::Text(pattern))),
        ])));
    }
    Sql::and(parts)
}

/// `list`: a filtered page of orders with their item count, and the page count.
pub fn list(conn: &Connection, input: &OrderListInput) -> Result<OrderListPage> {
    let where_sql = list_where(input);
    let pagination = parse_pagination(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let mut order_by = parse_order_by(Order::COLUMNS, input.sort.as_deref(), None);
    if order_by.is_empty() {
        order_by = vec![("number".to_owned(), SortDir::Desc)];
    }
    let mut sql = Sql::raw(format!(
        "SELECT {}, (SELECT coalesce(sum(i.\"quantity\"), 0) FROM \"OrderItem\" i WHERE i.\"orderId\" = \"Order\".\"id\") FROM \"Order\" WHERE ",
        Order::columns_sql(Some("\"Order\""))
    ));
    sql.append(where_sql.clone());
    sql.push(&format!(" {}", order_by_sql(&order_by, Some("\"Order\""))));
    if let Some(p) = pagination {
        sql.push(" LIMIT ");
        sql.push_bind(p.limit);
        sql.push(" OFFSET ");
        sql.push_bind(p.offset);
    }
    let n = Order::COLUMNS.len();
    let data = sql.query_map(conn, |row| {
        Ok(OrderListRow {
            order: Order::from_row(row)?,
            item_count: row.get(n)?,
        })
    })?;
    let mut count = Sql::raw("SELECT count(*) FROM \"Order\" WHERE ");
    count.append(where_sql);
    let total: i64 = count
        .query_map(conn, |row| row.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let page_count = match pagination {
        Some(p) if p.limit > 0 => (total + p.limit - 1) / p.limit,
        _ => 1,
    };
    Ok(OrderListPage { data, page_count })
}

/// `findDetail`: the order with its items, history (with actor user) and payments.
pub fn find_detail(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
) -> Result<Option<OrderDetail>> {
    let Some(order) = find(conn, workspace_id, order_id)? else {
        return Ok(None);
    };
    let items = find_many::<OrderItem>(
        conn,
        "WHERE \"orderId\" = ?1 ORDER BY \"id\" ASC",
        &[&order_id],
    )?;
    let rows = find_many::<OrderStatusHistory>(
        conn,
        "WHERE \"orderId\" = ?1 ORDER BY \"createdAt\" ASC, \"id\" ASC",
        &[&order_id],
    )?;
    let mut history = Vec::with_capacity(rows.len());
    for h in rows {
        let actor_user = match h.actor_user_id {
            Some(user_id) => Sql::new(
                "SELECT \"id\", \"name\" FROM \"User\" WHERE \"id\" = ?",
                vec![Value::Integer(user_id)],
            )
            .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?
            .into_iter()
            .next(),
            None => None,
        };
        history.push(OrderHistoryWithActor {
            history: h,
            actor_user,
        });
    }
    let payments = find_many::<OrderPayment>(
        conn,
        "WHERE \"orderId\" = ?1 ORDER BY \"createdAt\" DESC, \"id\" DESC",
        &[&order_id],
    )?;
    Ok(Some(OrderDetail {
        order,
        items,
        history,
        payments,
    }))
}

fn update_returning(conn: &Connection, set: &Patch, where_sql: Sql) -> Result<Option<Order>> {
    let mut sql = Sql::raw("UPDATE \"Order\" SET ");
    sql.append(patch::set_clause(Order::COLUMNS, set)?);
    sql.push(" WHERE ");
    sql.append(where_sql);
    sql.push(&format!(" RETURNING {}", Order::columns_sql(None)));
    sql.model::<Order>(conn)
}

fn scope(workspace_id: i64, order_id: i64) -> Sql {
    Sql::new(
        "\"id\" = ? AND \"workspaceId\" = ?",
        vec![Value::Integer(order_id), Value::Integer(workspace_id)],
    )
}

fn with_updated_at(values: &Patch) -> Patch {
    let mut set = values.clone();
    if !set.iter().any(|(c, _)| *c == "updatedAt") {
        set.push(("updatedAt", Value::Integer(now_ms())));
    }
    set
}

/// `markPaid`: an unpaid, not cancelled order becomes paid.
pub fn mark_paid(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    paid_at: i64,
) -> Result<Option<Order>> {
    let mut where_sql = scope(workspace_id, order_id);
    where_sql.push(" AND \"paymentStatus\" = 'unpaid' AND \"status\" <> 'cancelled'");
    update_returning(
        conn,
        &with_updated_at(&vec![
            ("paymentStatus", Value::Text("paid".to_owned())),
            ("paidAt", Value::Integer(paid_at)),
            ("paymentExpiresAt", Value::Null),
        ]),
        where_sql,
    )
}

/// `find`.
pub fn find(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<Option<Order>> {
    find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&order_id, &workspace_id],
    )
}

/// `findForUpdate`: the order row (the write job is the lock).
pub fn find_for_update(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
) -> Result<Option<Order>> {
    find(conn, workspace_id, order_id)
}

/// `findByExternalId`: `{ id, status }`.
pub fn find_by_external_id(
    conn: &Connection,
    workspace_id: i64,
    external_id: &str,
) -> Result<Option<OrderIdStatus>> {
    Ok(Sql::new(
        "SELECT \"id\", \"status\" FROM \"Order\" WHERE \"workspaceId\" = ? AND \"externalId\" = ? LIMIT 1",
        vec![Value::Integer(workspace_id), Value::Text(external_id.to_owned())],
    )
    .query_map(conn, |row| {
        Ok(OrderIdStatus {
            id: row.get(0)?,
            status: row.get(1)?,
        })
    })?
    .into_iter()
    .next())
}

/// `findByExternalIdForUpdate`: the order row (the write job is the lock).
pub fn find_by_external_id_for_update(
    conn: &Connection,
    workspace_id: i64,
    external_id: &str,
) -> Result<Option<Order>> {
    find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"externalId\" = ?2",
        &[&workspace_id, &external_id],
    )
}

/// `update`: the patch plus `updatedAt`.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    values: &Patch,
) -> Result<Option<Order>> {
    update_returning(
        conn,
        &with_updated_at(values),
        scope(workspace_id, order_id),
    )
}

/// `deleteItems`.
pub fn delete_items(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"OrderItem\" WHERE \"orderId\" = ?1 AND \"workspaceId\" = ?2",
        (order_id, workspace_id),
    )?;
    Ok(())
}

fn insert_patch(table: &str, columns: &[&str], values: &Patch) -> Result<Sql> {
    let mut all: Patch = vec![("id", Value::Integer(new_id()))];
    all.extend(values.iter().cloned());
    let mut sql = Sql::raw(format!("INSERT INTO \"{table}\" "));
    sql.append(patch::insert_clause(columns, &all)?);
    Ok(sql)
}

/// `insert` (`values` = the order columns except the id and timestamps).
pub fn insert(conn: &Connection, values: &Patch) -> Result<Option<Order>> {
    let mut sql = insert_patch("Order", Order::COLUMNS, values)?;
    sql.push(&format!(" RETURNING {}", Order::columns_sql(None)));
    sql.model::<Order>(conn)
}

/// `delete`: the deleted row, if any.
pub fn delete(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<Option<Order>> {
    Ok(returning::<Order>(
        conn,
        "DELETE FROM \"Order\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&order_id, &workspace_id],
    )?
    .into_iter()
    .next())
}

/// `insertItems`: each item is the item columns except the id, order and workspace.
pub fn insert_items(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    items: &[Patch],
) -> Result<Vec<OrderItem>> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let mut values: Patch = vec![
            ("orderId", Value::Integer(order_id)),
            ("workspaceId", Value::Integer(workspace_id)),
        ];
        values.extend(item.iter().cloned());
        let mut sql = insert_patch("OrderItem", OrderItem::COLUMNS, &values)?;
        sql.push(&format!(" RETURNING {}", OrderItem::columns_sql(None)));
        out.extend(sql.model::<OrderItem>(conn)?);
    }
    Ok(out)
}

/// `insertHistory` (`values` = the history columns except the id and timestamps).
pub fn insert_history(conn: &Connection, values: &Patch) -> Result<()> {
    insert_patch("OrderStatusHistory", OrderStatusHistory::COLUMNS, values)?.execute(conn)?;
    Ok(())
}

/// `transitionStatus`: moves the order to `to` only from one of `from`.
pub fn transition_status(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    from: &[String],
    to: &str,
    values: &Patch,
) -> Result<Option<Order>> {
    if from.is_empty() {
        return Ok(None);
    }
    let mut set = values.clone();
    set.push(("status", Value::Text(to.to_owned())));
    let mut where_sql = scope(workspace_id, order_id);
    where_sql.push(" AND \"status\" IN (SELECT value FROM json_each(");
    where_sql.push_bind(text_list(from));
    where_sql.push("))");
    update_returning(conn, &with_updated_at(&set), where_sql)
}

/// `setRating`.
pub fn set_rating(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    rating: i64,
    rating_comment: Option<&str>,
) -> Result<Option<Order>> {
    update_returning(
        conn,
        &with_updated_at(&vec![
            ("rating", Value::Integer(rating)),
            (
                "ratingComment",
                rating_comment.map_or(Value::Null, |c| Value::Text(c.to_owned())),
            ),
        ]),
        scope(workspace_id, order_id),
    )
}

/// `listItems`.
pub fn list_items(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<Vec<OrderItem>> {
    find_many(
        conn,
        "WHERE \"orderId\" = ?1 AND \"workspaceId\" = ?2",
        &[&order_id, &workspace_id],
    )
}
