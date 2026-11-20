//! `features/channel-posts/api/public.ts` (scope `contacts`): the posts a
//! `commentedOnPost` contact filter can pick (`channelPostService
//! .listFilterOptions` / `findByIds`).
//!
//! ponytail: SQLite `LIKE` folds ASCII case only (the original `ILIKE`
//! folds every letter); the keyset `sortAt` is millisecond ISO text, not
//! Postgres microsecond `timestamptz` text (the cursor accepts both).

use chatbotx_business::connection::providers::CHANNEL_TYPES;
use chatbotx_business::{BusinessError, tx};
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_db::repo::{id_list, iso, like_contains, parse_iso};
use chatbotx_db::rusqlite::{Row, params};
use serde_json::{Value, json};

use super::input::{Input, Num, Seg, Str, child, key};
use super::{ApiState, Call, LISTING, Reply, business_error, json, opt_date};

/// The routes of this module.
pub const ROUTES: &[&str] = &["channelPosts.list", "channelPosts.listSelected"];

const SELECT: &str = r#"SELECT p."id", p."caption", p."channel", p."externalPostId", p."inboxId", i."name",
    p."permalink", p."publishedAt", p."thumbnail", COALESCE(p."publishedAt", p."createdAt")
    FROM "ChannelPost" p JOIN "Inbox" i ON i."id" = p."inboxId""#;

/// `channelPostId`: `^[1-9]\d*$`, at most 19 characters and `i64::MAX`.
fn post_id(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<i64> {
    let text = input.string_at(path, value, Str::max(19), true)?;
    let digits = text.starts_with(|c: char| ('1'..='9').contains(&c))
        && text.bytes().all(|b| b.is_ascii_digit());
    if !digits {
        input.issue(
            path,
            "invalid_format",
            "Invalid string: must match pattern /^[1-9]\\d*$/",
        );
        return None;
    }
    let id = text.parse::<i64>().ok();
    if id.is_none() {
        input.issue(path, "custom", "Must be a signed bigint id");
    }
    id
}

/// `POSTGRES_TIMESTAMP_WITH_OFFSET` + `isValidPostgresTimestampWithOffset`:
/// `YYYY-MM-DD[ T]HH:MM:SS[.f{1,6}](Z|±HH[[:]MM])` as epoch ms.
pub fn parse_sort_at(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    let digits = |from: usize, len: usize| {
        b.get(from..from + len)
            .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
    };
    let layout = text.is_ascii()
        && digits(0, 4)
        && b.get(4) == Some(&b'-')
        && digits(5, 2)
        && b.get(7) == Some(&b'-')
        && digits(8, 2)
        && matches!(b.get(10), Some(b' ' | b'T'))
        && digits(11, 2)
        && b.get(13) == Some(&b':')
        && digits(14, 2)
        && b.get(16) == Some(&b':')
        && digits(17, 2)
        // A leap second (`:60`) is not a valid timestamp here.
        && &text[17..19] <= "59";
    if !layout {
        return None;
    }
    let mut rest = &text[19..];
    let mut fraction = "";
    if let Some(after) = rest.strip_prefix('.') {
        let count = after.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=6).contains(&count) {
            return None;
        }
        fraction = &rest[..=count];
        rest = &after[count..];
    }
    let offset = match rest.as_bytes() {
        [b'Z'] => "Z".to_string(),
        [sign @ (b'+' | b'-'), tail @ ..]
            if tail.iter().all(|c| c.is_ascii_digit() || *c == b':') =>
        {
            let sign = char::from(*sign);
            match tail {
                [h1, h2] => format!("{sign}{}{}:00", char::from(*h1), char::from(*h2)),
                [h1, h2, m1, m2] | [h1, h2, b':', m1, m2]
                    if ![*h1, *h2, *m1, *m2].contains(&b':') =>
                {
                    format!(
                        "{sign}{}{}:{}{}",
                        char::from(*h1),
                        char::from(*h2),
                        char::from(*m1),
                        char::from(*m2)
                    )
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    parse_iso(&format!(
        "{}T{}{fraction}{offset}",
        &text[..10],
        &text[11..19]
    ))
}

/// One `ChannelPostFilterOption` and its `sortAt`; `None` for a channel
/// outside `channelTypes` (skipped, as `toFilterOptions`).
fn option(
    row: &Row<'_>,
    storage_url: &str,
) -> chatbotx_db::rusqlite::Result<Option<(Value, i64, i64)>> {
    let id: i64 = row.get(0)?;
    let channel: String = row.get(2)?;
    if !CHANNEL_TYPES.contains(&channel.as_str()) {
        tracing::warn!(
            channel_post_id = id,
            channel,
            "channel-post: skipping a post with an unknown channel"
        );
        return Ok(None);
    }
    let thumbnail: Option<String> = row.get(8)?;
    let item = json!({
        "caption": row.get::<_, Option<String>>(1)?,
        "channel": channel,
        "externalPostId": row.get::<_, String>(3)?,
        "id": id.to_string(),
        "inboxId": row.get::<_, i64>(4)?.to_string(),
        "inboxName": row.get::<_, String>(5)?,
        "permalink": row.get::<_, Option<String>>(6)?,
        "publishedAt": opt_date(row.get(7)?),
        "thumbnailUrl": thumbnail.and_then(|key| get_public_file_url(&key, storage_url).ok()),
    });
    Ok(Some((item, id, row.get(9)?)))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let storage_url = super::contacts::tenant_urls(state).storage_url;
    if route_id == "channelPosts.listSelected" {
        let mut input = super::contacts::bracket_query(call.params, call.query);
        let ids = selected_ids(&mut input);
        input.finish()?;
        let ids = id_list(&ids.unwrap_or_default());
        let data = tx::read(&state.db, move |conn| {
            let sql = format!(
                r#"{SELECT} WHERE p."workspaceId" = ?1 AND p."id" IN (SELECT value FROM json_each(?2)) ORDER BY p."id""#
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| BusinessError::Repo(e.into()))?;
            let rows = stmt
                .query_map(params![ws, ids], |row| option(row, &storage_url))
                .and_then(|rows| rows.collect::<chatbotx_db::rusqlite::Result<Vec<_>>>())
                .map_err(|e| BusinessError::Repo(e.into()))?;
            Ok(rows.into_iter().flatten().map(|(item, _, _)| item).collect::<Vec<_>>())
        })
        .await
        .map_err(|e| business_error(e, LISTING))?;
        return Ok(json(200, json!({ "data": data })));
    }

    let mut input = Input::from_query(call.params, call.query);
    let cursor = cursor(&mut input);
    let limit = input
        .opt_coerce_int("limit", Num::int(1.0, 100.0))
        .unwrap_or(30);
    let search = input.opt_string("search", Str::trimmed(1, 200));
    input.finish()?;
    let pattern = search.map(|s| like_contains(&s));
    let (sort_at, before_id) = cursor.unzip();
    let rows = tx::read(&state.db, move |conn| {
        let sql = format!(
            r#"{SELECT} WHERE p."workspaceId" = ?1
              AND (?2 IS NULL OR p."caption" LIKE ?2 ESCAPE '\')
              AND (?3 IS NULL OR COALESCE(p."publishedAt", p."createdAt") < ?3
                   OR (COALESCE(p."publishedAt", p."createdAt") = ?3 AND p."id" < ?4))
              ORDER BY COALESCE(p."publishedAt", p."createdAt") DESC, p."id" DESC LIMIT ?5"#
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| BusinessError::Repo(e.into()))?;
        let rows = stmt
            .query_map(params![ws, pattern, sort_at, before_id, limit + 1], |row| {
                option(row, &storage_url)
            })
            .and_then(|rows| rows.collect::<chatbotx_db::rusqlite::Result<Vec<_>>>())
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(rows)
    })
    .await
    .map_err(|e| business_error(e, LISTING))?;
    // The page is cut before unknown channels are dropped, as the original.
    let more = rows.len() > usize::try_from(limit).unwrap_or_default();
    let page: Vec<_> = rows
        .into_iter()
        .take(usize::try_from(limit).unwrap_or_default())
        .collect();
    let last = page
        .last()
        .and_then(|row| row.as_ref().map(|(_, id, at)| (*id, *at)));
    let mut out =
        json!({ "items": page.into_iter().flatten().map(|(item, _, _)| item).collect::<Vec<_>>() });
    if more && let Some((id, at)) = last {
        out["nextCursor"] = json!({ "id": id.to_string(), "sortAt": iso(at) });
    }
    Ok(json(200, out))
}

/// `cursor` (`cursor[id]`, `cursor[sortAt]` query keys): `(sortAt ms, id)`.
fn cursor(input: &mut Input) -> Option<(i64, i64)> {
    let id = input.get("cursor[id]").cloned();
    let sort_at = input.get("cursor[sortAt]").cloned();
    if id.is_none() && sort_at.is_none() {
        return None;
    }
    let base = key("cursor");
    let id = post_id(input, &child(&base, "id"), id.as_ref());
    let path = child(&base, "sortAt");
    let text = input.string_at(&path, sort_at.as_ref(), Str::default(), true)?;
    let Some(at) = parse_sort_at(&text) else {
        input.issue(
            &path,
            "invalid_format",
            "Must be a valid PostgreSQL timestamp with offset",
        );
        return None;
    };
    Some((at, id?))
}

/// `channelPostIds`: 1-100 unique post ids.
fn selected_ids(input: &mut Input) -> Option<Vec<i64>> {
    let path = key("ids");
    let Some(Value::Array(items)) = input.get("ids").cloned() else {
        let received = super::input::type_name(input.get("ids"));
        input.issue(
            &path,
            "invalid_type",
            &format!("Invalid input: expected array, received {received}"),
        );
        return None;
    };
    if items.is_empty() {
        input.issue(
            &path,
            "too_small",
            "Too small: expected array to have >=1 items",
        );
    } else if items.len() > 100 {
        input.issue(
            &path,
            "too_big",
            "Too big: expected array to have <=100 items",
        );
    }
    let before = input.issue_count();
    let ids: Vec<i64> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let mut at = path.clone();
            at.push(Seg::Index(index));
            post_id(input, &at, Some(item))
        })
        .collect();
    if input.issue_count() == before {
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        if unique.len() != ids.len() {
            input.issue(&path, "custom", "Post ids must be unique");
        }
    }
    Some(ids)
}
