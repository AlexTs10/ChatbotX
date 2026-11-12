//! Port of `packages/utils/__tests__/created-at-from-id.test.ts`.

mod support;

use chatbotx_core::utils::id::{create_id, created_at_from_id};

/// Port of: packages/utils/__tests__/created-at-from-id.test.ts:5 "createdAtFromId > recovers the creation time packed into a createId id"
#[test]
fn createdatfromid_recovers_the_creation_time_packed_into_a_createid_id() {
    let before = support::now_ms();
    let id = create_id();
    let after = support::now_ms();
    let created_at = created_at_from_id(&id).unwrap_or(0);
    assert!(created_at >= before);
    assert!(created_at <= after);
    assert_eq!(created_at_from_id(&id), Some(created_at));
}

/// Port of: packages/utils/__tests__/created-at-from-id.test.ts:15 "createdAtFromId > returns null for non-numeric ids"
#[test]
fn createdatfromid_returns_null_for_non_numeric_ids() {
    assert_eq!(created_at_from_id("abc"), None);
    assert_eq!(created_at_from_id("0"), None);
}
