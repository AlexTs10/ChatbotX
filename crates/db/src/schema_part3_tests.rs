//! Unit checks for `0004_rest.sql` (`conversion/specs/P2.5.md`):
//! the 4 rollup tables and their 12 triggers (section 1.5), the exact filtered cosine
//! scan over the `BLOB` vector columns (section 2) and the rollup day buckets in a
//! half-hour time zone (section 1.5.3).

use rusqlite::types::Value;
use rusqlite::{Connection, params, params_from_iter};

use crate::functions::{vector_distance_cos, vector32};
use crate::schema::ROLLUP_TABLES;
use crate::{Role, migrate, setup_connection};

/// 2026-01-15T00:00:00Z.
const BASE_MS: i64 = 1_768_435_200_000;
const HOUR_MS: i64 = 3_600_000;
const DIM: usize = 1536;

fn migrated() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    setup_connection(&conn, Role::Writer).expect("setup");
    migrate(&mut conn).expect("migrate");
    conn
}

fn seed_workspaces(conn: &Connection, ids: &[i64]) {
    for id in ids {
        conn.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
            params![id, format!("owner-{id}@example.com")],
        )
        .expect("user");
        conn.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?1)"#,
            params![id, format!("Workspace {id}")],
        )
        .expect("workspace");
    }
}

fn rows(conn: &Connection, sql: &str, args: &[Value]) -> Vec<Vec<Value>> {
    let mut stmt = conn.prepare(sql).unwrap_or_else(|e| panic!("{e}: {sql}"));
    let n = stmt.column_count();
    stmt.query_map(params_from_iter(args), |r| {
        (0..n).map(|i| r.get::<_, Value>(i)).collect()
    })
    .expect("query")
    .collect::<rusqlite::Result<Vec<_>>>()
    .expect("rows")
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect(sql)
}

fn plan(conn: &Connection, sql: &str, args: &[Value]) -> String {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|e| panic!("{e}: {sql}"));
    stmt.query_map(params_from_iter(args), |r| r.get::<_, String>(3))
        .expect("plan")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("plan rows")
        .join("\n")
}

fn t(s: &str) -> Value {
    Value::Text(s.to_owned())
}

fn i(v: i64) -> Value {
    Value::Integer(v)
}

// ---------------------------------------------------------------------------
// Rollups (spec 1.5)
// ---------------------------------------------------------------------------

/// One rollup with its event table: the group columns (after `workspaceId`, `bucket`),
/// the values each group column cycles through (NULL, '' and 0 included so they must stay
/// separate groups), and the other NOT NULL event columns (set to 1).
struct Rollup {
    rollup: &'static str,
    events: &'static str,
    groups: &'static [&'static str],
    choices: fn(usize) -> Vec<Value>,
    other_required: &'static [&'static str],
}

fn contact_choices(col: usize) -> Vec<Value> {
    match col {
        0 => vec![
            t("contact_created"),
            t("contact_deleted"),
            t("contact_blocked"),
        ],
        1 => vec![Value::Null, t(""), t("whatsapp"), t("messenger")],
        2 => vec![Value::Null, t("bot"), t("human")],
        3 => vec![Value::Null, t(""), t("VN")],
        4 => vec![Value::Null, t("ads"), t("")],
        _ => vec![Value::Null, i(0), i(7)],
    }
}

fn bot_message_choices(col: usize) -> Vec<Value> {
    match col {
        0 => vec![i(0), i(1)],
        1 => vec![
            Value::Null,
            t("automated_response"),
            t("ai_agent"),
            t("flow"),
            t("none"),
        ],
        2 => vec![Value::Null, t("flow"), t("agent"), t("fallback")],
        3 => vec![Value::Null, t("success"), t("fallback")],
        _ => vec![Value::Null, t(""), t("openai"), t("claude")],
    }
}

fn conversation_choices(col: usize) -> Vec<Value> {
    match col {
        0 => vec![
            t("conversation_created"),
            t("conversation_assigned"),
            t("conversation_archived"),
        ],
        _ => vec![Value::Null, i(0), i(11), i(12)],
    }
}

fn message_choices(col: usize) -> Vec<Value> {
    match col {
        0 => vec![t("message_human_sent"), t("message_bot_sent")],
        1 => vec![Value::Null, t(""), t("zalo")],
        2 => vec![Value::Null, t("bot"), t("human")],
        _ => vec![Value::Null, i(0), i(3)],
    }
}

const ROLLUPS: &[Rollup] = &[
    Rollup {
        rollup: "analytics_contact_events_hourly",
        events: "AnalyticsContactEvent",
        groups: &[
            "eventType",
            "channel",
            "senderType",
            "country",
            "source",
            "adminId",
        ],
        choices: contact_choices,
        other_required: &["contactId"],
    },
    Rollup {
        rollup: "analytics_bot_message_events_hourly",
        events: "AnalyticsBotMessageEvent",
        groups: &[
            "hasResponse",
            "responseType",
            "routeType",
            "result",
            "aiProvider",
        ],
        choices: bot_message_choices,
        other_required: &["messageId", "conversationId"],
    },
    Rollup {
        rollup: "analytics_conversation_events_hourly",
        events: "AnalyticsConversationEvent",
        groups: &["eventType", "toAssignee"],
        choices: conversation_choices,
        other_required: &["conversationId"],
    },
    Rollup {
        rollup: "analytics_message_events_hourly",
        events: "AnalyticsMessageEvent",
        groups: &["eventType", "channel", "senderType", "adminId"],
        choices: message_choices,
        other_required: &["contactId"],
    },
];

/// Deterministic pseudo-random number for event `n`, group column `col`.
fn mix(n: usize, col: usize) -> usize {
    let x = (n as u64 + 1).wrapping_mul(6_364_136_223_846_793_005) ^ ((col as u64 + 1) << 29);
    (x.wrapping_mul(1_442_695_040_888_963_407) >> 33) as usize
}

impl Rollup {
    fn quoted_groups(&self) -> String {
        self.groups
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn group_values(&self, n: usize, salt: usize) -> Vec<Value> {
        (0..self.groups.len())
            .map(|col| {
                let choices = (self.choices)(col);
                choices[mix(n + salt, col) % choices.len()].clone()
            })
            .collect()
    }

    /// `INSERT … ON CONFLICT DO NOTHING` (the repository form). Returns the changed row count.
    fn insert(
        &self,
        conn: &Connection,
        ws: i64,
        event_id: &str,
        at: i64,
        groups: &[Value],
    ) -> usize {
        let mut cols = vec!["eventId", "workspaceId", "occurredAt"];
        cols.extend(self.other_required);
        cols.extend(self.groups);
        let list = cols
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let marks = (1..=cols.len())
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![t(event_id), i(ws), i(at)];
        args.extend(self.other_required.iter().map(|_| i(1)));
        args.extend(groups.iter().cloned());
        conn.execute(
            &format!(
                "INSERT INTO \"{}\" ({list}) VALUES ({marks}) ON CONFLICT DO NOTHING",
                self.events
            ),
            params_from_iter(args),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", self.events))
    }

    fn rollup_rows(&self, conn: &Connection) -> Vec<Vec<Value>> {
        let g = self.quoted_groups();
        rows(
            conn,
            &format!(
                "SELECT \"workspaceId\", \"bucket\", {g}, \"count\" FROM \"{}\" ORDER BY \"workspaceId\", \"bucket\", {g}",
                self.rollup
            ),
            &[],
        )
    }

    /// The continuous aggregate's definition, over the raw events.
    fn group_by_rows(&self, conn: &Connection) -> Vec<Vec<Value>> {
        let g = self.quoted_groups();
        rows(
            conn,
            &format!(
                "SELECT \"workspaceId\", (\"occurredAt\" / 3600000) * 3600000 AS b, {g}, count(*) FROM \"{}\" \
                 GROUP BY \"workspaceId\", b, {g} ORDER BY \"workspaceId\", b, {g}",
                self.events
            ),
            &[],
        )
    }

    #[track_caller]
    fn assert_exact(&self, conn: &Connection, step: &str) {
        let rollup = self.rollup_rows(conn);
        assert_eq!(
            rollup,
            self.group_by_rows(conn),
            "{} after {step}",
            self.rollup
        );
    }
}

const EVENTS: usize = 240;

fn event_at(n: usize) -> i64 {
    // 6 hours, several events per hour bucket, some exactly on the hour boundary.
    BASE_MS
        + (mix(n, 99) % 6) as i64 * HOUR_MS
        + if n % 10 == 0 {
            0
        } else {
            (mix(n, 98) % 3_600) as i64 * 1_000
        }
}

fn load_events(conn: &Connection, r: &Rollup) {
    for n in 0..EVENTS {
        let ws = 1 + (n % 2) as i64;
        let groups = r.group_values(n, 0);
        assert_eq!(
            r.insert(conn, ws, &format!("e{n}"), event_at(n), &groups),
            1
        );
    }
}

#[test]
fn rollup_tables_constant_lists_the_four_rollups_with_triggers() {
    let conn = migrated();
    assert_eq!(ROLLUP_TABLES.len(), ROLLUPS.len());
    for (r, (rollup, events)) in ROLLUPS.iter().zip(ROLLUP_TABLES) {
        assert_eq!((r.rollup, r.events), (*rollup, *events));
        let triggers: Vec<String> = rows(
            &conn,
            "SELECT name FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1 ORDER BY name",
            &[t(events)],
        )
        .into_iter()
        .map(|row| match &row[0] {
            Value::Text(s) => s.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
        assert_eq!(
            triggers,
            [
                format!("{events}_rollup_ad"),
                format!("{events}_rollup_ai"),
                format!("{events}_rollup_au"),
            ]
        );
    }
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = 'AnalyticsAgentResponseEvent'"
        ),
        0
    );
}

#[test]
fn rollups_equal_group_by_after_inserts() {
    let conn = migrated();
    seed_workspaces(&conn, &[1, 2]);
    for r in ROLLUPS {
        load_events(&conn, r);
        r.assert_exact(&conn, "inserts");
        let total = count(
            &conn,
            &format!("SELECT sum(\"count\") FROM \"{}\"", r.rollup),
        );
        assert_eq!(total, EVENTS as i64, "{}", r.rollup);
        assert!(
            r.rollup_rows(&conn).len() > 12,
            "{}: the load must make several groups per hour",
            r.rollup
        );
    }
}

#[test]
fn rollups_skip_a_duplicate_event() {
    let conn = migrated();
    seed_workspaces(&conn, &[1, 2]);
    for r in ROLLUPS {
        load_events(&conn, r);
        let before = r.rollup_rows(&conn);
        // Same PK ("occurredAt", "eventId"), other group values: ON CONFLICT DO NOTHING fires no trigger.
        for n in [0, 1, 17] {
            let ws = 1 + (n % 2) as i64;
            let other = r.group_values(n, 1000);
            assert_eq!(
                r.insert(&conn, ws, &format!("e{n}"), event_at(n), &other),
                0
            );
        }
        assert_eq!(r.rollup_rows(&conn), before, "{}", r.rollup);
        r.assert_exact(&conn, "duplicates");
    }
}

#[test]
fn rollups_follow_updates_of_group_columns_and_time() {
    let conn = migrated();
    seed_workspaces(&conn, &[1, 2]);
    for r in ROLLUPS {
        load_events(&conn, r);
        // Each group column in turn, on a different third of the rows.
        for (col, name) in r.groups.iter().enumerate() {
            let choices = (r.choices)(col);
            for n in (col % 3..EVENTS).step_by(3) {
                let value = choices[mix(n, 500 + col) % choices.len()].clone();
                conn.execute(
                    &format!(
                        "UPDATE \"{}\" SET \"{name}\" = ?1 WHERE \"eventId\" = ?2",
                        r.events
                    ),
                    params![value, format!("e{n}")],
                )
                .expect("update group column");
            }
            r.assert_exact(&conn, &format!("UPDATE of {name}"));
        }
        // Move events to another hour, and to the other workspace.
        conn.execute(
            &format!(
                "UPDATE \"{}\" SET \"occurredAt\" = \"occurredAt\" + 7200000 WHERE rowid % 4 = 0",
                r.events
            ),
            [],
        )
        .expect("move hour");
        r.assert_exact(&conn, "UPDATE of occurredAt");
        conn.execute(
            &format!(
                "UPDATE \"{}\" SET \"workspaceId\" = 3 - \"workspaceId\" WHERE rowid % 5 = 0",
                r.events
            ),
            [],
        )
        .expect("move workspace");
        r.assert_exact(&conn, "UPDATE of workspaceId");
        // A column outside the key fires no rollup change.
        let before = r.rollup_rows(&conn);
        conn.execute(
            &format!("UPDATE \"{}\" SET \"metadata\" = '{{\"a\":1}}'", r.events),
            [],
        )
        .expect("update metadata");
        assert_eq!(r.rollup_rows(&conn), before, "{}", r.rollup);
    }
}

#[test]
fn rollups_follow_deletes_and_drop_empty_groups() {
    let conn = migrated();
    seed_workspaces(&conn, &[1, 2]);
    for r in ROLLUPS {
        load_events(&conn, r);
        conn.execute(
            &format!("DELETE FROM \"{}\" WHERE rowid % 3 = 1", r.events),
            [],
        )
        .expect("delete some");
        r.assert_exact(&conn, "DELETE of a third");
        // The retention form: chunked deletes by time.
        conn.execute(
            &format!(
                "DELETE FROM \"{0}\" WHERE rowid IN (SELECT rowid FROM \"{0}\" WHERE \"occurredAt\" < ?1 LIMIT 5000)",
                r.events
            ),
            params![BASE_MS + 2 * HOUR_MS],
        )
        .expect("delete by time");
        r.assert_exact(&conn, "DELETE by time");
        conn.execute(&format!("DELETE FROM \"{}\"", r.events), [])
            .expect("delete all");
        assert_eq!(
            count(&conn, &format!("SELECT count(*) FROM \"{}\"", r.rollup)),
            0,
            "{}: a group whose last event goes away is deleted",
            r.rollup
        );
    }
}

#[test]
fn rollups_go_with_a_workspace_cascade() {
    let conn = migrated();
    seed_workspaces(&conn, &[1, 2]);
    for r in ROLLUPS {
        load_events(&conn, r);
    }
    conn.execute(r#"DELETE FROM "Workspace" WHERE "id" = 1"#, [])
        .expect("delete workspace");
    for r in ROLLUPS {
        let left = |table: &str, ws: i64| {
            count(
                &conn,
                &format!("SELECT count(*) FROM \"{table}\" WHERE \"workspaceId\" = {ws}"),
            )
        };
        assert_eq!(left(r.events, 1), 0, "{}", r.events);
        assert_eq!(left(r.rollup, 1), 0, "{}", r.rollup);
        assert_eq!(left(r.events, 2), (EVENTS / 2) as i64, "{}", r.events);
        r.assert_exact(&conn, "Workspace cascade");
    }
}

#[test]
fn rollups_keep_null_empty_and_zero_as_separate_groups() {
    let conn = migrated();
    seed_workspaces(&conn, &[1]);
    let r = &ROLLUPS[3]; // analytics_message_events_hourly
    let at = BASE_MS + 5 * 60_000;
    for (n, channel) in [Value::Null, t(""), Value::Null, t("")]
        .into_iter()
        .enumerate()
    {
        let groups = [t("message_human_sent"), channel, Value::Null, Value::Null];
        assert_eq!(r.insert(&conn, 1, &format!("m{n}"), at, &groups), 1);
    }
    let adm0 = [t("message_human_sent"), Value::Null, Value::Null, i(0)];
    assert_eq!(r.insert(&conn, 1, "m-admin-0", at, &adm0), 1);
    assert_eq!(
        rows(
            &conn,
            r#"SELECT "channel", "adminId", "count" FROM "analytics_message_events_hourly" ORDER BY "channel", "adminId""#,
            &[],
        ),
        [
            vec![Value::Null, Value::Null, i(2)],
            vec![Value::Null, i(0), i(1)],
            vec![t(""), Value::Null, i(2)],
        ]
    );
    // bucket = UTC hour start.
    assert_eq!(
        count(
            &conn,
            r#"SELECT DISTINCT "bucket" FROM "analytics_message_events_hourly""#
        ),
        BASE_MS
    );
}

#[test]
fn rollup_trigger_statements_use_the_rollup_key() {
    let conn = migrated();
    seed_workspaces(&conn, &[1, 2]);
    for r in ROLLUPS {
        load_events(&conn, r);
    }
    conn.execute_batch("PRAGMA optimize=0x10002")
        .expect("optimize");
    for r in ROLLUPS {
        // The WHERE of the `_ad` / `_au` trigger bodies, with OLD.* as bind values.
        let mut wh = vec![
            r#""workspaceId" = ?1"#.to_owned(),
            r#""bucket" = ?2"#.to_owned(),
        ];
        for (n, c) in r.groups.iter().enumerate() {
            let p = n + 3;
            wh.push(if n == 0 {
                format!("\"{c}\" = ?{p}")
            } else {
                format!("ifnull(\"{c}\", x'') = ifnull(?{p}, x'')")
            });
        }
        let sql = format!(
            "UPDATE \"{}\" SET \"count\" = \"count\" - 1 WHERE {}",
            r.rollup,
            wh.join(" AND ")
        );
        let mut args = vec![i(1), i(BASE_MS)];
        args.extend(r.group_values(0, 0));
        let p = plan(&conn, &sql, &args);
        let key = format!("{}_key", r.rollup);
        assert!(
            p.contains(&format!("USING INDEX {key}")),
            "{}: expected {key}:\n{p}",
            r.rollup
        );
    }
}

// ---------------------------------------------------------------------------
// Rollup day buckets in a half-hour time zone (spec 1.5.3)
// ---------------------------------------------------------------------------

#[test]
fn rollup_day_buckets_in_a_half_hour_zone_follow_the_utc_hour_bucket() {
    let conn = migrated();
    seed_workspaces(&conn, &[1]);
    let r = &ROLLUPS[3];
    let groups = [t("message_human_sent"), t("zalo"), t("human"), Value::Null];
    // Asia/Kolkata is UTC+05:30.
    let at_18_10 = BASE_MS + 18 * HOUR_MS + 10 * 60_000; // 23:40 IST on Jan 15
    let at_18_45 = BASE_MS + 18 * HOUR_MS + 45 * 60_000; // 00:15 IST on Jan 16
    let at_19_05 = BASE_MS + 19 * HOUR_MS + 5 * 60_000; // 00:35 IST on Jan 16
    for (n, at) in [at_18_10, at_18_45, at_19_05].into_iter().enumerate() {
        assert_eq!(r.insert(&conn, 1, &format!("k{n}"), at, &groups), 1);
    }
    let jan15 = BASE_MS;
    let jan16 = BASE_MS + 24 * HOUR_MS;

    // Rollup path: Postgres `time_bucket('1 day', bucket AT TIME ZONE tz AT TIME ZONE 'UTC')`
    // over the UTC hour buckets. The 18:00 UTC bucket is 23:30 IST, so the 18:45 event
    // counts on the previous local day.
    let rollup = rows(
        &conn,
        r#"SELECT (tz_wall_ms("bucket", tz_resolve(?1)) / 86400000) * 86400000 AS "day", SUM("count")
           FROM "analytics_message_events_hourly" WHERE "workspaceId" = 1
           GROUP BY "day" ORDER BY "day""#,
        &[t("Asia/Kolkata")],
    );
    assert_eq!(rollup, [vec![i(jan15), i(2)], vec![i(jan16), i(1)]]);

    // Raw path: each "occurredAt" on its own local day.
    let raw = rows(
        &conn,
        r#"SELECT (tz_wall_ms("occurredAt", tz_resolve(?1)) / 86400000) * 86400000 AS "day", count(*)
           FROM "AnalyticsMessageEvent" WHERE "workspaceId" = 1
           GROUP BY "day" ORDER BY "day""#,
        &[t("Asia/Kolkata")],
    );
    assert_eq!(raw, [vec![i(jan15), i(1)], vec![i(jan16), i(2)]]);

    // A whole-hour zone gives the same days on both paths.
    let q = |table: &str, col: &str| {
        rows(
            &conn,
            &format!(
                "SELECT (tz_wall_ms(\"{col}\", tz_resolve(?1)) / 86400000) * 86400000 AS d, count(*) \
                 FROM \"{table}\" GROUP BY d ORDER BY d"
            ),
            &[t("Asia/Saigon")],
        )
    };
    let rollup_days: Vec<Value> = q("analytics_message_events_hourly", "bucket")
        .into_iter()
        .map(|row| row[0].clone())
        .collect();
    let raw_days: Vec<Value> = q("AnalyticsMessageEvent", "occurredAt")
        .into_iter()
        .map(|row| row[0].clone())
        .collect();
    assert_eq!(rollup_days, raw_days);
    assert_eq!(rollup_days, [i(jan16)]);
}

// ---------------------------------------------------------------------------
// Vector columns and the exact filtered cosine scan (spec 2)
// ---------------------------------------------------------------------------

/// Spec 2.2, file chunks.
const FILE_SCAN: &str = r#"SELECT "id", "content", "aiFileId", "chunkIndex",
       1 - vector_distance_cos("embedding", vector32(?1)) AS "similarity"
FROM "AIEmbedding" INDEXED BY "AIEmbedding_aiFileId_chunkIndex_idx"
WHERE "workspaceId" = ?2
  AND "aiFileId" IN (SELECT value FROM json_each(?3))
  AND "status" = 'success'
  AND "embedding" IS NOT NULL
ORDER BY vector_distance_cos("embedding", vector32(?1)) ASC NULLS LAST, "id"
LIMIT ?4"#;

/// Spec 2.2, conversation sources.
const SOURCE_SCAN: &str = r#"SELECT "id", "chunkIndex", "content",
       1 - vector_distance_cos("embedding", vector32(?1)) AS "similarity"
FROM "AIConversationEmbedding"
WHERE "sourceId" = ?2 AND "workspaceId" = ?3 AND "status" = 'success' AND "embedding" IS NOT NULL
ORDER BY vector_distance_cos("embedding", vector32(?1)) ASC NULLS LAST, "id"
LIMIT ?4"#;

/// A deterministic 1536-dim vector.
fn vector(seed: usize) -> Vec<f32> {
    (0..DIM)
        .map(|k| ((mix(seed, k) % 2001) as f32 - 1000.0) / 1000.0)
        .collect()
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn to_json(v: &[f32]) -> String {
    let items: Vec<String> = v.iter().map(|f| format!("{f}")).collect();
    format!("[{}]", items.join(","))
}

fn insert_file(conn: &Connection, id: i64, ws: i64) {
    conn.execute(
        r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "workspaceId")
           VALUES (?1, 'f.txt', 'ai/f.txt', 10, 'text/plain', ?2)"#,
        params![id, ws],
    )
    .expect("AIFile");
}

fn insert_chunk(
    conn: &Connection,
    id: i64,
    ws: i64,
    file: i64,
    chunk: i64,
    status: &str,
    embedding: Option<Vec<u8>>,
) {
    conn.execute(
        r#"INSERT INTO "AIEmbedding" ("id", "content", "embedding", "status", "chunkIndex", "workspaceId", "aiFileId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
        params![id, format!("chunk {id}"), embedding, status, chunk, ws, file],
    )
    .expect("AIEmbedding");
}

/// Files 10, 11, 13 in workspace 1; file 12 in workspace 2. Chunks interleaved by file.
fn load_chunks(conn: &Connection, query: &[f32]) {
    seed_workspaces(conn, &[1, 2]);
    for (file, ws) in [(10, 1), (11, 1), (12, 2), (13, 1)] {
        insert_file(conn, file, ws);
    }
    let mut id = 1000;
    for chunk in 0..60 {
        for (file, ws) in [(10, 1), (11, 1), (12, 2), (13, 1)] {
            id += 1;
            let (status, embedding) = match id % 9 {
                0 => ("pending", None),
                1 => ("error", Some(to_blob(&vector(id as usize)))),
                _ => ("success", Some(to_blob(&vector(id as usize)))),
            };
            insert_chunk(conn, id, ws, file, chunk, status, embedding);
        }
    }
    // Exact matches of the query that the filters must drop …
    let exact = to_blob(query);
    insert_chunk(conn, 9001, 2, 12, 100, "success", Some(exact.clone())); // other workspace
    insert_chunk(conn, 9002, 1, 13, 100, "success", Some(exact.clone())); // file not asked for
    insert_chunk(conn, 9003, 1, 10, 100, "error", Some(exact.clone())); // not success
    // … and the ones it must keep: the same direction (distance 0), and a zero vector (NULL).
    let doubled: Vec<f32> = query.iter().map(|x| x * 2.0).collect();
    insert_chunk(conn, 9004, 1, 11, 100, "success", Some(to_blob(&doubled)));
    insert_chunk(
        conn,
        9005,
        1,
        10,
        101,
        "success",
        Some(to_blob(&[0.0; DIM])),
    );
}

fn scan_ids(conn: &Connection, query: &[f32], limit: i64) -> Vec<(i64, Option<f64>)> {
    rows(
        conn,
        FILE_SCAN,
        &[t(&to_json(query)), i(1), t("[10, 11]"), i(limit)],
    )
    .into_iter()
    .map(|row| {
        let id = match row[0] {
            Value::Integer(v) => v,
            ref other => panic!("{other:?}"),
        };
        let similarity = match row[4] {
            Value::Real(f) => Some(f),
            Value::Null => None,
            ref other => panic!("{other:?}"),
        };
        (id, similarity)
    })
    .collect()
}

#[test]
fn vector_columns_take_only_6144_byte_blobs() {
    let conn = migrated();
    seed_workspaces(&conn, &[1]);
    insert_file(&conn, 10, 1);
    insert_chunk(&conn, 1, 1, 10, 0, "success", Some(to_blob(&vector(1))));
    insert_chunk(&conn, 2, 1, 10, 1, "pending", None);
    let short = conn.execute(
        r#"INSERT INTO "AIEmbedding" ("id", "content", "embedding", "workspaceId", "aiFileId") VALUES (3, 'c', x'00', 1, 10)"#,
        [],
    );
    assert!(
        short
            .expect_err("1 byte")
            .to_string()
            .contains("CHECK constraint failed"),
    );
    let text = conn.execute(
        r#"INSERT INTO "AIEmbedding" ("id", "content", "embedding", "workspaceId", "aiFileId") VALUES (4, 'c', '[0.1]', 1, 10)"#,
        [],
    );
    assert!(
        text.expect_err("TEXT")
            .to_string()
            .contains("cannot store TEXT value in BLOB column")
    );
    // vector32() of the JSON text is the stored layout.
    let json = to_json(&vector(1));
    assert_eq!(
        count(
            &conn,
            &format!(
                r#"SELECT count(*) FROM "AIEmbedding" WHERE "embedding" = vector32('{json}')"#
            )
        ),
        1
    );
    assert_eq!(vector32(&json).expect("vector32"), to_blob(&vector(1)));
}

#[test]
fn file_vector_scan_is_the_exact_filtered_cosine_order() {
    let conn = migrated();
    let query = vector(424_242);
    load_chunks(&conn, &query);

    // Brute force in Rust over the rows the filters keep.
    let candidates = rows(
        &conn,
        r#"SELECT "id", "embedding" FROM "AIEmbedding"
           WHERE "workspaceId" = 1 AND "aiFileId" IN (10, 11) AND "status" = 'success' AND "embedding" IS NOT NULL"#,
        &[],
    );
    let q = to_blob(&query);
    let mut expected: Vec<(i64, Option<f64>)> = candidates
        .into_iter()
        .map(|row| match (&row[0], &row[1]) {
            (Value::Integer(id), Value::Blob(b)) => {
                (*id, vector_distance_cos(b, &q).expect("distance"))
            }
            other => panic!("{other:?}"),
        })
        .collect();
    expected.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => x.total_cmp(&y).then(a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    let all = expected.len() as i64;
    assert!(all > 80, "{all} candidates");

    let got = scan_ids(&conn, &query, all + 10);
    assert_eq!(got.len(), expected.len());
    for (g, e) in got.iter().zip(&expected) {
        assert_eq!(g.0, e.0);
        match (g.1, e.1) {
            (Some(s), Some(d)) => assert!((s - (1.0 - d)).abs() < 1e-12, "{g:?} vs {e:?}"),
            (None, None) => {}
            other => panic!("{other:?}"),
        }
    }
    // Nearest: the same direction in file 11; the zero vector sorts last (NULLS LAST).
    assert_eq!(got[0].0, 9004);
    assert!((got[0].1.expect("similarity") - 1.0).abs() < 1e-6);
    assert_eq!(got.last().copied(), Some((9005, None)));
    for dropped in [9001, 9002, 9003] {
        assert!(
            got.iter().all(|g| g.0 != dropped),
            "{dropped} must be filtered out"
        );
    }

    let top = scan_ids(&conn, &query, 5);
    assert_eq!(
        top.iter().map(|g| g.0).collect::<Vec<_>>(),
        expected[..5].iter().map(|e| e.0).collect::<Vec<_>>()
    );
}

#[test]
fn file_vector_scan_plan_searches_the_file_index() {
    let conn = migrated();
    let query = vector(7);
    load_chunks(&conn, &query);
    conn.execute_batch("PRAGMA optimize=0x10002")
        .expect("optimize");
    let p = plan(
        &conn,
        FILE_SCAN,
        &[t(&to_json(&query)), i(1), t("[10, 11]"), i(5)],
    );
    assert!(
        p.lines().any(|l| l.contains(
            "SEARCH AIEmbedding USING INDEX AIEmbedding_aiFileId_chunkIndex_idx (aiFileId=?)"
        )),
        "{p}"
    );
    assert!(!p.contains("SCAN AIEmbedding"), "{p}");
    assert!(!p.contains("AIEmbedding_workspaceId_idx"), "{p}");
}

#[test]
fn conversation_vector_scan_plan_uses_the_source_status_index() {
    let conn = migrated();
    // The plan needs only statistics; parents do not matter here.
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("fk off");
    let mut id: i64 = 0;
    for source in 1..=40i64 {
        for chunk in 0..10 {
            id += 1;
            let status = if chunk % 4 == 0 { "pending" } else { "success" };
            conn.execute(
                r#"INSERT INTO "AIConversationEmbedding" ("id", "sourceId", "workspaceId", "conversationId", "chunkIndex", "content", "embedding", "status")
                   VALUES (?1, ?2, ?3, ?4, ?5, 'c', ?6, ?7)"#,
                params![id, source, 1 + source % 3, source, chunk, to_blob(&vector(id as usize)), status],
            )
            .expect("AIConversationEmbedding");
        }
    }
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA optimize=0x10002")
        .expect("optimize");
    let query = vector(3);
    let p = plan(&conn, SOURCE_SCAN, &[t(&to_json(&query)), i(5), i(3), i(5)]);
    assert!(
        p.contains(
            "USING INDEX AIConversationEmbedding_sourceId_status_idx (sourceId=? AND status=?)"
        ),
        "{p}"
    );
    let got = rows(
        &conn,
        SOURCE_SCAN,
        &[t(&to_json(&query)), i(5), i(3), i(50)],
    );
    assert_eq!(got.len(), 7, "source 5: 10 chunks, 3 pending");
}
