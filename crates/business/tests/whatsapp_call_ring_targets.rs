//! Port of `packages/business/__tests__/whatsapp-call-ring-targets.test.ts`
//! (`isEligibleForConversationCall`, `selectRingTargets`).
use std::collections::HashMap;

use chatbotx_business::whatsapp_voip::{
    Assignment, MAX_VOIP_RING_TARGETS, is_eligible, select_ring_targets,
};
use serde_json::{Value, json};

fn perms(entries: &[(i64, Value)]) -> HashMap<i64, Value> {
    entries.iter().cloned().collect()
}

#[test]
fn super_admin_is_always_eligible_any_conversation() {
    assert!(is_eligible(1, &json!({ "superAdmin": true }), None));
    assert!(is_eligible(1, &json!({ "superAdmin": true }), Some(2)));
}

#[test]
fn contacts_permission_is_always_eligible_any_conversation() {
    assert!(is_eligible(1, &json!({ "contacts": true }), Some(2)));
}

#[test]
fn only_assigned_contacts_is_eligible_only_when_individually_assigned() {
    let only = json!({ "onlyAssignedContacts": true });
    assert!(is_eligible(1, &only, Some(1)));
    assert!(!is_eligible(1, &only, Some(2)));
    assert!(!is_eligible(1, &only, None));
}

#[test]
fn no_qualifying_permission_is_not_eligible() {
    assert!(!is_eligible(1, &json!({ "inbox": true }), Some(1)));
    assert!(!is_eligible(1, &json!({}), Some(1)));
}

#[test]
fn assignee_online_and_eligible_tier_one_wins() {
    let p = perms(&[
        (1, json!({ "contacts": true })),
        (2, json!({ "contacts": true })),
    ]);
    let a = Assignment {
        user_id: Some(1),
        team_id: None,
    };
    assert_eq!(select_ring_targets(Some(a), &[2, 1], &p, &[]), vec![1]);
}

#[test]
fn assignee_online_but_ineligible_falls_through() {
    let p = perms(&[
        (1, json!({ "inbox": true })),
        (2, json!({ "contacts": true })),
    ]);
    let a = Assignment {
        user_id: Some(1),
        team_id: None,
    };
    assert_eq!(select_ring_targets(Some(a), &[1, 2], &p, &[]), vec![2]);
}

#[test]
fn assignee_offline_falls_through_to_team_tier() {
    let p = perms(&[
        (2, json!({ "contacts": true })),
        (3, json!({ "contacts": true })),
    ]);
    let a = Assignment {
        user_id: Some(1),
        team_id: Some(9),
    };
    assert_eq!(select_ring_targets(Some(a), &[3, 2], &p, &[2]), vec![2]);
}

#[test]
fn team_member_with_only_assigned_contacts_is_skipped_in_tier_two() {
    let p = perms(&[
        (2, json!({ "onlyAssignedContacts": true })),
        (3, json!({ "contacts": true })),
    ]);
    let a = Assignment {
        user_id: None,
        team_id: Some(9),
    };
    assert_eq!(select_ring_targets(Some(a), &[2, 3], &p, &[2]), vec![3]);
}

#[test]
fn nobody_every_tier_empty() {
    assert!(select_ring_targets(None, &[], &HashMap::new(), &[]).is_empty());
}

#[test]
fn online_id_with_no_permissions_entry_is_excluded() {
    let p = perms(&[(2, json!({ "contacts": true }))]);
    assert_eq!(select_ring_targets(None, &[1, 2], &p, &[]), vec![2]);
}

#[test]
fn caps_the_winning_tier_preserving_presence_order_after_eligibility() {
    let online: Vec<i64> = (1..=15).collect();
    let mut p = perms(&[]);
    for user in &online {
        // The odd users are ineligible: the cap applies after the filter.
        let value = if user % 2 == 0 {
            json!({ "contacts": true })
        } else {
            json!({})
        };
        p.insert(*user, value);
    }
    for user in 16..=30 {
        p.insert(user, json!({ "contacts": true }));
    }
    let online: Vec<i64> = (1..=30).collect();
    let targets = select_ring_targets(None, &online, &p, &[]);
    assert_eq!(targets.len(), MAX_VOIP_RING_TARGETS);
    assert_eq!(&targets[..3], &[2, 4, 6]);
}
