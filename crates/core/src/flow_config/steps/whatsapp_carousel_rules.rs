//! Port of `packages/flow-config/src/steps/whatsapp-carousel-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn whatsapp_carousel_card_limits() -> &'static Value {
    static V: LazyLock<Value> =
        LazyLock::new(|| serde_json::from_str(r#"{"max":10,"min":2}"#).expect("valid JSON"));
    &V
}

/// `readCarouselCardUrlButton`: the single link button of a card.
pub fn read_carousel_card_url_button(buttons: &[Value]) -> Option<(Value, String)> {
    let button = buttons.first()?;
    if buttons.len() != 1 || button["buttonType"] != "openWebsite" {
        return None;
    }
    Some((
        button.clone(),
        str_field(&button["beforeStep"], "url").to_owned(),
    ))
}

/// `readStrandedCarouselLinkButton`: a link button next to other buttons.
pub fn read_stranded_carousel_link_button(buttons: &[Value]) -> Option<&Value> {
    if buttons.len() > 1 {
        buttons.iter().find(|b| is_link_button(b))
    } else {
        None
    }
}

fn read_card_button_signature(buttons: &[Value]) -> String {
    if buttons.len() == 1 && is_link_button(&buttons[0]) {
        "link".to_owned()
    } else {
        format!("reply:{}", buttons.len())
    }
}

/// `refineWhatsappCarouselStep(step, ctx)`.
pub fn refine_whatsapp_carousel_step(step: &Value, ctx: &mut RefineCtx) {
    let cards = arr_field(step, "cards");
    if let Some(stranded) = cards
        .iter()
        .position(|c| read_stranded_carousel_link_button(arr_field(c, "buttons")).is_some())
    {
        ctx.issue(
            path!["cards", stranded, "buttons"],
            "whatsappCarouselLinkButtonNotAlone",
        );
    }
    if cards.len() >= 2 {
        let expected =
            read_card_button_signature(cards.first().map_or(&[][..], |c| arr_field(c, "buttons")));
        if !cards
            .iter()
            .all(|c| read_card_button_signature(arr_field(c, "buttons")) == expected)
        {
            ctx.issue(path!["cards"], "whatsappCarouselButtonsMismatch");
        }
    }
}
