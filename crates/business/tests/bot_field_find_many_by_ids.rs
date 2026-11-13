//! Port of the `findManyByIds` part of
//! `packages/business/__tests__/bot-field-resolve-by-name-and-type.test.ts`
//! (P3.6-D2 slice bot fields). Adapted: the mocked `findMany` becomes real
//! rows ("scoped by workspaceId" = a field of another workspace is not
//! returned). The `resolveByNameAndType` cases are not-applicable here (the
//! template install planner is not ported in this slice).

mod common;

use chatbotx_business::bot_field::find_many_by_ids;
use common::{OTHER_WS, WS, bot_field, setup, two_workspaces};

// ---- botFieldService.findManyByIds ----

#[tokio::test]
async fn bot_field_service_find_many_by_ids_scopes_the_lookup_by_workspace_id_alongside_the_id_list()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        bot_field(tx, 1, WS, "a", "shortText", None, 1_000)?;
        bot_field(tx, 2, OTHER_WS, "b", "shortText", None, 1_000)?;
        Ok(())
    })
    .await;
    let rows = find_many_by_ids(&db, WS, &[1, 2]).await.expect("find");
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![1]);
}

#[tokio::test]
async fn bot_field_service_find_many_by_ids_returns_an_empty_array_for_an_empty_id_list_without_querying()
 {
    let db = two_workspaces().await;
    assert!(
        find_many_by_ids(&db, WS, &[])
            .await
            .expect("find")
            .is_empty()
    );
}
