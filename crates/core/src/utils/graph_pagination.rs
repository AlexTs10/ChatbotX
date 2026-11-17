//! `packages/utils/src/graph-pagination.ts`: bounded Graph API pagination.

use std::collections::HashSet;
use std::fmt;
use std::future::Future;

/// `DEFAULT_GRAPH_PAGE_LIMIT`.
pub const DEFAULT_GRAPH_PAGE_LIMIT: usize = 100;
/// `DEFAULT_GRAPH_MAX_PAGES`.
pub const DEFAULT_GRAPH_MAX_PAGES: usize = 20;

/// A page of a Graph list (`data` + `paging`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphPage<T> {
    pub data: Vec<T>,
    /// `paging.cursors.after`.
    pub after: Option<String>,
    /// `paging.next`.
    pub next: Option<String>,
}

/// A pagination failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphPaginationError<E> {
    /// `GraphPaginationMaxPagesReachedError`: the walk hit `max_pages`.
    MaxPagesReached { pages_fetched: usize },
    /// The page getter failed.
    Get(E),
}

impl<E: fmt::Display> fmt::Display for GraphPaginationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphPaginationError::MaxPagesReached { pages_fetched } => {
                write!(f, "Graph pagination exceeded {pages_fetched} pages")
            }
            GraphPaginationError::Get(err) => err.fmt(f),
        }
    }
}

fn origin_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
}

/// `fetchAllNextPages`: follows same-origin `paging.next` links (never
/// another host, never a revisited page) up to `max_pages`; reaching the
/// bound with a next page left is an error (after `on_max_pages_reached`).
pub async fn fetch_all_next_pages<T, E, G, Fut>(
    first_url: &str,
    mut get: G,
    max_pages: Option<usize>,
    mut on_max_pages_reached: Option<&mut dyn FnMut(usize)>,
) -> Result<Vec<T>, GraphPaginationError<E>>
where
    G: FnMut(String) -> Fut,
    Fut: Future<Output = Result<GraphPage<T>, E>>,
{
    let max_pages = max_pages.unwrap_or(DEFAULT_GRAPH_MAX_PAGES);
    let origin = origin_of(first_url);
    let mut results = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut next_url = Some(first_url.to_owned());
    for _ in 0..max_pages {
        let Some(url) = next_url.take() else {
            return Ok(results);
        };
        visited.insert(url.clone());
        let page = get(url).await.map_err(GraphPaginationError::Get)?;
        results.extend(page.data);
        next_url = page
            .next
            .filter(|next| !next.is_empty() && !visited.contains(next))
            .filter(|next| origin.is_some() && origin_of(next) == origin);
    }
    if next_url.is_some() {
        if let Some(callback) = on_max_pages_reached.as_mut() {
            callback(max_pages);
        }
        return Err(GraphPaginationError::MaxPagesReached {
            pages_fetched: max_pages,
        });
    }
    Ok(results)
}

/// The search params of one cursor page request, in insertion order.
pub type SearchParams = Vec<(String, String)>;

/// `fetchAllCursorPages`: `fields`, `access_token`, `limit` (and `after`)
/// on every request, following `paging.cursors.after` while `paging.next`
/// is set, at most `max_pages` pages.
pub async fn fetch_all_cursor_pages<T, E, G, Fut>(
    endpoint: &str,
    fields: &str,
    access_token: &str,
    mut get: G,
    limit: Option<usize>,
    max_pages: Option<usize>,
) -> Result<Vec<T>, E>
where
    G: FnMut(String, SearchParams) -> Fut,
    Fut: Future<Output = Result<GraphPage<T>, E>>,
{
    let limit = limit.unwrap_or(DEFAULT_GRAPH_PAGE_LIMIT);
    let max_pages = max_pages.unwrap_or(DEFAULT_GRAPH_MAX_PAGES);
    let mut results = Vec::new();
    let mut cursor: Option<String> = None;
    let mut page_count = 0;
    while page_count < max_pages {
        let mut params: SearchParams = vec![
            ("fields".to_owned(), fields.to_owned()),
            ("access_token".to_owned(), access_token.to_owned()),
            ("limit".to_owned(), limit.to_string()),
        ];
        if let Some(after) = cursor.as_ref().filter(|c| !c.is_empty()) {
            params.push(("after".to_owned(), after.clone()));
        }
        let page = get(endpoint.to_owned(), params).await?;
        results.extend(page.data);
        page_count += 1;
        cursor = if page.next.as_deref().is_some_and(|n| !n.is_empty()) {
            page.after
        } else {
            None
        };
        if cursor.as_deref().is_none_or(str::is_empty) {
            break;
        }
    }
    Ok(results)
}
