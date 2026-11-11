//! `EXPLAIN QUERY PLAN` checks for the message / conversation hot queries of
//! `conversion/specs/P2.4.md` section 2.2 (DECISIONS P2.4 #3 and #5).
//!
//! The plans depend on statistics: the database gets a synthetic load (as in the spec:
//! 60,000 messages in 3 workspaces over 600 conversations / contact inboxes, 12,000
//! attachments, 3,000 conversations), then `PRAGMA optimize=0x10002`, the same statement
//! that runs when a `Db` opens and after `migrate`.

use rusqlite::types::Value;
use rusqlite::{Connection, params};

use crate::{Role, migrate, setup_connection};

const WORKSPACES: i64 = 3;
const CONVERSATIONS: i64 = 3_000;
const MESSAGE_CONVERSATIONS: i64 = 600;
const MESSAGES_PER_CONVERSATION: i64 = 100;
const ATTACHMENT_EVERY: i64 = 5;
const BASE_MS: i64 = 1_790_000_000_000;

fn workspace_of(conversation: i64) -> i64 {
    conversation % WORKSPACES + 1
}

fn message_created_at(conversation: i64, n: i64) -> i64 {
    BASE_MS + (n * MESSAGE_CONVERSATIONS + conversation) * 1_000
}

/// A migrated in-memory database with the synthetic load and fresh statistics.
fn loaded() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    setup_connection(&conn, Role::Writer).expect("setup");
    migrate(&mut conn).expect("migrate");
    // The load only fills the message tables and `Conversation`; their parents do not matter here.
    conn.execute_batch("PRAGMA foreign_keys = OFF; BEGIN IMMEDIATE")
        .expect("begin");
    {
        let mut conversation = conn
            .prepare(
                r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId", "lastActivityAt",
                     "assignedUserId", "assignedInboxTeamId", "responseDueAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            )
            .expect("prepare conversation");
        for c in 1..=CONVERSATIONS {
            let source = (c % 4 == 0).then(|| format!("thread-{c}"));
            let user = (c % 3 == 0).then_some(100 + c % 20);
            let team = (c % 7 == 0).then_some(200 + c % 5);
            let due = (c % 10 == 0).then_some(BASE_MS + c * 60_000);
            conversation
                .execute(params![
                    c,
                    workspace_of(c),
                    c,
                    source,
                    BASE_MS + c * 1_000,
                    user,
                    team,
                    due
                ])
                .expect("insert conversation");
        }

        let mut message = conn
            .prepare(
                r#"INSERT INTO "Message" ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId",
                     "workspaceId", "text", "messageType", "contentType", "senderType", "sourceId", "type", "parentId")
                   VALUES (?1, ?2, ?2, ?3, ?3, ?4, 'hello', ?5, 'text', ?6, ?7, ?8, ?9)"#,
            )
            .expect("prepare message");
        let mut attachment = conn
            .prepare(
                r#"INSERT INTO "Attachment" ("id", "createdAt", "updatedAt", "workspaceId", "conversationId",
                     "messageId", "messageCreatedAt", "fileType", "mimeType", "originPath")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?2, 'image', 'image/png', 'a/b.png')"#,
            )
            .expect("prepare attachment");
        let mut id = 0;
        for n in 0..MESSAGES_PER_CONVERSATION {
            for c in 1..=MESSAGE_CONVERSATIONS {
                id += 1;
                let created = message_created_at(c, n);
                let incoming = n % 2 == 0;
                // Every 10th message is an outgoing message without a source id.
                let source = (n % 10 != 9).then(|| format!("m_{id}"));
                let comment = c % 50 == 0 && n > 0;
                let parent = comment.then(|| format!("m_{}", id - MESSAGE_CONVERSATIONS));
                message
                    .execute(params![
                        id,
                        created,
                        c,
                        workspace_of(c),
                        if incoming { "incoming" } else { "outgoing" },
                        if incoming { "contact" } else { "bot" },
                        source,
                        if comment { "comment" } else { "message" },
                        parent
                    ])
                    .expect("insert message");
                if id % ATTACHMENT_EVERY == 0 {
                    attachment
                        .execute(params![id, created, workspace_of(c), c, id])
                        .expect("insert attachment");
                }
            }
        }
    }
    conn.execute_batch("COMMIT; PRAGMA foreign_keys = ON; PRAGMA optimize=0x10002")
        .expect("commit + optimize");
    conn
}

/// The `detail` lines of `EXPLAIN QUERY PLAN sql` with `args` bound.
fn plan(conn: &Connection, sql: &str, args: &[Value]) -> String {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|e| panic!("{e}: {sql}"));
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
        .expect("plan")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("plan rows");
    rows.join("\n")
}

/// True when the plan searches or scans `table` with the index `index`.
fn uses(plan: &str, table: &str, index: &str) -> bool {
    plan.lines().any(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        words.get(1) == Some(&table) && words.contains(&index)
    })
}

#[track_caller]
fn assert_index(plan: &str, table: &str, index: &str) {
    assert!(
        uses(plan, table, index),
        "expected {index} on {table}:\n{plan}"
    );
}

#[track_caller]
fn assert_no_sort(plan: &str) {
    assert!(
        !plan.contains("USE TEMP B-TREE"),
        "unexpected sort step:\n{plan}"
    );
}

fn int(v: i64) -> Value {
    Value::Integer(v)
}

fn text(v: &str) -> Value {
    Value::Text(v.to_owned())
}

#[test]
fn bundled_sqlite_has_stat4() {
    let conn = Connection::open_in_memory().expect("open");
    let options: Vec<String> = conn
        .prepare("PRAGMA compile_options")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("rows");
    assert!(
        options.iter().any(|o| o == "ENABLE_STAT4"),
        "bundled SQLite lacks SQLITE_ENABLE_STAT4: {options:?}"
    );
}

#[test]
fn optimize_after_the_load_writes_statistics() {
    let conn = loaded();
    for table in ["Message", "Attachment", "Conversation"] {
        let rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_stat1 WHERE tbl = ?1",
                params![table],
                |r| r.get(0),
            )
            .expect("stat1");
        assert!(rows > 0, "no sqlite_stat1 rows for {table}");
    }
    let stat4: i64 = conn
        .query_row(
            r#"SELECT count(*) FROM sqlite_stat4 WHERE tbl = 'Message'"#,
            [],
            |r| r.get(0),
        )
        .expect("stat4");
    assert!(stat4 > 0, "no sqlite_stat4 samples for Message");
}

#[test]
fn message_hot_queries_use_the_planned_indexes() {
    let conn = loaded();
    let ws = workspace_of(7);
    let conversation = 7;
    let mid = message_created_at(conversation, 50);
    let source = "m_30007";

    // listByConversation / findAIContextMessages / findLastByConversation / findManyByConversation.
    let p = plan(
        &conn,
        r#"SELECT * FROM "Message" WHERE "conversationId" = ?1 AND "workspaceId" = ?2 AND "createdAt" < ?3
           ORDER BY "createdAt" DESC, "id" DESC LIMIT 50"#,
        &[int(conversation), int(ws), int(mid)],
    );
    assert_index(&p, "Message", "Message_conversation_history_idx");
    assert_no_sort(&p);
    // Keyset cursor on (createdAt, id).
    let p = plan(
        &conn,
        r#"SELECT * FROM "Message" WHERE "conversationId" = ?1 AND "workspaceId" = ?2
             AND ("createdAt" < ?3 OR ("createdAt" = ?3 AND "id" < ?4))
           ORDER BY "createdAt" DESC, "id" DESC LIMIT 50"#,
        &[int(conversation), int(ws), int(mid), int(30_007)],
    );
    assert_index(&p, "Message", "Message_conversation_history_idx");
    assert_no_sort(&p);

    // listByConversation by contact inbox / listIncomingTextsByContactInbox.
    let p = plan(
        &conn,
        r#"SELECT * FROM "Message" WHERE "workspaceId" = ?1 AND "contactInboxId" = ?2 AND "createdAt" >= ?3
           ORDER BY "createdAt" DESC LIMIT 50"#,
        &[int(ws), int(conversation), int(BASE_MS)],
    );
    assert!(
        uses(
            &p,
            "Message",
            "Message_contactInboxId_sourceId_createdAt_idx"
        ) || uses(&p, "Message", "sqlite_autoindex_Message_1"),
        "expected an index with the contactInboxId prefix:\n{p}"
    );

    // findById / updateAcrossShards / deleteById.
    let p = plan(
        &conn,
        r#"SELECT * FROM "Message" WHERE "id" = ?1 AND "createdAt" = ?2 AND "workspaceId" = ?3"#,
        &[int(30_007), int(mid), int(ws)],
    );
    assert!(p.contains("INTEGER PRIMARY KEY (rowid=?)"), "{p}");

    // patchBySourceId / mergeContentAttributesBySourceId / deleteBySourceId (no time bound, no contact inbox).
    for sql in [
        r#"SELECT "id", "createdAt" FROM "Message" WHERE "sourceId" = ?1 AND "workspaceId" = ?2"#,
        r#"UPDATE "Message" SET "text" = 'x', "updatedAt" = 1 WHERE "sourceId" = ?1 AND "workspaceId" = ?2"#,
        r#"DELETE FROM "Message" WHERE "sourceId" = ?1 AND "workspaceId" = ?2"#,
    ] {
        let p = plan(&conn, sql, &[text(source), int(ws)]);
        assert_index(&p, "Message", "Message_workspaceId_sourceId_idx");
    }

    // findBySourceId (parent comment lookup).
    let p = plan(
        &conn,
        r#"SELECT * FROM "Message" WHERE "sourceId" = ?1 AND "conversationId" = ?2 AND "workspaceId" = ?3
             AND "createdAt" >= ?4 LIMIT 1"#,
        &[text(source), int(conversation), int(ws), int(BASE_MS)],
    );
    assert_index(&p, "Message", "Message_workspaceId_sourceId_idx");

    // findManyBySourceIds.
    let p = plan(
        &conn,
        r#"SELECT * FROM "Message" WHERE "workspaceId" = ?1
             AND "contactInboxId" IN (SELECT value FROM json_each(?2))
             AND "sourceId" IN (SELECT value FROM json_each(?3)) AND "createdAt" >= ?4"#,
        &[
            int(ws),
            text("[7, 10]"),
            text(r#"["m_30007", "m_30010"]"#),
            int(BASE_MS),
        ],
    );
    assert!(
        uses(&p, "Message", "Message_workspaceId_sourceId_idx")
            || uses(
                &p,
                "Message",
                "Message_contactInboxId_sourceId_createdAt_idx"
            )
            || uses(&p, "Message", "sqlite_autoindex_Message_1"),
        "expected a source id index:\n{p}"
    );

    // Comment replies by parent.
    let p = plan(
        &conn,
        r#"SELECT * FROM "Message" WHERE "workspaceId" = ?1 AND "parentId" = ?2 AND "type" = 'comment'
           ORDER BY "createdAt" DESC LIMIT 20"#,
        &[int(workspace_of(50)), text("m_50")],
    );
    assert_index(&p, "Message", "Message_parentId_idx");
    assert_no_sort(&p);

    // Workspace purge, chunked by rowid.
    let p = plan(
        &conn,
        r#"DELETE FROM "Message" WHERE rowid IN (SELECT rowid FROM "Message" WHERE "workspaceId" = ?1 LIMIT 5000)"#,
        &[int(ws)],
    );
    assert_index(&p, "Message", "Message_workspace_created_idx");
}

#[test]
fn attachment_hot_queries_use_the_planned_indexes() {
    let conn = loaded();
    let ws = workspace_of(5);
    let p = plan(
        &conn,
        r#"SELECT * FROM "Attachment" WHERE "messageId" IN (SELECT value FROM json_each(?1)) AND "workspaceId" = ?2"#,
        &[text("[5, 10, 15]"), int(ws)],
    );
    assert_index(&p, "Attachment", "Attachment_message_idx");

    let p = plan(
        &conn,
        r#"SELECT * FROM "Attachment" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        &[int(5), int(ws)],
    );
    assert!(p.contains("INTEGER PRIMARY KEY (rowid=?)"), "{p}");
}

#[test]
fn conversation_hot_queries_use_the_planned_indexes() {
    let conn = loaded();
    let ws = 2;

    // Inbox list: all, assigned user, team.
    let p = plan(
        &conn,
        r#"SELECT * FROM "Conversation" WHERE "workspaceId" = ?1
           ORDER BY "lastActivityAt" DESC NULLS LAST, "id" DESC LIMIT 50"#,
        &[int(ws)],
    );
    assert_index(
        &p,
        "Conversation",
        "Conversation_workspaceId_lastActivityAt_id_idx",
    );
    assert_no_sort(&p);
    let p = plan(
        &conn,
        r#"SELECT * FROM "Conversation" WHERE "workspaceId" = ?1 AND "assignedUserId" = ?2
           ORDER BY "lastActivityAt" DESC, "id" DESC LIMIT 50"#,
        &[int(ws), int(103)],
    );
    assert_index(
        &p,
        "Conversation",
        "Conversation_workspaceId_assignedUserId_lastActivityAt_id_idx",
    );
    assert_no_sort(&p);
    let p = plan(
        &conn,
        r#"SELECT * FROM "Conversation" WHERE "workspaceId" = ?1 AND "assignedInboxTeamId" = ?2
           ORDER BY "lastActivityAt" DESC, "id" DESC LIMIT 50"#,
        &[int(ws), int(202)],
    );
    assert_index(
        &p,
        "Conversation",
        "Conversation_ws_team_lastActivity_id_idx",
    );
    assert_no_sort(&p);

    // Inbox SLA filter.
    let p = plan(
        &conn,
        r#"SELECT "id" FROM "Conversation" WHERE "workspaceId" = ?1 AND "responseDueAt" IS NOT NULL
             AND "responseDueAt" <= ?2"#,
        &[int(ws), int(BASE_MS + 90_000_000)],
    );
    assert_index(
        &p,
        "Conversation",
        "Conversation_workspaceId_responseDueAt_idx",
    );

    // DM of a contact.
    let p = plan(
        &conn,
        r#"SELECT * FROM "Conversation" WHERE "contactId" = ?1 AND "sourceId" IS NULL"#,
        &[int(5)],
    );
    assert!(
        uses(&p, "Conversation", "Conversation_contactId_idx")
            || uses(&p, "Conversation", "Conversation_contactId_dm_key"),
        "expected a contactId index:\n{p}"
    );
}

#[test]
fn queue_scans_use_the_planned_indexes() {
    let conn = loaded();
    let p = plan(
        &conn,
        r#"SELECT "id" FROM "SequenceDispatch" WHERE "status" = 'pending' AND "runAtMs" <= ?1
           ORDER BY "runAtMs" LIMIT 100"#,
        &[int(BASE_MS)],
    );
    // Both Postgres indexes serve this scan without a sort step; with no statistics for the
    // (empty) table the planner may take the covering `(status, runAtMs)` one.
    assert!(
        uses(
            &p,
            "SequenceDispatch",
            "SequenceDispatch_pending_runAtMs_idx"
        ) || uses(
            &p,
            "SequenceDispatch",
            "SequenceDispatch_status_runAtMs_idx"
        ),
        "expected a (status, runAtMs) index:\n{p}"
    );
    assert_no_sort(&p);

    let p = plan(
        &conn,
        r#"SELECT "contactId" FROM "ContactOnBroadcast" WHERE "broadcastId" = ?1 AND "sent" = 0
             AND "failedAt" IS NULL ORDER BY "contactInboxId" LIMIT 500"#,
        &[int(1)],
    );
    assert_index(
        &p,
        "ContactOnBroadcast",
        "ContactOnBroadcast_unsent_order_idx",
    );
    assert_no_sort(&p);

    let p = plan(
        &conn,
        r#"SELECT "id" FROM "ContactOnSmartDelay" WHERE "status" = 'pending' AND "triggerAt" <= ?1
           ORDER BY "triggerAt" LIMIT 100"#,
        &[int(BASE_MS)],
    );
    assert_index(
        &p,
        "ContactOnSmartDelay",
        "ContactOnSmartDelay_status_triggerAt_idx",
    );
    assert_no_sort(&p);
}
