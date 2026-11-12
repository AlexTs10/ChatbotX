//! Port of `packages/flow-config/__tests__/payment-request.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::CollectPaymentRequestIssuesOptions;

#[test]
fn paymentamounttominorunits_s_s() {
    let cases: [(&str, Option<i64>); 7] = [
        ("12.34", Some(1234)),
        ("10", Some(1000)),
        ("0.5", Some(50)),
        ("0.01", Some(1)),
        (" 7.10 ", Some(710)),
        ("9007199254740991", None), // x100 overflows a safe integer
        ("90071992547409.91", Some(9_007_199_254_740_991)),
    ];
    for (amount, expected) in cases {
        assert_eq!(
            fc::payment_amount_to_minor_units(amount, None),
            expected,
            "{amount}"
        );
    }
}

#[test]
fn paymentamounttominorunits_rejects_j() {
    for amount in [
        "0",
        "0.00",
        "-1",
        "1.234",
        "abc",
        "1e3",
        "",
        ".5",
        "1,5",
        "99999999999999999999",
    ] {
        assert_eq!(
            fc::payment_amount_to_minor_units(amount, None),
            None,
            "{amount}"
        );
    }
}

#[test]
fn paymentamounttominorunits_honours_the_currency_s_fraction_digits() {
    assert_eq!(
        fc::payment_amount_to_minor_units("1500", Some(0)),
        Some(1500)
    );
    assert_eq!(fc::payment_amount_to_minor_units("1.5", Some(0)), None);
    assert_eq!(
        fc::payment_amount_to_minor_units("1.234", Some(3)),
        Some(1234)
    );
}

#[test]
fn paymentrequestschema_rejects_fields_a_method_does_not_carry() {
    assert!(
        !fc::payment_request_schema()
            .safe_parse(&json!({ "method": "boleto", "digitableLine": "1", "amount": "10" }))
            .success
    );
}

#[test]
fn createemptypaymentrequest_s_seed_parses_with_the_schema_and_is_not_shared() {
    for method in fc::payment_request_methods().enum_values() {
        let mut request = fc::create_empty_payment_request(&method);
        assert!(
            fc::payment_request_schema().safe_parse(&request).success,
            "{method}"
        );
        // A fresh value each call: changing one seed leaves the next intact.
        request["mutated"] = json!(true);
        assert!(
            fc::create_empty_payment_request(&method)
                .get("mutated")
                .is_none()
        );
    }
}

const EDITING: CollectPaymentRequestIssuesOptions = CollectPaymentRequestIssuesOptions {
    allow_variables: true,
};
const SENDING: CollectPaymentRequestIssuesOptions = CollectPaymentRequestIssuesOptions {
    allow_variables: false,
};

fn issues(request: &Value, options: CollectPaymentRequestIssuesOptions) -> Vec<(String, String)> {
    fc::collect_payment_request_issues(request, options)
        .into_iter()
        .map(|issue| (issue.field, issue.kind.to_owned()))
        .collect()
}

fn issue(field: &str, kind: &str) -> (String, String) {
    (field.to_owned(), kind.to_owned())
}

#[test]
fn collectpaymentrequestissues_blank_required_values_are_required_issues_optional_ones_are_not() {
    assert_eq!(
        issues(&fc::create_empty_payment_request("pix"), EDITING),
        vec![issue("code", "required")]
    );
    assert_eq!(
        issues(&fc::create_empty_payment_request("card_on_file"), EDITING),
        vec![
            issue("lastFourDigits", "required"),
            issue("amount", "required")
        ]
    );
}

#[test]
fn collectpaymentrequestissues_valid_requests_have_no_issues() {
    let requests = [
        json!({ "method": "pix", "code": "000201", "amount": "10.50" }),
        json!({ "method": "pix", "code": "000201" }),
        json!({ "method": "boleto", "digitableLine": "1234.5678" }),
        json!({ "method": "payment_link", "url": "https://pay.test/x" }),
        json!({ "method": "payment_link", "url": "myapp://pay/1" }),
        json!({ "method": "card_on_file", "lastFourDigits": "4242", "credentialId": "cred", "amount": "5" }),
    ];
    for request in requests {
        assert!(issues(&request, SENDING).is_empty(), "{request}");
    }
}

#[test]
fn collectpaymentrequestissues_flags_malformed_amount_url_and_last_four_digits() {
    assert_eq!(
        issues(
            &json!({ "method": "pix", "code": "x", "amount": "1.234" }),
            EDITING
        ),
        vec![issue("amount", "invalid")]
    );
    assert_eq!(
        issues(
            &json!({ "method": "payment_link", "url": "not a url" }),
            EDITING
        ),
        vec![issue("url", "invalid")]
    );
    assert_eq!(
        issues(
            &json!({ "method": "card_on_file", "lastFourDigits": "42", "amount": "5" }),
            EDITING
        ),
        vec![issue("lastFourDigits", "invalid")]
    );
}

#[test]
fn collectpaymentrequestissues_variables_are_accepted_while_editing_and_rejected_when_sending() {
    let request = json!({ "method": "payment_link", "url": "{{pay_url}}" });
    assert!(issues(&request, EDITING).is_empty());
    assert_eq!(issues(&request, SENDING), vec![issue("url", "unresolved")]);
    assert_eq!(
        issues(
            &json!({ "method": "pix", "code": "000{{unknown}}" }),
            SENDING
        ),
        vec![issue("code", "unresolved")]
    );
}

#[test]
fn collectpaymentrequestissues_whitespace_only_values_count_as_blank() {
    assert_eq!(
        issues(
            &json!({ "method": "boleto", "digitableLine": "  " }),
            SENDING
        ),
        vec![issue("digitableLine", "required")]
    );
}
