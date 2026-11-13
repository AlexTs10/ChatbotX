//! Port of `packages/business/__tests__/copy-name.test.ts` (P3.6-D2). The
//! mocked `isUniqueViolationError` becomes the `is_unique_violation`
//! argument with the same rule (code `23505`; a given constraint must be
//! `Menu_key`).

use std::cell::Cell;
use std::collections::HashSet;

use chatbotx_business::copy_name::{
    CopyNameOptions, CopyNameStyle, MAX_COPY_NAME_ATTEMPTS, copy_name, copy_name_prefix,
    free_copy_names, insert_with_free_copy_name,
};

const DASH: CopyNameOptions = CopyNameOptions {
    style: CopyNameStyle::Dash,
    max_length: 255,
};
const UNDERSCORE: CopyNameOptions = CopyNameOptions {
    style: CopyNameStyle::Underscore,
    max_length: 255,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DbErr {
    code: Option<&'static str>,
    message: &'static str,
}

fn conflict() -> DbErr {
    DbErr {
        code: Some("23505"),
        message: "taken",
    }
}

fn is_unique_violation(error: &DbErr, constraint: Option<&str>) -> bool {
    error.code == Some("23505") && (constraint.is_none() || constraint == Some("Menu_key"))
}

fn taken(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

// describe("copy names")

#[test]
fn numbers_copies_from_the_second_on_per_style() {
    assert_eq!(copy_name("Menu", 1, DASH), "Menu - Copy");
    assert_eq!(copy_name("Menu", 3, DASH), "Menu - Copy 3");
    assert_eq!(copy_name("Menu", 1, UNDERSCORE), "Menu _copy");
    assert_eq!(copy_name("Menu", 2, UNDERSCORE), "Menu _copy 2");
}

#[test]
fn cuts_the_base_so_the_suffix_fits_the_limit() {
    let name = copy_name(
        &"x".repeat(300),
        1,
        CopyNameOptions {
            max_length: 20,
            ..DASH
        },
    );
    assert_eq!(name.chars().count(), 20);
    assert!(name.ends_with(" - Copy"));
}

#[test]
fn the_prefix_is_shared_by_every_copy_name_of_a_long_base() {
    let options = CopyNameOptions {
        max_length: 30,
        ..DASH
    };
    let base = "y".repeat(100);
    let prefix = copy_name_prefix(&base, options);
    for attempt in 1..=MAX_COPY_NAME_ATTEMPTS {
        assert!(copy_name(&base, attempt, options).starts_with(&prefix));
    }
}

#[test]
fn an_uncut_base_searches_for_its_own_copies_not_every_name_that_begins_with_it() {
    assert_eq!(copy_name_prefix("A", DASH), "A - Copy");
    assert_eq!(copy_name_prefix("A", UNDERSCORE), "A _copy");
}

#[test]
fn lists_the_free_names_lowest_number_first() {
    let free = free_copy_names("Menu", &taken(&["Menu - Copy", "Menu - Copy 3"]), DASH);
    assert_eq!(
        free[..3].to_vec(),
        vec!["Menu - Copy 2", "Menu - Copy 4", "Menu - Copy 5"]
    );
}

// describe("insertWithFreeCopyName")

#[test]
fn moves_to_the_next_candidate_when_a_concurrent_copy_took_the_name() {
    let calls = Cell::new(0);
    let result = insert_with_free_copy_name(
        "Menu",
        &HashSet::new(),
        DASH,
        None,
        is_unique_violation,
        |name: &str| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Err(conflict())
            } else {
                Ok(name.to_string())
            }
        },
    );
    assert_eq!(result, Ok(Some("Menu - Copy 2".to_string())));
    assert_eq!(calls.get(), 2);
}

#[test]
fn gives_up_with_undefined_after_the_bounded_insert_attempts() {
    let calls = Cell::new(0);
    let result: Result<Option<String>, DbErr> = insert_with_free_copy_name(
        "Menu",
        &HashSet::new(),
        DASH,
        None,
        is_unique_violation,
        |_: &str| {
            calls.set(calls.get() + 1);
            Err(conflict())
        },
    );
    assert_eq!(result, Ok(None));
    assert_eq!(calls.get(), 2);
}

#[test]
fn with_a_constraint_a_violation_of_another_index_is_not_a_taken_name() {
    let violation = DbErr {
        code: Some("23505"),
        message: "slug",
    };
    let result: Result<Option<String>, DbErr> = insert_with_free_copy_name(
        "Menu",
        &HashSet::new(),
        DASH,
        Some("Other_key"),
        is_unique_violation,
        |_: &str| Err(violation.clone()),
    );
    assert_eq!(result, Err(violation));
}

#[test]
fn rethrows_anything_that_is_not_a_unique_violation() {
    let result: Result<Option<String>, DbErr> = insert_with_free_copy_name(
        "Menu",
        &HashSet::new(),
        DASH,
        None,
        is_unique_violation,
        |_: &str| {
            Err(DbErr {
                code: None,
                message: "boom",
            })
        },
    );
    assert_eq!(result.map_err(|e| e.message), Err("boom"));
}
