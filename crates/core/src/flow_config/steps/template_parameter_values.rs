// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/steps/template-parameter-values.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn template_parameter_kinds = z::enum_(&["text", "image", "video", "document", "latitude", "longitude", "location_name", "location_address", "url_suffix", "coupon_code", "catalog_product", "expiration_ms", "pix_code", "boleto_digitable_line", "payment_link_url", "payment_amount", "card_last_four_digits", "card_credential_id"]));

// HAND-PORT describeWaTemplateParameters:
// function describeWaTemplateParameters(components){return collectWaSlots(components).slots.map(toSpec)}

// HAND-PORT applyWaTemplateParameterValues:
// function applyWaTemplateParameterValues(components,values){const{slots,unsupported}=collectWaSlots(components);return applySlots({skeleton:extractTemplateParams(components),slots,unsupported,values})}

// HAND-PORT describeMessengerTemplateParameters:
// function describeMessengerTemplateParameters(components){return collectMessengerSlots(components).map(toSpec)}

// HAND-PORT applyMessengerTemplateParameterValues:
// function applyMessengerTemplateParameterValues(components,parameterFormat,values){return applySlots({skeleton:extractMessengerTemplateParams(components,parameterFormat),slots:collectMessengerSlots(components),unsupported:[],values})}
