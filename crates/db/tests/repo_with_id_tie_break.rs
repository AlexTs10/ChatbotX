//! Port of `packages/database/__tests__/with-id-tie-break.test.ts` (P3.2).
//! The TS order-by object becomes an ordered `Vec<(column, SortDir)>`.

use chatbotx_db::repo::utils::{SortDir, with_id_tie_break};

fn o(items: &[(&str, SortDir)]) -> Vec<(String, SortDir)> {
    items.iter().map(|(c, d)| ((*c).to_owned(), *d)).collect()
}

/// Port of: packages/database/__tests__/with-id-tie-break.test.ts:5 "withIdTieBreak > settles ties in the direction of the requested sort"
#[test]
fn with_id_tie_break_settles_ties_in_the_direction_of_the_requested_sort() {
    assert_eq!(
        with_id_tie_break(&o(&[("createdAt", SortDir::Desc)])),
        o(&[("createdAt", SortDir::Desc), ("id", SortDir::Desc)])
    );
    assert_eq!(
        with_id_tie_break(&o(&[("createdAt", SortDir::Asc)])),
        o(&[("createdAt", SortDir::Asc), ("id", SortDir::Asc)])
    );
}

/// Port of: packages/database/__tests__/with-id-tie-break.test.ts:16 "withIdTieBreak > the requested sort stays first"
#[test]
fn with_id_tie_break_the_requested_sort_stays_first() {
    let keys: Vec<String> = with_id_tie_break(&o(&[("name", SortDir::Asc)]))
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert_eq!(keys, vec!["name".to_owned(), "id".to_owned()]);
}

/// Port of: packages/database/__tests__/with-id-tie-break.test.ts:20 "withIdTieBreak > a requested id sort is kept"
#[test]
fn with_id_tie_break_a_requested_id_sort_is_kept() {
    assert_eq!(
        with_id_tie_break(&o(&[("createdAt", SortDir::Desc), ("id", SortDir::Asc)])),
        o(&[("createdAt", SortDir::Desc), ("id", SortDir::Asc)])
    );
}

/// Port of: packages/database/__tests__/with-id-tie-break.test.ts:27 "withIdTieBreak > with no sort, newest first"
#[test]
fn with_id_tie_break_with_no_sort_newest_first() {
    assert_eq!(with_id_tie_break(&[]), o(&[("id", SortDir::Desc)]));
}
