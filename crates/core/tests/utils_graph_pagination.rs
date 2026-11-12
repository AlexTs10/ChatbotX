//! Port of `packages/utils/__tests__/graph-pagination.test.ts`. The mocked
//! `get` is a closure that records its calls and returns scripted pages.

use std::cell::RefCell;
use std::collections::VecDeque;

use chatbotx_core::utils::graph_pagination::{
    GraphPage, GraphPaginationError, SearchParams, fetch_all_cursor_pages, fetch_all_next_pages,
};

const GRAPH_URL: &str = "https://graph.facebook.com/v23.0/waba/phone_numbers";

fn page(ids: &[&str], after: Option<&str>, next: Option<&str>) -> GraphPage<String> {
    GraphPage {
        data: ids.iter().map(|id| (*id).to_owned()).collect(),
        after: after.map(str::to_owned),
        next: next.map(str::to_owned),
    }
}

fn params(pairs: &[(&str, &str)]) -> SearchParams {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:11 "fetchAllCursorPages > follows cursor pages and sends the configured limit"
#[tokio::test]
async fn fetchallcursorpages_follows_cursor_pages_and_sends_the_configured_limit() {
    let calls: RefCell<Vec<(String, SearchParams)>> = RefCell::new(Vec::new());
    let pages = RefCell::new(VecDeque::from([
        page(
            &["first"],
            Some("next-cursor"),
            Some("https://graph.example/page?after=next-cursor"),
        ),
        page(&["second"], None, None),
    ]));
    let result = fetch_all_cursor_pages(
        "v23.0/me/accounts",
        "id,name",
        "token",
        |endpoint, search| {
            calls.borrow_mut().push((endpoint, search));
            let next = pages.borrow_mut().pop_front().unwrap();
            async move { Ok::<_, String>(next) }
        },
        Some(50),
        None,
    )
    .await
    .unwrap();

    assert_eq!(result, vec!["first", "second"]);
    let calls = calls.into_inner();
    assert_eq!(
        calls[0],
        (
            "v23.0/me/accounts".to_owned(),
            params(&[
                ("fields", "id,name"),
                ("access_token", "token"),
                ("limit", "50")
            ])
        )
    );
    assert_eq!(
        calls[1],
        (
            "v23.0/me/accounts".to_owned(),
            params(&[
                ("fields", "id,name"),
                ("access_token", "token"),
                ("limit", "50"),
                ("after", "next-cursor")
            ])
        )
    );
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:49 "fetchAllCursorPages > stops at maxPages even when Graph keeps returning cursors"
#[tokio::test]
async fn fetchallcursorpages_stops_at_maxpages_even_when_graph_keeps_returning_cursors() {
    let count = RefCell::new(0);
    let result = fetch_all_cursor_pages(
        "v23.0/me/accounts",
        "id",
        "token",
        |_, _| {
            *count.borrow_mut() += 1;
            async {
                Ok::<_, String>(page(
                    &["page"],
                    Some("cursor"),
                    Some("https://graph.example/page?after=cursor"),
                ))
            }
        },
        None,
        Some(2),
    )
    .await
    .unwrap();

    assert_eq!(result, vec!["page", "page"]);
    assert_eq!(*count.borrow(), 2);
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:72 "fetchAllNextPages > follows paging.next until Graph stops offering one"
#[tokio::test]
async fn fetchallnextpages_follows_paging_next_until_graph_stops_offering_one() {
    let calls = RefCell::new(Vec::new());
    let pages = RefCell::new(VecDeque::from([
        page(&["first"], None, Some(&format!("{GRAPH_URL}?after=b"))),
        page(&["second"], None, None),
    ]));
    let result = fetch_all_next_pages(
        GRAPH_URL,
        |url| {
            calls.borrow_mut().push(url);
            let next = pages.borrow_mut().pop_front().unwrap();
            async move { Ok::<_, String>(next) }
        },
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(result, vec!["first", "second"]);
    assert_eq!(
        *calls.borrow(),
        vec![GRAPH_URL.to_owned(), format!("{GRAPH_URL}?after=b")]
    );
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:88 "fetchAllNextPages > returns the page when Graph omits paging entirely"
#[tokio::test]
async fn fetchallnextpages_returns_the_page_when_graph_omits_paging_entirely() {
    let calls = RefCell::new(0);
    let result = fetch_all_next_pages(
        GRAPH_URL,
        |_| {
            *calls.borrow_mut() += 1;
            async { Ok::<_, String>(page(&["only"], None, None)) }
        },
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(result, vec!["only"]);
    assert_eq!(*calls.borrow(), 1);
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:97 "fetchAllNextPages > never forwards the request to another origin"
#[tokio::test]
async fn fetchallnextpages_never_forwards_the_request_to_another_origin() {
    let calls = RefCell::new(Vec::new());
    let result = fetch_all_next_pages(
        GRAPH_URL,
        |url| {
            calls.borrow_mut().push(url);
            async {
                Ok::<_, String>(page(
                    &["first"],
                    None,
                    Some("https://attacker.example/steal?token=1"),
                ))
            }
        },
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(result, vec!["first"]);
    assert_eq!(*calls.borrow(), vec![GRAPH_URL.to_owned()]);
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:110 "fetchAllNextPages > ignores a paging.next that is not a usable URL"
#[tokio::test]
async fn fetchallnextpages_ignores_a_paging_next_that_is_not_a_usable_url() {
    let calls = RefCell::new(0);
    let result = fetch_all_next_pages(
        GRAPH_URL,
        |_| {
            *calls.borrow_mut() += 1;
            async { Ok::<_, String>(page(&["first"], None, Some("not-a-url"))) }
        },
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(result, vec!["first"]);
    assert_eq!(*calls.borrow(), 1);
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:122 "fetchAllNextPages > breaks a cycle that revisits an earlier page"
#[tokio::test]
async fn fetchallnextpages_breaks_a_cycle_that_revisits_an_earlier_page() {
    let calls = RefCell::new(0);
    let pages = RefCell::new(VecDeque::from([
        page(&["first"], None, Some(&format!("{GRAPH_URL}?after=b"))),
        page(&["second"], None, Some(GRAPH_URL)),
    ]));
    let result = fetch_all_next_pages(
        GRAPH_URL,
        |_| {
            *calls.borrow_mut() += 1;
            let next = pages.borrow_mut().pop_front().unwrap();
            async move { Ok::<_, String>(next) }
        },
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(result, vec!["first", "second"]);
    assert_eq!(*calls.borrow(), 2);
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:140 "fetchAllNextPages > throws once maxPages is reached so callers cannot use a partial list"
#[tokio::test]
async fn fetchallnextpages_throws_once_maxpages_is_reached_so_callers_cannot_use_a_partial_list() {
    let calls = RefCell::new(0usize);
    let reached = RefCell::new(Vec::new());
    let mut on_max = |pages: usize| reached.borrow_mut().push(pages);
    let result = fetch_all_next_pages(
        GRAPH_URL,
        |url| {
            *calls.borrow_mut() += 1;
            let after = *calls.borrow();
            async move {
                Ok::<_, String>(page(
                    &[url.as_str()],
                    None,
                    Some(&format!("{GRAPH_URL}?after={after}")),
                ))
            }
        },
        Some(3),
        Some(&mut on_max as &mut dyn FnMut(usize)),
    )
    .await;

    assert!(matches!(
        result,
        Err(GraphPaginationError::MaxPagesReached { .. })
    ));
    assert_eq!(*calls.borrow(), 3);
    assert_eq!(*reached.borrow(), vec![3]);
}

/// Port of: packages/utils/__tests__/graph-pagination.test.ts:159 "fetchAllNextPages > stays quiet when the walk ends before maxPages"
#[tokio::test]
async fn fetchallnextpages_stays_quiet_when_the_walk_ends_before_maxpages() {
    let reached = RefCell::new(Vec::new());
    let mut on_max = |pages: usize| reached.borrow_mut().push(pages);
    fetch_all_next_pages(
        GRAPH_URL,
        |_| async { Ok::<_, String>(page(&["only"], None, None)) },
        Some(3),
        Some(&mut on_max as &mut dyn FnMut(usize)),
    )
    .await
    .unwrap();

    assert!(reached.borrow().is_empty());
}
