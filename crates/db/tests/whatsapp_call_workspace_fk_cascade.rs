//! Port of `packages/database/__tests__/whatsapp-call-workspace-fk-cascade.test.ts`
//! (P2.4 spec section 3.1, row 5.1). Class C: runs against a migrated SQLite file.

mod common;

use chatbotx_db::rusqlite::params;
use common::{fks, fresh_db, insert_contact_inbox, insert_conversation, query_i64, seed};

/// Port of: packages/database/__tests__/whatsapp-call-workspace-fk-cascade.test.ts:6 "WhatsappCall.workspaceId foreign key > cascades on Workspace delete — a deleted workspace must never strand orphaned call rows"
///
/// Same assertion on the SQLite catalog: the FK on `workspaceId` points to `Workspace` with
/// `ON DELETE CASCADE`. Extra check of the same intent: a deleted workspace leaves no call row.
#[tokio::test]
async fn cascades_on_workspace_delete_a_deleted_workspace_must_never_strand_orphaned_call_rows() {
    let db = fresh_db().await;

    let all = fks(&db, "WhatsappCall").await;
    let workspace_fk: Vec<_> = all.iter().filter(|fk| fk.from == "workspaceId").collect();
    assert_eq!(
        workspace_fk.len(),
        1,
        "expected one FK on WhatsappCall.workspaceId: {all:?}"
    );
    assert_eq!(workspace_fk[0].parent, "Workspace");
    assert_eq!(workspace_fk[0].on_delete, "CASCADE");

    let s = seed(&db).await;
    db.write(move |tx| {
        insert_contact_inbox(tx, s, 8201, "wa-src-1", None, None)?;
        insert_conversation(tx, s, 8301, None)?;
        tx.execute(
            r#"INSERT INTO "WhatsappCall"
                 ("id", "direction", "workspaceId", "inboxId", "contactInboxId", "conversationId")
               VALUES (8401, 'userInitiated', ?1, ?2, 8201, 8301)"#,
            params![s.workspace_id, s.inbox_id],
        )?;
        Ok(())
    })
    .await
    .expect("insert one call");

    db.write(move |tx| {
        tx.execute(
            r#"DELETE FROM "Workspace" WHERE "id" = ?1"#,
            params![s.workspace_id],
        )?;
        Ok(())
    })
    .await
    .expect("delete the workspace");

    assert_eq!(
        query_i64(
            &db,
            r#"SELECT count(*) FROM "WhatsappCall" WHERE "workspaceId" = ?1"#,
            vec![s.workspace_id.into()],
        )
        .await,
        0,
        "a deleted workspace must not strand call rows"
    );
}
