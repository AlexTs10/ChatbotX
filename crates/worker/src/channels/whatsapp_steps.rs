//! The WhatsApp flow steps beyond text / image / template:
//! `send-carousel.ts` + `interactive-carousel.ts` + `send-card.ts`,
//! `send-view-catalog.ts`, `whatsapp-option-list.ts`,
//! `whatsapp-call-button.ts`, `whatsapp-location.ts`, `whatsapp-flow.ts` and
//! `send-products.ts`.

use chatbotx_core::flow_config::steps::{
    WHATSAPP_CALL_BUTTON_BODY_MAX, WHATSAPP_CALL_BUTTON_LABEL_MAX, WHATSAPP_FLOW_BUTTON_MAX,
    WHATSAPP_LOCATION_LATITUDE_LIMIT, WHATSAPP_LOCATION_LONGITUDE_LIMIT,
    WHATSAPP_LOCATION_TEXT_MAX, WHATSAPP_OPTION_LIST_BUTTON_MAX,
    WHATSAPP_OPTION_LIST_DESCRIPTION_MAX, WHATSAPP_OPTION_LIST_TITLE_MAX,
    is_valid_whatsapp_location_coordinate, read_carousel_card_url_button,
    read_stranded_carousel_link_button,
};
use chatbotx_core::flow_config::util::append_code_to_magic_link;
use serde_json::{Map, Value, json};

use super::whatsapp::{
    WaMessage, button_messages, button_payload, clamp_text, interactive, limits, media_message,
    split_text, str_field, text_message,
};
use crate::channel::{ChannelResult, SendFailure, SendFlowStep};

/// `WHATSAPP_FLOW_MESSAGE_VERSION`.
const FLOW_MESSAGE_VERSION: &str = "3";
const ROW_ID_MAX_LENGTH: usize = 200;
const MAX_LIST_ROWS: usize = 10;
const CAROUSEL_CARD_TYPE: &str = "cta_url";
const CAROUSEL_MAIN_BODY: &str = ".";
const CAROUSEL_MIN: usize = 2;
const CAROUSEL_MAX: usize = 10;

/// JS `.slice(0, max)` (UTF-16 units, no trim).
fn slice16(value: &str, max: usize) -> String {
    let mut units = 0;
    value
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}

fn fail(message: &str) -> SendFailure {
    SendFailure::other(message)
}

fn array<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// `convertFlowStepWhatsappCallButton`: Meta's `voice_call` interactive.
pub fn call_button(step: &Value) -> Vec<WaMessage> {
    let text = slice16(
        str_field(step, "text").trim(),
        WHATSAPP_CALL_BUTTON_BODY_MAX,
    );
    let display = slice16(
        str_field(step, "buttonLabel").trim(),
        WHATSAPP_CALL_BUTTON_LABEL_MAX,
    );
    if text.is_empty() || display.is_empty() {
        return Vec::new();
    }
    vec![WaMessage::raw(
        "interactive",
        json!({ "type": "voice_call", "body": { "text": text },
            "action": { "name": "voice_call", "parameters": { "display_text": display } } }),
    )]
}

/// `convertFlowStepWhatsappLocation`: out-of-range coordinates send nothing.
pub fn location(step: &Value) -> Vec<WaMessage> {
    let valid = |key: &str, limit: usize| {
        step[key]
            .as_f64()
            .is_some_and(|v| is_valid_whatsapp_location_coordinate(v, limit as f64))
    };
    if !(valid("latitude", WHATSAPP_LOCATION_LATITUDE_LIMIT)
        && valid("longitude", WHATSAPP_LOCATION_LONGITUDE_LIMIT))
    {
        return Vec::new();
    }
    let mut location = json!({ "latitude": step["latitude"], "longitude": step["longitude"] });
    for key in ["name", "address"] {
        let clean = slice16(str_field(step, key).trim(), WHATSAPP_LOCATION_TEXT_MAX);
        if !clean.is_empty() {
            location[key] = Value::String(clean);
        }
    }
    vec![WaMessage::raw("location", location)]
}

/// `convertFlowStepWhatsappOptionList`: one list message, one section.
pub fn option_list(data: &SendFlowStep) -> ChannelResult<Vec<WaMessage>> {
    let step = &data.step;
    let button = slice16(
        str_field(step, "buttonLabel").trim(),
        WHATSAPP_OPTION_LIST_BUTTON_MAX,
    );
    let rows: Vec<Value> = array(step, "options")
        .iter()
        .map(|option| {
            let mut row = json!({
                "id": slice16(&button_payload(data, str_field(option, "id"), false), ROW_ID_MAX_LENGTH),
                "title": slice16(str_field(option, "title"), WHATSAPP_OPTION_LIST_TITLE_MAX),
            });
            let description = slice16(
                str_field(option, "description"),
                WHATSAPP_OPTION_LIST_DESCRIPTION_MAX,
            );
            if !description.is_empty() {
                row["description"] = Value::String(description);
            }
            row
        })
        .collect();
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    // `ActionList` / `ListSection` throw on these, failing the step.
    if button.is_empty() {
        return Err(fail("Button content cannot be an empty string"));
    }
    if rows.len() > MAX_LIST_ROWS {
        return Err(fail("ListSection can't have more than 10 rows"));
    }
    Ok(vec![interactive(
        "list",
        json!({ "button": button, "sections": [{ "rows": rows }] }),
        str_field(step, "text"),
        None,
    )])
}

/// `convertFlowStepWhatsappFlow`: a `flow` interactive that opens the start
/// screen; skipped when the Flow is gone from Meta.
pub fn whatsapp_flow(data: &SendFlowStep) -> ChannelResult<Vec<WaMessage>> {
    let step = &data.step;
    let flow = &step["flow"];
    let (Some(button), source_id, screen) = (
        array(step, "buttons").first(),
        str_field(flow, "sourceId"),
        str_field(flow, "startScreenId"),
    ) else {
        return Ok(skip_flow(step));
    };
    if source_id.is_empty() || screen.is_empty() {
        return Ok(skip_flow(step));
    }
    let cta = slice16(str_field(button, "label").trim(), WHATSAPP_FLOW_BUTTON_MAX);
    // ponytail: `ActionFlow` also rejects an emoji CTA; Meta rejects it too.
    if cta.is_empty() {
        return Err(fail("Flow CTA must be between 1 and 20 characters"));
    }
    Ok(vec![interactive(
        "flow",
        json!({ "name": "flow", "parameters": {
            "flow_message_version": FLOW_MESSAGE_VERSION,
            "flow_id": source_id,
            "flow_token": button_payload(data, str_field(button, "id"), false),
            "flow_cta": cta,
            "flow_action": "navigate",
            "flow_action_payload": { "screen": screen },
        }}),
        str_field(step, "text"),
        None,
    )])
}

fn skip_flow(step: &Value) -> Vec<WaMessage> {
    tracing::warn!(
        stepId = str_field(step, "id"),
        flowId = %step["flow"]["id"],
        "whatsappFlow step skipped: missing button, sourceId, or startScreenId — flow may have been deleted from Meta"
    );
    Vec::new()
}

/// `convertFlowStepViewCatalog`: `catalog_message` with an optional
/// thumbnail product and footer.
pub fn view_catalog(step: &Value) -> Vec<WaMessage> {
    if str_field(step, "text").trim().is_empty() {
        return Vec::new();
    }
    let mut action = json!({ "name": "catalog_message" });
    let thumbnail = str_field(step, "thumbnailProductId").trim();
    if !thumbnail.is_empty() {
        action["parameters"] = json!({ "thumbnail_product_retailer_id": thumbnail });
    }
    let mut message = interactive("catalog_message", action, str_field(step, "text"), None);
    let footer = str_field(step, "footer").trim();
    if !footer.is_empty() {
        message.payload["footer"] = json!({ "text": clamp_text(footer, limits::FOOTER_TEXT) });
    }
    vec![message]
}

// ---- cards and carousels ---------------------------------------------------

/// `readCardContent`.
struct CardContent {
    body: String,
    footer: Option<String>,
    image_url: Option<String>,
    caption: String,
}

fn read_card_content(card: &Value) -> CardContent {
    let title = str_field(card, "title").trim().to_owned();
    let subtitle = str_field(card, "subtitle").trim().to_owned();
    let image_url = str_field(&card["image"], "url").trim().to_owned();
    CardContent {
        body: if title.is_empty() {
            subtitle.clone()
        } else {
            title.clone()
        },
        footer: (!title.is_empty() && !subtitle.is_empty()).then(|| subtitle.clone()),
        image_url: (!image_url.is_empty()).then_some(image_url),
        caption: [title, subtitle]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// `send-card.ts` `generateOutgoingMessages`: the richest native layout.
fn card_messages(data: &SendFlowStep, card: &Value, quick: &[Value]) -> Vec<WaMessage> {
    let content = read_card_content(card);
    let buttons = array(card, "buttons");
    let has_replies = !buttons.is_empty() || !quick.is_empty();
    if has_replies && !content.body.is_empty() {
        let messages = button_messages(
            data,
            buttons,
            quick,
            &content.body,
            content
                .image_url
                .as_deref()
                .map(|url| media_message("image", url)),
            content.footer.as_deref(),
        );
        if !messages.is_empty() {
            return messages;
        }
    }
    if let Some(url) = &content.image_url {
        let mut chunks = split_text(&content.caption, limits::CAPTION).into_iter();
        let mut image = media_message("image", url);
        if let Some(caption) = chunks.next() {
            image.payload["caption"] = Value::String(caption);
        }
        return std::iter::once(image)
            .chain(chunks.map(|c| text_message(&c)))
            .collect();
    }
    split_text(&content.caption, limits::TEXT)
        .iter()
        .map(|c| text_message(c))
        .collect()
}

/// `buildCardAction`: one link button keeps its URL; otherwise every button
/// is a quick reply (a stranded link is logged and dropped).
fn card_action(data: &SendFlowStep, buttons: &[Value], card_index: usize) -> Option<Value> {
    if let Some((button, url)) = read_carousel_card_url_button(buttons) {
        let id = button_payload(data, str_field(&button, "id"), true);
        return Some(json!({ "name": CAROUSEL_CARD_TYPE, "parameters": {
            "display_text": clamp_text(str_field(&button, "label"), limits::BUTTON_TITLE),
            "url": append_code_to_magic_link(&url, &id),
        }}));
    }
    if let Some(stranded) = read_stranded_carousel_link_button(buttons) {
        tracing::warn!(
            flowId = data.flow_id,
            flowVersionId = ?data.flow_version_id,
            cardIndex = card_index,
            buttonId = str_field(stranded, "id"),
            "Carousel card mixes a link button with other buttons; sending every button as a reply and dropping the link"
        );
    }
    let replies: Vec<Value> = buttons
        .iter()
        .map(|b| {
            json!({ "type": "quick_reply", "quick_reply": {
                "id": clamp_text(&button_payload(data, str_field(b, "id"), false), limits::BUTTON_ID),
                "title": clamp_text(str_field(b, "label"), limits::BUTTON_TITLE),
            }})
        })
        .collect();
    (!replies.is_empty()).then(|| json!({ "buttons": replies }))
}

fn carousel_card(data: &SendFlowStep, card: &Value, card_index: usize) -> Value {
    let content = read_card_content(card);
    let mut out = Map::new();
    out.insert("card_index".into(), json!(card_index));
    out.insert("type".into(), json!(CAROUSEL_CARD_TYPE));
    if let Some(url) = content.image_url {
        out.insert(
            "header".into(),
            json!({ "type": "image", "image": { "link": url } }),
        );
    }
    let body = clamp_text(&content.caption, limits::CAROUSEL_CARD_BODY);
    if !body.is_empty() {
        out.insert("body".into(), json!({ "text": body }));
    }
    if let Some(action) = card_action(data, array(card, "buttons"), card_index) {
        out.insert("action".into(), action);
    }
    Value::Object(out)
}

/// `chunkCarouselCards`: chunks of 2..10 (11 cards = 9 + 2).
pub fn chunk_carousel_cards<T: Clone>(cards: &[T]) -> Vec<Vec<T>> {
    let mut chunks: Vec<Vec<T>> = cards.chunks(CAROUSEL_MAX).map(<[T]>::to_vec).collect();
    let n = chunks.len();
    if n >= 2
        && chunks[n - 1].len() == 1
        && let Some(moved) = chunks[n - 2].pop()
    {
        chunks[n - 1].insert(0, moved);
    }
    chunks
}

/// `send-carousel.ts`: a native carousel from 2 cards, else legacy cards
/// (node quick replies on the last one).
pub fn carousel(data: &SendFlowStep, quick: &[Value]) -> Vec<WaMessage> {
    let cards = array(&data.step, "cards");
    if cards.len() >= CAROUSEL_MIN {
        return chunk_carousel_cards(cards)
            .iter()
            .map(|chunk| {
                WaMessage::raw(
                    "interactive",
                    json!({ "type": "carousel", "body": { "text": CAROUSEL_MAIN_BODY },
                        "action": { "cards": chunk.iter().enumerate()
                            .map(|(i, card)| carousel_card(data, card, i)).collect::<Vec<_>>() } }),
                )
            })
            .collect();
    }
    let last = cards.len().saturating_sub(1);
    cards
        .iter()
        .enumerate()
        .flat_map(|(i, card)| card_messages(data, card, if i == last { quick } else { &[] }))
        .collect()
}

// ---- products (`send-products.ts`) ----------------------------------------

fn catalog_id_of<'a>(items: &[&'a Value]) -> ChannelResult<&'a str> {
    items
        .first()
        .map(|i| str_field(&i["catalogReference"], "catalogId"))
        .filter(|c| !c.is_empty())
        .ok_or_else(|| fail("A WhatsApp product message needs at least one product"))
}

fn text_part(message: &Value, key: &str) -> Option<Value> {
    message
        .get(key)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(|t| json!({ "text": t }))
}

/// `toWhatsappProductMessage`: a resolved `ResolvedProductMessage`.
pub fn product_message(message: &Value) -> ChannelResult<WaMessage> {
    let retailer = |item: &Value| str_field(&item["catalogReference"], "retailerId").to_owned();
    match str_field(message, "display") {
        "products" => {
            let sections = array(message, "sections");
            let items: Vec<&Value> = sections.iter().flat_map(|s| array(s, "items")).collect();
            let mut interactive = Map::new();
            if let [only] = items.as_slice() {
                interactive.insert("type".into(), json!("product"));
                interactive.insert(
                    "action".into(),
                    json!({ "catalog_id": str_field(&only["catalogReference"], "catalogId"),
                        "product_retailer_id": retailer(only) }),
                );
                if let Some(body) = text_part(message, "body") {
                    interactive.insert("body".into(), body);
                }
            } else {
                let header = str_field(message, "header");
                let Some(body) = text_part(message, "body").filter(|_| !header.is_empty()) else {
                    return Err(fail(
                        "A WhatsApp multi-product message needs a header and a body",
                    ));
                };
                interactive.insert("type".into(), json!("product_list"));
                interactive.insert("header".into(), json!({ "type": "text", "text": header }));
                interactive.insert("body".into(), body);
                interactive.insert(
                    "action".into(),
                    json!({ "catalog_id": catalog_id_of(&items)?, "sections": sections.iter().map(|s| {
                        let mut section = Map::new();
                        if let Some(title) = s.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                            section.insert("title".into(), json!(title));
                        }
                        section.insert("product_items".into(), array(s, "items").iter()
                            .map(|i| json!({ "product_retailer_id": retailer(i) })).collect());
                        Value::Object(section)
                    }).collect::<Vec<_>>() }),
                );
            }
            if let Some(footer) = text_part(message, "footer") {
                interactive.insert("footer".into(), footer);
            }
            Ok(WaMessage::raw("interactive", Value::Object(interactive)))
        }
        "carousel" => {
            let items: Vec<&Value> = array(message, "items").iter().collect();
            let catalog_id = catalog_id_of(&items)?;
            Ok(WaMessage::raw(
                "interactive",
                json!({ "type": "carousel", "body": { "text": message["body"] },
                    "action": { "cards": items.iter().enumerate().map(|(i, item)| json!({
                        "card_index": i, "type": "product",
                        "action": { "product_retailer_id": retailer(item), "catalog_id": catalog_id },
                    })).collect::<Vec<_>>() } }),
            ))
        }
        _ => Err(SendFailure::Other(format!(
            "Unsupported product message: {message}"
        ))),
    }
}

/// `convertFlowStepSendProducts`.
pub fn send_products(data: &SendFlowStep) -> ChannelResult<Vec<WaMessage>> {
    let message = data.product_message.as_ref().ok_or_else(|| {
        fail("sendProducts reached the WhatsApp handler without resolved products")
    })?;
    Ok(vec![product_message(message)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carousel_chunks_keep_two_to_ten() {
        let sizes = |n: usize| {
            chunk_carousel_cards(&vec![0; n])
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>()
        };
        assert_eq!(sizes(11), vec![9, 2]);
        assert_eq!(sizes(10), vec![10]);
        assert_eq!(sizes(21), vec![10, 9, 2]);
        assert_eq!(sizes(3), vec![3]);
    }

    #[test]
    fn location_drops_bad_coordinates() {
        assert!(location(&json!({ "latitude": 91.0, "longitude": 0.0 })).is_empty());
        let out = location(
            &json!({ "latitude": 1.5, "longitude": 2.5, "name": " Shop ", "address": "" }),
        );
        assert_eq!(
            out[0].payload,
            json!({ "latitude": 1.5, "longitude": 2.5, "name": "Shop" })
        );
        assert!(out[0].raw);
    }

    #[test]
    fn call_button_needs_text_and_label() {
        assert!(call_button(&json!({ "text": " ", "buttonLabel": "Call" })).is_empty());
        let out = call_button(&json!({ "text": "Hi", "buttonLabel": "Call us" }));
        assert_eq!(
            out[0].payload["action"],
            json!({ "name": "voice_call", "parameters": { "display_text": "Call us" } })
        );
    }

    #[test]
    fn view_catalog_thumbnail_and_footer() {
        let out =
            view_catalog(&json!({ "text": "Shop", "thumbnailProductId": " sku ", "footer": "" }));
        assert_eq!(
            out[0].payload["action"],
            json!({ "name": "catalog_message", "parameters": { "thumbnail_product_retailer_id": "sku" } })
        );
        assert!(out[0].payload.get("footer").is_none());
        assert!(view_catalog(&json!({ "text": " " })).is_empty());
    }

    #[test]
    fn products_single_list_and_carousel() {
        let item = |r: &str| json!({ "catalogReference": { "catalogId": "c1", "retailerId": r } });
        let single = product_message(&json!({ "display": "products", "body": "B",
            "sections": [{ "items": [item("a")] }] }))
        .unwrap();
        assert_eq!(single.payload["type"], "product");
        assert_eq!(single.payload["action"]["product_retailer_id"], "a");
        assert!(
            product_message(&json!({ "display": "products", "body": "B",
                "sections": [{ "items": [item("a"), item("b")] }] }))
            .is_err()
        );
        let carousel = product_message(&json!({ "display": "carousel", "body": "B",
            "items": [item("a"), item("b")] }))
        .unwrap();
        assert_eq!(
            carousel.payload["action"]["cards"][1]["action"]["catalog_id"],
            "c1"
        );
    }

    #[test]
    fn slice16_counts_utf16() {
        assert_eq!(slice16("a😀b", 2), "a");
        assert_eq!(slice16("abc", 2), "ab");
    }
}
