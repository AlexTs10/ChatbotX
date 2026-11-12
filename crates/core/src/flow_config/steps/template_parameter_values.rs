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

/// `TemplateParameterSpec`: one value the caller has to fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateParameterSpec {
    pub key: String,
    pub component: String,
    pub kind: String,
    pub required: bool,
    pub placeholder: Option<String>,
}

/// `AppliedTemplateParameters`.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedTemplateParameters {
    pub params: Value,
    pub missing: Vec<String>,
    pub unknown: Vec<String>,
    pub invalid: Vec<String>,
    pub unsupported: Vec<String>,
}

/// `TemplateParameterValues`.
pub type TemplateParameterValues = std::collections::BTreeMap<String, String>;

/// `describeWaTemplateParameters(components)`.
pub fn describe_wa_template_parameters(components: &[Value]) -> Vec<TemplateParameterSpec> {
    todo!("P3.4b")
}

/// `applyWaTemplateParameterValues(components, values)`.
pub fn apply_wa_template_parameter_values(
    components: &[Value],
    values: &TemplateParameterValues,
) -> AppliedTemplateParameters {
    todo!("P3.4b")
}

/// `describeMessengerTemplateParameters(components)`.
pub fn describe_messenger_template_parameters(components: &[Value]) -> Vec<TemplateParameterSpec> {
    todo!("P3.4b")
}

/// `applyMessengerTemplateParameterValues(components, parameterFormat, values)`.
pub fn apply_messenger_template_parameter_values(
    components: &[Value],
    parameter_format: &str,
    values: &TemplateParameterValues,
) -> AppliedTemplateParameters {
    todo!("P3.4b")
}
