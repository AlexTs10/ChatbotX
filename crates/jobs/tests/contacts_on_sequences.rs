//! Port of `packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts`
//! (spec 5.2).

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use chatbotx_jobs::sequence::contacts::{
    ContactInboxRow, SequenceDiff, calculate_sequence_diff, get_all_sequence_ids,
    get_contact_inboxes,
};
use chatbotx_jobs::sequence::repo::StatementKind;
use serde_json::json;

use sequence::{
    CI_1, CI_MINE, CI_OTHER, CONTACT_1, CONTACT_2, COS_1, COS_2, COS_3, Harness, INBOX_1, INBOX_2,
    INBOX_99, INBOX_OTHER, MY_CONTACT, MY_WORKSPACE, OTHER_CONTACT, OTHER_WORKSPACE, SEQ_1, SEQ_2,
    SEQ_3, WS_1, id,
};

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// calculateSequenceDiff
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:32 "calculateSequenceDiff > returns empty toAdd and toRemove when both current and new are empty"
#[test]
fn returns_empty_toadd_and_toremove_when_both_current_and_new() {
    let result = calculate_sequence_diff(&[], &[]);

    assert_eq!(result, SequenceDiff::default());
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:42 "calculateSequenceDiff > returns all new ids in toAdd and empty toRemove when current is empty"
#[test]
fn returns_all_new_ids_in_toadd_and_empty_toremove_when() {
    let result = calculate_sequence_diff(&[], &["seq-1", "seq-2"]);

    assert_eq!(sorted(result.to_add.clone()), strings(&["seq-1", "seq-2"]));
    assert_eq!(result.to_add.len(), 2);
    assert!(result.to_remove.is_empty());
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:57 "calculateSequenceDiff > returns empty toAdd and all current ids in toRemove when new is empty"
#[test]
fn returns_empty_toadd_and_all_current_ids_in_toremove_when() {
    let result = calculate_sequence_diff(&["seq-a", "seq-b"], &[]);

    assert!(result.to_add.is_empty());
    assert_eq!(
        sorted(result.to_remove.clone()),
        strings(&["seq-a", "seq-b"])
    );
    assert_eq!(result.to_remove.len(), 2);
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:72 "calculateSequenceDiff > returns empty toAdd and toRemove when current and new are identical"
#[test]
fn returns_empty_toadd_and_toremove_when_current_and_new_are() {
    let result = calculate_sequence_diff(&["seq-1", "seq-2"], &["seq-1", "seq-2"]);

    assert_eq!(result, SequenceDiff::default());
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:85 "calculateSequenceDiff > returns disjoint toAdd and toRemove for completely different sets"
#[test]
fn returns_disjoint_toadd_and_toremove_for_completely() {
    let result = calculate_sequence_diff(&["old-1", "old-2"], &["new-1", "new-2"]);

    let to_add = sorted(result.to_add);
    let to_remove = sorted(result.to_remove);
    assert!(to_add.contains(&"new-1".to_owned()) && to_add.contains(&"new-2".to_owned()));
    assert!(to_remove.contains(&"old-1".to_owned()) && to_remove.contains(&"old-2".to_owned()));
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:99 "calculateSequenceDiff > returns only the non-overlapping delta for partially overlapping sets"
#[test]
fn returns_only_the_non_overlapping_delta_for_partially() {
    let result = calculate_sequence_diff(&["seq-keep", "seq-old"], &["seq-keep", "seq-new"]);

    assert_eq!(result.to_add, strings(&["seq-new"]));
    assert_eq!(result.to_remove, strings(&["seq-old"]));
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:114 "calculateSequenceDiff > deduplicates duplicate ids in the inputs"
#[test]
fn deduplicates_duplicate_ids_in_the_inputs() {
    let result = calculate_sequence_diff(&["seq-1", "seq-1"], &["seq-2", "seq-2"]);

    assert_eq!(result.to_add, strings(&["seq-2"]));
    assert_eq!(result.to_remove, strings(&["seq-1"]));
}

// ---------------------------------------------------------------------------
// getAllSequenceIds
// ---------------------------------------------------------------------------

async fn seed_enrollment(h: &Harness, enrollment: i64, contact: i64, sequence_id: i64) {
    sequence::seed_enrollment(
        &h.db,
        json!({ "id": enrollment, "contactId": contact, "sequenceId": sequence_id }),
    )
    .await;
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:130 "getAllSequenceIds > returns an array of sequenceId strings from the rows"
#[tokio::test]
async fn returns_an_array_of_sequenceid_strings_from_the_rows() {
    let h = sequence::harness(Default::default()).await;
    seed_enrollment(&h, COS_1, CONTACT_1, SEQ_1).await;
    seed_enrollment(&h, COS_2, CONTACT_1, SEQ_2).await;
    seed_enrollment(&h, COS_3, CONTACT_2, SEQ_3).await;

    let result = get_all_sequence_ids(&h.db, &id(CONTACT_1))
        .await
        .expect("ids");

    assert_eq!(result, vec![id(SEQ_1), id(SEQ_2)]);
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:147 "getAllSequenceIds > returns an empty array when no sequences are found"
#[tokio::test]
async fn returns_an_empty_array_when_no_sequences_are_found() {
    let h = sequence::harness(Default::default()).await;

    let result = get_all_sequence_ids(&h.db, &id(CONTACT_1))
        .await
        .expect("ids");

    assert_eq!(result, Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:163 "public API > does not expose the unused bulkRemoveIds helper"
///
/// Adapted (TS-only `in` check): a source guard; the module declares no
/// `bulk_remove_ids` symbol.
#[test]
fn does_not_expose_the_unused_bulkremoveids_helper() {
    let source = include_str!("../src/sequence/contacts.rs");

    assert!(!source.contains("bulk_remove_ids"));
}

// ---------------------------------------------------------------------------
// getContactInboxes
// ---------------------------------------------------------------------------

/// A `"ContactInbox"` row `ci` of `contact` on `inbox`; the contact and the
/// inbox are created in `ws`.
async fn seed_contact_inbox(h: &Harness, ci: i64, contact: i64, inbox: i64, ws: i64) {
    sequence::seed_contact_inbox_row(&h.db, ci, contact, inbox, ws, false).await;
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:173 "getContactInboxes > returns contact inboxes matching workspace inboxes"
#[tokio::test]
async fn returns_contact_inboxes_matching_workspace_inboxes() {
    let h = sequence::harness(Default::default()).await;
    sequence::ensure_inbox(&h.db, INBOX_1, WS_1).await;
    sequence::ensure_inbox(&h.db, INBOX_2, WS_1).await;
    seed_contact_inbox(&h, CI_1, CONTACT_1, INBOX_1, WS_1).await;

    let result = get_contact_inboxes(&h.ctx, &id(WS_1), &id(CONTACT_1))
        .await
        .expect("contact inboxes");

    assert_eq!(
        result,
        vec![ContactInboxRow {
            id: id(CI_1),
            contact_id: id(CONTACT_1),
            inbox_id: id(INBOX_1),
        }]
    );
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:186 "getContactInboxes > returns empty array when no inboxes exist in the workspace"
#[tokio::test]
async fn returns_empty_array_when_no_inboxes_exist_in_the_workspace() {
    let h = sequence::harness(Default::default()).await;

    let result = get_contact_inboxes(&h.ctx, &id(WS_1), &id(CONTACT_1))
        .await
        .expect("contact inboxes");

    assert_eq!(result, Vec::<ContactInboxRow>::new());
    assert_eq!(h.probe.count(StatementKind::Select, "ContactInbox"), 0);
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:197 "getContactInboxes > queries inboxModel scoped by workspaceId"
#[tokio::test]
async fn queries_inboxmodel_scoped_by_workspaceid() {
    let h = sequence::harness(Default::default()).await;
    sequence::ensure_workspace(&h.db, MY_WORKSPACE).await;
    // Decoy: an inbox of another workspace with a contact inbox of the contact.
    seed_contact_inbox(&h, CI_OTHER, CONTACT_1, INBOX_OTHER, OTHER_WORKSPACE).await;

    let result = get_contact_inboxes(&h.ctx, &id(MY_WORKSPACE), &id(CONTACT_1))
        .await
        .expect("contact inboxes");

    assert_eq!(result, Vec::<ContactInboxRow>::new());
}

/// Port of: packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts:211 "getContactInboxes > queries contactInboxModel scoped by contactId"
#[tokio::test]
async fn queries_contactinboxmodel_scoped_by_contactid() {
    let h = sequence::harness(Default::default()).await;
    sequence::ensure_inbox(&h.db, INBOX_99, WS_1).await;
    // Decoy: a contact inbox of another contact on the same inbox.
    seed_contact_inbox(&h, CI_OTHER, OTHER_CONTACT, INBOX_99, WS_1).await;
    seed_contact_inbox(&h, CI_MINE, MY_CONTACT, INBOX_99, WS_1).await;

    let result = get_contact_inboxes(&h.ctx, &id(WS_1), &id(MY_CONTACT))
        .await
        .expect("contact inboxes");

    assert_eq!(
        result,
        vec![ContactInboxRow {
            id: id(CI_MINE),
            contact_id: id(MY_CONTACT),
            inbox_id: id(INBOX_99),
        }]
    );
}
