//! Port of `packages/database/src/utils.ts`.

/// `maxLimit`: the largest page size of [`parse_pagination`].
pub const MAX_LIMIT: i64 = 50;

/// `defaultPagination`.
pub const DEFAULT_PAGINATION: Pagination = Pagination {
    limit: 20,
    offset: 0,
};

/// Input of [`parse_pagination`] (`{ page?, perPage? }`; `0` counts as absent, as the
/// JS truthiness check).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaginationInput {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// `{ limit, offset }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
}

/// `escapeLikePattern`: backslash-escapes `\`, `%` and `_` (use with `ESCAPE '\'`).
pub fn escape_like_pattern(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// `likeContains`: `%<escaped>%`.
pub fn like_contains(value: &str) -> String {
    format!("%{}%", escape_like_pattern(value))
}

/// `isBigintId`: digits only, at most 19 of them, and at most `i64::MAX`.
pub fn is_bigint_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 19
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<i64>().is_ok()
}

/// `parsePagination`.
pub fn parse_pagination(input: PaginationInput) -> Option<Pagination> {
    let page = input.page.filter(|p| *p != 0);
    let per_page = input.per_page.filter(|p| *p != 0);
    if let Some(per_page) = per_page {
        let limit = MAX_LIMIT.min(per_page);
        return Some(Pagination {
            limit,
            offset: (page.unwrap_or(1) - 1) * limit,
        });
    }
    if let Some(page) = page {
        // `input.perPage ?? 20`: a `0` per page stays `0` here.
        let limit = MAX_LIMIT.min(input.per_page.unwrap_or(DEFAULT_PAGINATION.limit));
        return Some(Pagination {
            limit,
            offset: (page - 1) * limit,
        });
    }
    None
}

/// `getPaginationWithDefaults`.
pub fn get_pagination_with_defaults(input: PaginationInput) -> Pagination {
    parse_pagination(input).unwrap_or(DEFAULT_PAGINATION)
}

/// Sort direction of an order-by entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    /// `ASC` / `DESC`.
    pub fn sql(self) -> &'static str {
        match self {
            SortDir::Asc => "ASC",
            SortDir::Desc => "DESC",
        }
    }
}

/// One `{ id, desc }` sort item of a list input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortItem {
    pub id: String,
    pub desc: bool,
}

/// `parseOrderByAsObject` / `parseOrderBy`: keeps the items whose column is in `columns`
/// (the model's columns) and in `allowed` (when given), in input order.
pub fn parse_order_by(
    columns: &[&str],
    sort: Option<&[SortItem]>,
    allowed: Option<&[&str]>,
) -> Vec<(String, SortDir)> {
    let Some(sort) = sort else {
        return Vec::new();
    };
    let mut out: Vec<(String, SortDir)> = Vec::new();
    for item in sort {
        let sortable = columns.contains(&item.id.as_str())
            && allowed.is_none_or(|a| a.contains(&item.id.as_str()));
        if !sortable {
            continue;
        }
        let dir = if item.desc {
            SortDir::Desc
        } else {
            SortDir::Asc
        };
        // An object keeps one entry per key (a later item overwrites in place).
        match out.iter_mut().find(|(c, _)| *c == item.id) {
            Some(slot) => slot.1 = dir,
            None => out.push((item.id.clone(), dir)),
        }
    }
    out
}

/// `withIdTieBreak`: appends `id` in the direction of the first entry (`desc` without
/// one) unless `id` is already there.
pub fn with_id_tie_break(order_by: &[(String, SortDir)]) -> Vec<(String, SortDir)> {
    let mut out = order_by.to_vec();
    if !out.iter().any(|(c, _)| c == "id") {
        let dir = out.first().map_or(SortDir::Desc, |(_, d)| *d);
        out.push(("id".to_owned(), dir));
    }
    out
}

/// `ORDER BY` text of an order-by list (`"col" DIR, ...`); empty for an empty list.
pub fn order_by_sql(order_by: &[(String, SortDir)], alias: Option<&str>) -> String {
    if order_by.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = order_by
        .iter()
        .map(|(c, d)| match alias {
            Some(a) => format!("{a}.\"{c}\" {}", d.sql()),
            None => format!("\"{c}\" {}", d.sql()),
        })
        .collect();
    format!("ORDER BY {}", parts.join(", "))
}

/// `chunkById`: calls `query(last_id)` and `callback(rows)` until a short page, an
/// empty page or `callback` returns `false`. `id_of` reads the id of a row.
pub fn chunk_by_id<T, E>(
    chunk_size: usize,
    mut query: impl FnMut(Option<i64>) -> Result<Vec<T>, E>,
    id_of: impl Fn(&T) -> i64,
    mut callback: impl FnMut(&[T]) -> Result<bool, E>,
) -> Result<(), E> {
    let mut last_id: Option<i64> = None;
    loop {
        let records = query(last_id)?;
        if records.is_empty() {
            break;
        }
        if !callback(&records)? {
            break;
        }
        if records.len() < chunk_size {
            break;
        }
        last_id = records.last().map(&id_of);
    }
    Ok(())
}

/// `normalizeForMatch` (`packages/utils/src/normalize-for-match.ts`): NFD, the Combining
/// Diacritical Marks (U+0300-U+036F) dropped, `đ` / `Đ` -> `d`, lower-cased. Same as
/// `chatbotx_core::utils::normalize_for_match` (crates/db cannot depend on crates/core).
pub fn normalize_for_match(value: &str) -> String {
    let decomposed = icu_normalizer::DecomposingNormalizerBorrowed::new_nfd().normalize(value);
    let folded: String = decomposed
        .chars()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .map(|c| if c == 'đ' || c == 'Đ' { 'd' } else { c })
        .collect();
    folded.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_follows_the_js_truthiness_rules() {
        let none = PaginationInput::default();
        assert_eq!(parse_pagination(none), None);
        assert_eq!(get_pagination_with_defaults(none), DEFAULT_PAGINATION);
        let p = parse_pagination(PaginationInput {
            page: Some(3),
            per_page: Some(80),
        });
        assert_eq!(
            p,
            Some(Pagination {
                limit: 50,
                offset: 100
            })
        );
        let p = parse_pagination(PaginationInput {
            page: Some(2),
            per_page: None,
        });
        assert_eq!(
            p,
            Some(Pagination {
                limit: 20,
                offset: 20
            })
        );
    }

    #[test]
    fn bigint_ids_and_like_escaping() {
        assert!(is_bigint_id("9223372036854775807"));
        assert!(!is_bigint_id("9223372036854775808"));
        assert!(!is_bigint_id("12a"));
        assert!(!is_bigint_id(""));
        assert_eq!(escape_like_pattern("a%_\\"), "a\\%\\_\\\\");
    }

    #[test]
    fn order_by_keeps_allowed_model_columns() {
        let sort = vec![
            SortItem {
                id: "name".into(),
                desc: true,
            },
            SortItem {
                id: "secret".into(),
                desc: false,
            },
            SortItem {
                id: "nope".into(),
                desc: false,
            },
        ];
        let cols = ["id", "name", "secret"];
        assert_eq!(
            parse_order_by(&cols, Some(&sort), Some(&["name"])),
            vec![("name".to_owned(), SortDir::Desc)]
        );
        assert_eq!(parse_order_by(&cols, Some(&sort), None).len(), 2);
        assert!(parse_order_by(&cols, None, None).is_empty());
    }
}
