//! Port of `packages/flow-config/src/channel-rules/send-products-validator.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `sendProductsValidator`: structure everywhere, the product message rules
/// on channels that declare them.
pub fn send_products_validator() -> StepValidator {
    static V: LazyLock<StepValidator> = LazyLock::new(|| {
        let structural = send_products_step_schema().super_refine(refine_send_products_structure);
        let overrides = channel_flow_policies()
            .iter()
            .filter_map(|(channel, policy)| {
                let capability = policy.product_messages.clone()?;
                Some((
                    (*channel).to_owned(),
                    structural
                        .clone()
                        .super_refine(refine_product_message(capability)),
                ))
            })
            .collect();
        StepValidator::PerChannel {
            base: structural,
            overrides,
        }
    });
    V.clone()
}
