//! Port of `packages/flow-config/src/validation-codes.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn flow_validation_codes() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"whatsappCarouselButtonsMismatch":"whatsappCarouselButtonsMismatch","whatsappCarouselLinkButtonNotAlone":"whatsappCarouselLinkButtonNotAlone","tiktokCardTitleTooLong":"tiktokCardTitleTooLong","sendTextTooLongForChannel":"sendTextTooLongForChannel","sendViewCatalogBodyTooLong":"sendViewCatalogBodyTooLong","sendViewCatalogFooterTooLong":"sendViewCatalogFooterTooLong","mediaButtonsUnsupported":"mediaButtonsUnsupported","unsupportedBlock":"unsupportedBlock","constraintExceeded":"constraintExceeded","waTemplateMpmNoProducts":"waTemplateMpmNoProducts","waTemplateMpmTooManySections":"waTemplateMpmTooManySections","waTemplateMpmTooManyProducts":"waTemplateMpmTooManyProducts","waTemplateLtoExpirationRequired":"waTemplateLtoExpirationRequired","waTemplateMpmIncompleteProducts":"waTemplateMpmIncompleteProducts","waTemplatePaymentRequestIncomplete":"waTemplatePaymentRequestIncomplete","waTemplatePaymentRequestInvalid":"waTemplatePaymentRequestInvalid","waTemplatePaymentRequestMisplaced":"waTemplatePaymentRequestMisplaced","quickReplyNextStepRequired":"quickReplyNextStepRequired","quickReplyRetryMessageRequired":"quickReplyRetryMessageRequired","quickReplyRetryWithGetUserData":"quickReplyRetryWithGetUserData","whatsappFlowIncomplete":"whatsappFlowIncomplete","sendProductsDisplayUnsupported":"sendProductsDisplayUnsupported","sendProductsFieldRequired":"sendProductsFieldRequired","sendProductsFieldNotAllowed":"sendProductsFieldNotAllowed","sendProductsFieldTooLong":"sendProductsFieldTooLong","sendProductsProductCountOutOfRange":"sendProductsProductCountOutOfRange","sendProductsSectionCountOutOfRange":"sendProductsSectionCountOutOfRange","sendProductsDuplicateProduct":"sendProductsDuplicateProduct","templateButtonOpenMiniAppUnsupported":"templateButtonOpenMiniAppUnsupported","buttonCallPhoneNumberUnsupported":"buttonCallPhoneNumberUnsupported","buttonCopyTextUnsupported":"buttonCopyTextUnsupported","invalidPhoneNumber":"invalidPhoneNumber"}"#).expect("valid JSON")
    });
    &V
}

/// `isFlowValidationCode(message)`.
pub fn is_flow_validation_code(message: &str) -> bool {
    flow_validation_codes().get(message).is_some()
}
