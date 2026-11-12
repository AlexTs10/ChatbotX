//! Port of `packages/utils/__tests__/math-expression.test.ts`.
//! `{ ok: false }` is `None`; `{ ok: true, value }` is `Some(value)`.

use std::collections::HashMap;

use chatbotx_core::utils::field_value_calculator::math_expression::{
    MATH_EXPRESSION_LIMITS, evaluate_math_expression,
};

fn ones(count: usize) -> Vec<&'static str> {
    vec!["1"; count]
}

fn run(source: &str) -> Option<f64> {
    let variables = HashMap::from([("value".to_owned(), 10.0)]);
    evaluate_math_expression(source, &variables)
}

fn close_to(actual: Option<f64>, expected: f64, digits: i32) {
    let actual = actual.expect("a value");
    assert!(
        (actual - expected).abs() < 10f64.powi(-digits) / 2.0,
        "{actual} vs {expected}"
    );
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > 1 + 2 * 3 = 7"
#[test]
fn evaluatemathexpression_1_2_3_7() {
    assert_eq!(run("1 + 2 * 3"), Some(7.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > (1 + 2) * 3 = 9"
#[test]
fn evaluatemathexpression_1_2_3_9() {
    assert_eq!(run("(1 + 2) * 3"), Some(9.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > 10 - 4 - 3 = 3"
#[test]
fn evaluatemathexpression_10_4_3_3() {
    assert_eq!(run("10 - 4 - 3"), Some(3.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > 2 * 3 % 4 = 2"
#[test]
fn evaluatemathexpression_2_3_4_2() {
    assert_eq!(run("2 * 3 % 4"), Some(2.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > -3 + 5 = 2"
#[test]
fn evaluatemathexpression_3_5_2() {
    assert_eq!(run("-3 + 5"), Some(2.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > --3 = 3"
#[test]
fn evaluatemathexpression_3_3() {
    assert_eq!(run("--3"), Some(3.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > +4 = 4"
#[test]
fn evaluatemathexpression_4_4() {
    assert_eq!(run("+4"), Some(4.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > -(2 + 3) = -5"
#[test]
fn evaluatemathexpression_2_3_5() {
    assert_eq!(run("-(2 + 3)"), Some(-5.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > 1e2 + .5 = 100.5"
#[test]
fn evaluatemathexpression_1e2_5_100_5() {
    assert_eq!(run("1e2 + .5"), Some(100.5));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > VALUE * 2 = 20"
#[test]
fn evaluatemathexpression_value_2_20() {
    assert_eq!(run("VALUE * 2"), Some(20.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > value / 4 = 2.5"
#[test]
fn evaluatemathexpression_value_4_2_5() {
    assert_eq!(run("value / 4"), Some(2.5));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > pi = 3.141592653589793"
#[test]
fn evaluatemathexpression_pi_3_141592653589793() {
    assert_eq!(run("pi"), Some(std::f64::consts::PI));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > e = 2.718281828459045"
#[test]
fn evaluatemathexpression_e_2_718281828459045() {
    assert_eq!(run("e"), Some(std::f64::consts::E));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:17 "evaluateMathExpression > 2e3 = 2000"
#[test]
fn evaluatemathexpression_2e3_2000() {
    assert_eq!(run("2e3"), Some(2000.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > abs(-3) = 3"
#[test]
fn evaluatemathexpression_abs_3_3() {
    close_to(run("abs(-3)"), 3.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > ceil(1.2) = 2"
#[test]
fn evaluatemathexpression_ceil_1_2_2() {
    close_to(run("ceil(1.2)"), 2.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > floor(1.8) = 1"
#[test]
fn evaluatemathexpression_floor_1_8_1() {
    close_to(run("floor(1.8)"), 1.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > round(2.5) = 3"
#[test]
fn evaluatemathexpression_round_2_5_3() {
    close_to(run("round(2.5)"), 3.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > min(3, 1, 2) = 1"
#[test]
fn evaluatemathexpression_min_3_1_2_1() {
    close_to(run("min(3, 1, 2)"), 1.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > max(3, 1, 2) = 3"
#[test]
fn evaluatemathexpression_max_3_1_2_3() {
    close_to(run("max(3, 1, 2)"), 3.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > pow(2, 10) = 1024"
#[test]
fn evaluatemathexpression_pow_2_10_1024() {
    close_to(run("pow(2, 10)"), 1024.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > sqrt(81) = 9"
#[test]
fn evaluatemathexpression_sqrt_81_9() {
    close_to(run("sqrt(81)"), 9.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > logTen(1000) = 3"
#[test]
fn evaluatemathexpression_logten_1000_3() {
    close_to(run("logTen(1000)"), 3.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > hypot(3, 4) = 5"
#[test]
fn evaluatemathexpression_hypot_3_4_5() {
    close_to(run("hypot(3, 4)"), 5.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > fMod(7, 4) = 3"
#[test]
fn evaluatemathexpression_fmod_7_4_3() {
    close_to(run("fMod(7, 4)"), 3.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > round(value / 3) = 3"
#[test]
fn evaluatemathexpression_round_value_3_3() {
    close_to(run("round(value / 3)"), 3.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:36 "evaluateMathExpression > max(1, abs(-5) + 1) = 6"
#[test]
fn evaluatemathexpression_max_1_abs_5_1_6() {
    close_to(run("max(1, abs(-5) + 1)"), 6.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "" without throwing"
#[test]
fn evaluatemathexpression_rejects_without_throwing() {
    assert_eq!(run(""), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "   " without throwing"
#[test]
fn evaluatemathexpression_rejects_without_throwing_2() {
    assert_eq!(run("   "), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "1 +" without throwing"
#[test]
fn evaluatemathexpression_rejects_1_without_throwing() {
    assert_eq!(run("1 +"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "(1 + 2" without throwing"
#[test]
fn evaluatemathexpression_rejects_1_2_without_throwing() {
    assert_eq!(run("(1 + 2"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "1 2" without throwing"
#[test]
fn evaluatemathexpression_rejects_1_2_without_throwing_2() {
    assert_eq!(run("1 2"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "foo" without throwing"
#[test]
fn evaluatemathexpression_rejects_foo_without_throwing() {
    assert_eq!(run("foo"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "foo(1)" without throwing"
#[test]
fn evaluatemathexpression_rejects_foo_1_without_throwing() {
    assert_eq!(run("foo(1)"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "abs()" without throwing"
#[test]
fn evaluatemathexpression_rejects_abs_without_throwing() {
    assert_eq!(run("abs()"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "abs(1, 2)" without throwing"
#[test]
fn evaluatemathexpression_rejects_abs_1_2_without_throwing() {
    assert_eq!(run("abs(1, 2)"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "pow(1)" without throwing"
#[test]
fn evaluatemathexpression_rejects_pow_1_without_throwing() {
    assert_eq!(run("pow(1)"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "1 / 0" without throwing"
#[test]
fn evaluatemathexpression_rejects_1_0_without_throwing() {
    assert_eq!(run("1 / 0"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "5 % 0" without throwing"
#[test]
fn evaluatemathexpression_rejects_5_0_without_throwing() {
    assert_eq!(run("5 % 0"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "1e309" without throwing"
#[test]
fn evaluatemathexpression_rejects_1e309_without_throwing() {
    assert_eq!(run("1e309"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "pow(10, 400)" without throwing"
#[test]
fn evaluatemathexpression_rejects_pow_10_400_without_throwing() {
    assert_eq!(run("pow(10, 400)"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "sqrt(-1)" without throwing"
#[test]
fn evaluatemathexpression_rejects_sqrt_1_without_throwing() {
    assert_eq!(run("sqrt(-1)"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "1 $ 2" without throwing"
#[test]
fn evaluatemathexpression_rejects_1_2_without_throwing_3() {
    assert_eq!(run("1 $ 2"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "constructor" without throwing"
#[test]
fn evaluatemathexpression_rejects_constructor_without_throwing() {
    assert_eq!(run("constructor"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "__proto__" without throwing"
#[test]
fn evaluatemathexpression_rejects_proto_without_throwing() {
    assert_eq!(run("__proto__"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "toString(1)" without throwing"
#[test]
fn evaluatemathexpression_rejects_tostring_1_without_throwing() {
    assert_eq!(run("toString(1)"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "value.x" without throwing"
#[test]
fn evaluatemathexpression_rejects_value_x_without_throwing() {
    assert_eq!(run("value.x"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:54 "evaluateMathExpression > rejects "2 e" without throwing"
#[test]
fn evaluatemathexpression_rejects_2_e_without_throwing() {
    assert_eq!(run("2 e"), None);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:80 "evaluateMathExpression > enforces its limits"
#[test]
fn evaluatemathexpression_enforces_its_limits() {
    assert_eq!(
        run(&"1".repeat(MATH_EXPRESSION_LIMITS.max_length + 1)),
        None
    );
    assert_eq!(
        run(&ones(MATH_EXPRESSION_LIMITS.max_tokens).join("+")),
        None
    );
    assert_eq!(run(&format!("{}1{}", "(".repeat(40), ")".repeat(40))), None);
    assert_eq!(run(&format!("{}1", "-".repeat(40))), None);
    assert_eq!(
        run(&format!(
            "max({})",
            ones(MATH_EXPRESSION_LIMITS.max_arguments + 1).join(",")
        )),
        None
    );
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:94 "evaluateMathExpression > accepts the maximum number of arguments"
#[test]
fn evaluatemathexpression_accepts_the_maximum_number_of_arguments() {
    assert_eq!(
        run(&format!(
            "max({})",
            ones(MATH_EXPRESSION_LIMITS.max_arguments).join(",")
        )),
        Some(1.0)
    );
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:100 "evaluateMathExpression > never throws on arbitrary input"
#[test]
fn evaluatemathexpression_never_throws_on_arbitrary_input() {
    let alphabet: Vec<char> = "0123456789+-*/%(),. eEvalupitnsqrtabcm_$\n\t"
        .chars()
        .collect();
    // The JavaScript LCG runs in doubles; f64 reproduces it exactly.
    let mut seed = 42.0f64;
    let mut next = move || {
        seed = (seed * 1_103_515_245.0 + 12_345.0) % 2_147_483_648.0;
        seed
    };
    for _ in 0..3000 {
        let length = (next() % 40.0) as usize;
        let source: String = (0..length)
            .map(|_| alphabet[(next() % alphabet.len() as f64) as usize])
            .collect();
        let result = std::panic::catch_unwind(|| run(&source));
        assert!(result.is_ok(), "panicked on {source:?}");
    }
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:119 "every registered function > exp(0) = 1"
#[test]
fn every_registered_function_exp_0_1() {
    assert_eq!(run("exp(0)"), Some(1.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:119 "every registered function > sin(0) = 0"
#[test]
fn every_registered_function_sin_0_0() {
    assert_eq!(run("sin(0)"), Some(0.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:119 "every registered function > cos(0) = 1"
#[test]
fn every_registered_function_cos_0_1() {
    assert_eq!(run("cos(0)"), Some(1.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:119 "every registered function > tan(0) = 0"
#[test]
fn every_registered_function_tan_0_0() {
    assert_eq!(run("tan(0)"), Some(0.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:119 "every registered function > asin(0) = 0"
#[test]
fn every_registered_function_asin_0_0() {
    assert_eq!(run("asin(0)"), Some(0.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:119 "every registered function > acos(1) = 0"
#[test]
fn every_registered_function_acos_1_0() {
    assert_eq!(run("acos(1)"), Some(0.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:119 "every registered function > atan(0) = 0"
#[test]
fn every_registered_function_atan_0_0() {
    assert_eq!(run("atan(0)"), Some(0.0));
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:131 "every registered function > log(e) is 1"
#[test]
fn every_registered_function_log_e_is_1() {
    close_to(run("log(e)"), 1.0, 10);
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:135 "every registered function > log(0) is rejected"
#[test]
fn every_registered_function_log_0_is_rejected() {
    assert!(run("log(0)").is_none());
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:135 "every registered function > asin(2) is rejected"
#[test]
fn every_registered_function_asin_2_is_rejected() {
    assert!(run("asin(2)").is_none());
}

/// Port of: packages/utils/__tests__/math-expression.test.ts:135 "every registered function > acos(2) is rejected"
#[test]
fn every_registered_function_acos_2_is_rejected() {
    assert!(run("acos(2)").is_none());
}
