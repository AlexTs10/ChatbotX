//! Parity p1-07 10/11/12/15 and p1-12 31 (Q.P5-fixB): the coupon and
//! discount-code ids are `zodBigintAsString` (no length limit), so a 19-digit
//! unknown id is a 404 / business refusal, not a 422; an error message lists
//! its data in the original's key order.

mod handlers_common;

use chatbotx_api::handlers::business_error;
use handlers_common::{WS, call, fixture};
use serde_json::json;

const LONG: &str = "1000000000000000001";

#[tokio::test]
async fn a_19_digit_coupon_id_is_not_found() {
    let f = fixture().await;
    let (status, body) = call(&f, "coupons.getTopic", WS, &[("id", LONG)], None, None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (404, Some("notFound")),
        "{body}"
    );
    let (status, body) = call(
        &f,
        "coupons.bulkCreate",
        WS,
        &[],
        None,
        Some(json!({ "topicId": LONG, "codes": ["A-1"] })),
    )
    .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (400, Some("couponTopicInactive")),
        "{body}"
    );
    // Not digits is still a 422.
    let (status, _) = call(&f, "coupons.getTopic", WS, &[("id", "abc")], None, None).await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn a_19_digit_discount_code_id_is_not_found() {
    let f = fixture().await;
    let (status, body) = call(&f, "discountCodes.get", WS, &[("id", LONG)], None, None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (404, Some("notFound")),
        "{body}"
    );
    let (status, body) = call(
        &f,
        "discountCodes.create",
        WS,
        &[],
        None,
        Some(json!({
            "code": "LONGID1",
            "discountType": "percentage",
            "discountValue": 10,
            "appliesTo": "category",
            "categoryId": LONG,
        })),
    )
    .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (422, Some("validation")),
        "{body}"
    );
}

#[test]
fn payment_provider_in_use_lists_provider_then_count() {
    let error = business_error(
        chatbotx_business::payment_provider::in_use("stripe", 1),
        &[("paymentProviderInUse", 409)],
    );
    assert!(
        error.message.ends_with("(provider=stripe, count=1)"),
        "{}",
        error.message
    );
}
