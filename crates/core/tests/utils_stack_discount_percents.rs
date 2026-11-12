//! Port of `packages/utils/__tests__/stack-discount-percents.test.ts`.

use chatbotx_core::utils::money::stack_discount_percents;

/// Port of: packages/utils/__tests__/stack-discount-percents.test.ts:5 "stackDiscountPercents > stacks the product and vendor discounts"
#[test]
fn stackdiscountpercents_stacks_the_product_and_vendor_discounts() {
    // `toBeCloseTo(28)`: 2 decimal digits.
    assert!((stack_discount_percents(10.0, 20.0) - 28.0).abs() < 0.005);
}

/// Port of: packages/utils/__tests__/stack-discount-percents.test.ts:9 "stackDiscountPercents > is the other discount alone when one is 0"
#[test]
fn stackdiscountpercents_is_the_other_discount_alone_when_one_is_0() {
    assert_eq!(stack_discount_percents(0.0, 15.0), 15.0);
    assert_eq!(stack_discount_percents(15.0, 0.0), 15.0);
}

/// Port of: packages/utils/__tests__/stack-discount-percents.test.ts:14 "stackDiscountPercents > never goes past 100"
#[test]
fn stackdiscountpercents_never_goes_past_100() {
    assert_eq!(stack_discount_percents(100.0, 50.0), 100.0);
}
