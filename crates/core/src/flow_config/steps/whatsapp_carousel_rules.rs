//! Port of `packages/flow-config/src/steps/whatsapp-carousel-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn whatsapp_carousel_card_limits() -> &'static Value {
    static V: LazyLock<Value> =
        LazyLock::new(|| serde_json::from_str(r#"{"max":10,"min":2}"#).expect("valid JSON"));
    &V
}

// HAND-PORT readCarouselCardUrlButton:
// buttons=>{const[button]=buttons;if(buttons.length!==1||button?.buttonType!==buttonTypes.enum.openWebsite){return}return{button,url:button.beforeStep.url}}

// HAND-PORT readStrandedCarouselLinkButton:
// buttons=>buttons.length>1?buttons.find(isLinkButton):void 0

// HAND-PORT refineWhatsappCarouselStep:
// (step,ctx)=>{const strandedIndex=step.cards.findIndex(card=>readStrandedCarouselLinkButton(card.buttons)!==void 0);if(strandedIndex!==-1){ctx.addIssue({code:"custom",message:flowValidationCodes.whatsappCarouselLinkButtonNotAlone,path:["cards",strandedIndex,"buttons"]})}if(step.cards.length>=whatsappCarouselCardLimits.min&&!hasMatchingButtons(step.cards)){ctx.addIssue({code:"custom",message:flowValidationCodes.whatsappCarouselButtonsMismatch,path:["cards"]})}}
