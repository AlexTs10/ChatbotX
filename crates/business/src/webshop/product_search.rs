//! `webshop/product-search.ts`: "Search Products" — the products a
//! customer's message names, and the bot's answer with them.
//!
//! ponytail: no 60 s cache of the searchable names; one indexed read per
//! keyword miss. `\p{Diacritic}` reads as the combining marks block.

use std::sync::LazyLock;

use chatbotx_core::flow_config::get_channel_flow_policy;
use chatbotx_core::utils::id::create_id;
use chatbotx_core::utils::normalize_for_match::{nfc, nfd, strip_combining_marks};
use chatbotx_db::Db;
use regex::Regex;
use serde_json::{Value, json};

use crate::errors::Result;
use crate::{ecommerce_settings, order, tx};

const SEARCHABLE_NAMES_LIMIT: i64 = 2000;
const MIN_MATCH_LENGTH: usize = 3;
const MIN_PARTIAL_MATCH_LENGTH: usize = 4;
pub const PRODUCT_SEARCH_MAX_RESULTS: usize = 10;

static NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}]+").expect("word regex"));

fn normalize_words(text: &str) -> String {
    NON_WORD
        .replace_all(&nfc(text).to_lowercase(), " ")
        .trim()
        .to_owned()
}

/// `normalizeForSearch`.
pub fn normalize_for_search(text: &str) -> String {
    let folded: String = strip_combining_marks(&nfd(text))
        .chars()
        .map(|c| if c == 'đ' || c == 'Đ' { 'd' } else { c })
        .collect();
    normalize_words(&folded)
}

struct Word {
    text: String,
    folded: String,
    accented: bool,
}

fn words(text: &str) -> Vec<Word> {
    normalize_words(text)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let folded = normalize_for_search(w);
            Word {
                accented: folded != w,
                text: w.to_owned(),
                folded,
            }
        })
        .collect()
}

/// A word typed with accents must match them; one without matches either.
fn same(message: &Word, name: &Word) -> bool {
    if message.accented {
        message.text == name.text
    } else {
        message.folded == name.folded
    }
}

/// `inner` as consecutive whole words of `outer`; `same(outer, inner)`.
fn contains_run(outer: &[Word], inner: &[Word], same: impl Fn(&Word, &Word) -> bool) -> bool {
    !inner.is_empty()
        && inner.len() <= outer.len()
        && (0..=outer.len() - inner.len()).any(|start| {
            inner
                .iter()
                .enumerate()
                .all(|(i, w)| same(&outer[start + i], w))
        })
}

fn folded_len(words: &[Word]) -> usize {
    words
        .iter()
        .map(|w| w.folded.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .count()
}

/// `matchProductNames`: best first (the whole message is the name, the name
/// is in the message, the message is part of the name; then longer names).
pub fn match_product_names<'a>(
    message: &str,
    products: &'a [(i64, String)],
) -> Vec<&'a (i64, String)> {
    let message_words = words(message);
    let message_len = folded_len(&message_words);
    if message_len < MIN_MATCH_LENGTH {
        return Vec::new();
    }
    let mut scored: Vec<(u8, usize, usize, &(i64, String))> = products
        .iter()
        .enumerate()
        .filter_map(|(index, product)| {
            let name_words = words(&product.1);
            let len = folded_len(&name_words);
            if len < MIN_MATCH_LENGTH {
                return None;
            }
            let name_in_message = contains_run(&message_words, &name_words, same);
            let score = if name_in_message && name_words.len() == message_words.len() {
                3
            } else if name_in_message {
                2
            } else if message_len >= MIN_PARTIAL_MATCH_LENGTH
                && contains_run(&name_words, &message_words, |n, m| same(m, n))
            {
                1
            } else {
                return None;
            };
            Some((score, len, index, product))
        })
        .collect();
    // A stable sort, as `Array.prototype.sort`.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    scored.into_iter().map(|(.., p)| p).collect()
}

/// `buildProductSearchReply`: a collection step where the channel shows one,
/// else the names above a button opening the shop on them. `None` = no
/// product named, or "Search Products" off.
pub async fn build_product_search_reply(
    db: &Db,
    app_url: &str,
    workspace_id: i64,
    channel: &str,
    text: &str,
) -> Result<Option<Value>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let (enabled, names, language) = tx::read(db, move |conn| {
        let settings = ecommerce_settings::get_in(conn, workspace_id)?;
        if settings["general"]["searchProducts"] != true {
            return Ok((false, Vec::new(), None));
        }
        let source = settings["general"]["storeSource"]
            .as_str()
            .map(str::to_owned);
        let names = chatbotx_db::repo::product::list_searchable_names(
            conn,
            workspace_id,
            SEARCHABLE_NAMES_LIMIT,
            source.as_deref(),
        )?;
        let language: Option<String> = conn
            .query_row(
                r#"SELECT "language" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .ok();
        Ok((true, names, language))
    })
    .await?;
    if !enabled {
        return Ok(None);
    }
    let products: Vec<&(i64, String)> = match_product_names(text, &names)
        .into_iter()
        .take(PRODUCT_SEARCH_MAX_RESULTS)
        .collect();
    if products.is_empty() {
        return Ok(None);
    }
    let shows_collection = get_channel_flow_policy(Some(channel))
        .and_then(|p| p.steps.get("sendCollection"))
        .is_some_and(|support| *support == "full");
    if shows_collection {
        return Ok(Some(json!({
            "id": create_id(),
            "stepType": "sendCollection",
            "dataType": "products",
            "collectionType": "manual",
            "conditions": { "name": "", "categoryId": null, "subcategoryId": null, "vendorId": null },
            "productIds": products.iter().map(|p| p.0.to_string()).collect::<Vec<_>>(),
        })));
    }
    let strings =
        &order::messages::strings(order::messages::language(language.as_deref())).product_search;
    let label = |key: &str| strings.get(key).cloned().unwrap_or_default();
    let target = match products.as_slice() {
        [one] => format!("/products/{}?token={{{{shop_token}}}}", one.0),
        many => format!(
            "?token={{{{shop_token}}}}&products={}",
            many.iter()
                .map(|p| p.0.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
    };
    let url = chatbotx_core::utils::web_url::join_app_url(
        app_url,
        &format!("/shop/{workspace_id}{target}"),
    );
    let mut lines = vec![label("found")];
    lines.extend(products.iter().map(|p| format!("• {}", p.1)));
    Ok(Some(json!({
        "id": create_id(),
        "stepType": "sendText",
        "text": lines.join("\n"),
        "buttons": [{
            "id": create_id(),
            "label": label("view"),
            "buttonType": "openWebsite",
            "beforeStep": { "id": create_id(), "stepType": "openWebsite", "url": url, "browserSize": 100 },
            "steps": [],
        }],
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(found: Vec<&(i64, String)>) -> Vec<&str> {
        found.into_iter().map(|p| p.1.as_str()).collect()
    }

    #[test]
    fn matches_whole_words_with_accents_where_typed() {
        let products = vec![
            (1, "Áo thun".to_owned()),
            (2, "Áo thun cotton".to_owned()),
            (3, "Cam".to_owned()),
            (4, "Set".to_owned()),
        ];
        assert_eq!(
            names(match_product_names("ao thun", &products)),
            vec!["Áo thun", "Áo thun cotton"]
        );
        assert_eq!(
            names(match_product_names("cho mình xem áo thun!", &products)),
            vec!["Áo thun"]
        );
        assert!(match_product_names("cảm ơn", &products).is_empty());
        assert_eq!(names(match_product_names("cam", &products)), vec!["Cam"]);
        assert!(match_product_names("se", &products).is_empty());
        assert_eq!(normalize_for_search("Đồ Ăn!"), "do an");
    }
}
