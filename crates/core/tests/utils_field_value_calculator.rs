//! Port of `packages/utils/__tests__/field-value-calculator.test.ts`.

mod support;

use chatbotx_core::utils::custom_field::CustomFieldType;
use chatbotx_core::utils::custom_field_operation::{
    FieldOperationType, field_operation_by_api_name, field_operation_codes,
    field_operation_definition, field_operations_for_type, is_field_operation_allowed,
};
use chatbotx_core::utils::field_value_calculator::date_expression::DateClock;
use chatbotx_core::utils::field_value_calculator::{
    FieldValueCalculationInput, FieldValueCalculationResult, FieldValueRejection,
    calculate_field_value,
};
use std::collections::HashSet;

use FieldValueCalculationResult as R;

fn now() -> i64 {
    support::iso_ms("2026-03-10T05:30:00.000Z")
}

fn clock(now_ms: i64, time_zone: &str) -> DateClock {
    DateClock {
        now_ms,
        time_zone: time_zone.to_owned(),
    }
}

/// `calculate({ fieldType: "number", currentValue: null, operand: "", clock, ...overrides })`.
struct Calc<'a> {
    field_type: CustomFieldType,
    operation: FieldOperationType,
    current_value: Option<&'a str>,
    operand: &'a str,
    clock: DateClock,
}

fn calc(operation: FieldOperationType) -> Calc<'static> {
    Calc {
        field_type: CustomFieldType::Number,
        operation,
        current_value: None,
        operand: "",
        clock: clock(now(), "UTC"),
    }
}

impl<'a> Calc<'a> {
    fn field(mut self, field_type: CustomFieldType) -> Self {
        self.field_type = field_type;
        self
    }
    fn current(mut self, value: Option<&'a str>) -> Self {
        self.current_value = value;
        self
    }
    fn operand(mut self, operand: &'a str) -> Self {
        self.operand = operand;
        self
    }
    fn clock(mut self, clock: DateClock) -> Self {
        self.clock = clock;
        self
    }
    fn run(self) -> FieldValueCalculationResult {
        calculate_field_value(&FieldValueCalculationInput {
            field_type: self.field_type,
            operation: self.operation,
            current_value: self.current_value,
            operand: self.operand,
            clock: self.clock,
        })
    }
}

fn set(value: &str) -> FieldValueCalculationResult {
    R::Set(value.to_owned())
}

fn op(code: &str) -> FieldOperationType {
    FieldOperationType::parse(code).unwrap()
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:33 "operation registry > O01-O05 keep their stored codes"
#[test]
fn operation_registry_o01_o05_keep_their_stored_codes() {
    assert_eq!(FieldOperationType::Set.as_str(), "O01");
    assert_eq!(FieldOperationType::Append.as_str(), "O02");
    assert_eq!(FieldOperationType::Prepend.as_str(), "O03");
    assert_eq!(FieldOperationType::Increase.as_str(), "O04");
    assert_eq!(FieldOperationType::Decrease.as_str(), "O05");
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:43 "operation registry > every operation resolves from its public api name and is unique"
#[test]
fn operation_registry_every_operation_resolves_from_its_public_api_name_and_is_unique() {
    let names: Vec<&str> = field_operation_codes()
        .iter()
        .map(|code| field_operation_definition(*code).api_name)
        .collect();
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
    for code in field_operation_codes() {
        assert_eq!(
            field_operation_by_api_name(field_operation_definition(*code).api_name),
            Some(*code)
        );
    }
    assert_eq!(field_operation_by_api_name("constructor"), None);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > shortText offers its supported operations"
#[test]
fn operation_registry_shorttext_offers_its_supported_operations() {
    let codes: Vec<&str> = field_operations_for_type(CustomFieldType::parse("shortText").unwrap())
        .iter()
        .map(|o| o.as_str())
        .collect();
    assert_eq!(codes, vec!["O01", "O02", "O03", "O06"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > longText offers its supported operations"
#[test]
fn operation_registry_longtext_offers_its_supported_operations() {
    let codes: Vec<&str> = field_operations_for_type(CustomFieldType::parse("longText").unwrap())
        .iter()
        .map(|o| o.as_str())
        .collect();
    assert_eq!(codes, vec!["O01", "O02", "O03", "O06"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > email offers its supported operations"
#[test]
fn operation_registry_email_offers_its_supported_operations() {
    let codes: Vec<&str> = field_operations_for_type(CustomFieldType::parse("email").unwrap())
        .iter()
        .map(|o| o.as_str())
        .collect();
    assert_eq!(codes, vec!["O01", "O06"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > phoneNumber offers its supported operations"
#[test]
fn operation_registry_phonenumber_offers_its_supported_operations() {
    let codes: Vec<&str> =
        field_operations_for_type(CustomFieldType::parse("phoneNumber").unwrap())
            .iter()
            .map(|o| o.as_str())
            .collect();
    assert_eq!(codes, vec!["O01", "O06"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > boolean offers its supported operations"
#[test]
fn operation_registry_boolean_offers_its_supported_operations() {
    let codes: Vec<&str> = field_operations_for_type(CustomFieldType::parse("boolean").unwrap())
        .iter()
        .map(|o| o.as_str())
        .collect();
    assert_eq!(codes, vec!["O01", "O06"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > number offers its supported operations"
#[test]
fn operation_registry_number_offers_its_supported_operations() {
    let codes: Vec<&str> = field_operations_for_type(CustomFieldType::parse("number").unwrap())
        .iter()
        .map(|o| o.as_str())
        .collect();
    assert_eq!(codes, vec!["O01", "O04", "O05", "O06", "O07"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > date offers its supported operations"
#[test]
fn operation_registry_date_offers_its_supported_operations() {
    let codes: Vec<&str> = field_operations_for_type(CustomFieldType::parse("date").unwrap())
        .iter()
        .map(|o| o.as_str())
        .collect();
    assert_eq!(codes, vec!["O01", "O06", "O08", "O09", "O10", "O11", "O12"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:56 "operation registry > datetime offers its supported operations"
#[test]
fn operation_registry_datetime_offers_its_supported_operations() {
    let codes: Vec<&str> = field_operations_for_type(CustomFieldType::parse("datetime").unwrap())
        .iter()
        .map(|o| o.as_str())
        .collect();
    assert_eq!(codes, vec!["O01", "O06", "O08", "O09", "O10", "O11", "O12"]);
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:71 "calculateFieldValue > rejects every operation on a type it does not allow"
#[test]
fn calculatefieldvalue_rejects_every_operation_on_a_type_it_does_not_allow() {
    for operation in field_operation_codes() {
        for field_type in CustomFieldType::ALL {
            if is_field_operation_allowed(*operation, *field_type) {
                continue;
            }
            assert_eq!(
                calc(*operation).field(*field_type).operand("1").run(),
                R::Rejected(FieldValueRejection::OperationNotAllowed)
            );
        }
    }
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:85 "calculateFieldValue > rejects a blank operand where one is required"
#[test]
fn calculatefieldvalue_rejects_a_blank_operand_where_one_is_required() {
    for operation in field_operation_codes() {
        let definition = field_operation_definition(*operation);
        if !definition.requires_operand {
            continue;
        }
        let field_type = definition.allowed_types[0];
        assert_eq!(
            calc(*operation).field(field_type).operand("  ").run(),
            R::Rejected(FieldValueRejection::OperandRequired)
        );
    }
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:100 "calculateFieldValue > text > set returns the operand, even when blank"
#[test]
fn calculatefieldvalue_text_set_returns_the_operand_even_when_blank() {
    assert_eq!(
        calc(op("O01"))
            .field(CustomFieldType::ShortText)
            .operand("")
            .run(),
        set("")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:106 "calculateFieldValue > text > O02 on "abc" with "xyz" gives "abcxyz""
#[test]
fn calculatefieldvalue_text_o02_on_abc_with_xyz_gives_abcxyz() {
    assert_eq!(
        calc(op("O02"))
            .field(CustomFieldType::LongText)
            .current(Some("abc"))
            .operand("xyz")
            .run(),
        set("abcxyz")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:106 "calculateFieldValue > text > O03 on "abc" with "xyz" gives "xyzabc""
#[test]
fn calculatefieldvalue_text_o03_on_abc_with_xyz_gives_xyzabc() {
    assert_eq!(
        calc(op("O03"))
            .field(CustomFieldType::LongText)
            .current(Some("abc"))
            .operand("xyz")
            .run(),
        set("xyzabc")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:106 "calculateFieldValue > text > O02 on null with "xyz" gives "xyz""
#[test]
fn calculatefieldvalue_text_o02_on_null_with_xyz_gives_xyz() {
    assert_eq!(
        calc(op("O02"))
            .field(CustomFieldType::LongText)
            .current(None)
            .operand("xyz")
            .run(),
        set("xyz")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:106 "calculateFieldValue > text > O03 on "" with "xyz" gives "xyz""
#[test]
fn calculatefieldvalue_text_o03_on_with_xyz_gives_xyz() {
    assert_eq!(
        calc(op("O03"))
            .field(CustomFieldType::LongText)
            .current(Some(""))
            .operand("xyz")
            .run(),
        set("xyz")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:122 "calculateFieldValue > text > an empty operand leaves the value unchanged"
#[test]
fn calculatefieldvalue_text_an_empty_operand_leaves_the_value_unchanged() {
    assert_eq!(
        calc(op("O02"))
            .field(CustomFieldType::ShortText)
            .current(Some("a"))
            .operand("")
            .run(),
        R::Unchanged
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O04 on "5" with "3" gives 8"
#[test]
fn calculatefieldvalue_number_o04_on_5_with_3_gives_8() {
    assert_eq!(
        calc(op("O04")).current(Some("5")).operand("3").run(),
        set("8")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O05 on "5" with "3" gives 2"
#[test]
fn calculatefieldvalue_number_o05_on_5_with_3_gives_2() {
    assert_eq!(
        calc(op("O05")).current(Some("5")).operand("3").run(),
        set("2")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O04 on null with "7" gives 7"
#[test]
fn calculatefieldvalue_number_o04_on_null_with_7_gives_7() {
    assert_eq!(calc(op("O04")).current(None).operand("7").run(), set("7"));
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O04 on "" with "7" gives 7"
#[test]
fn calculatefieldvalue_number_o04_on_with_7_gives_7() {
    assert_eq!(
        calc(op("O04")).current(Some("")).operand("7").run(),
        set("7")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O05 on "" with "7" gives -7"
#[test]
fn calculatefieldvalue_number_o05_on_with_7_gives_7() {
    assert_eq!(
        calc(op("O05")).current(Some("")).operand("7").run(),
        set("-7")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O05 on null with "7" gives -7"
#[test]
fn calculatefieldvalue_number_o05_on_null_with_7_gives_7() {
    assert_eq!(calc(op("O05")).current(None).operand("7").run(), set("-7"));
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O04 on "0.1" with "0.2" gives 0.30000000000000004"
#[test]
fn calculatefieldvalue_number_o04_on_0_1_with_0_2_gives_0_30000000000000004() {
    assert_eq!(
        calc(op("O04")).current(Some("0.1")).operand("0.2").run(),
        set("0.30000000000000004")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:135 "calculateFieldValue > number > O04 on " 5 " with "1e2" gives 105"
#[test]
fn calculatefieldvalue_number_o04_on_5_with_1e2_gives_105() {
    assert_eq!(
        calc(op("O04")).current(Some(" 5 ")).operand("1e2").run(),
        set("105")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:151 "calculateFieldValue > number > rejects a non-numeric operand or current value"
#[test]
fn calculatefieldvalue_number_rejects_a_non_numeric_operand_or_current_value() {
    assert_eq!(
        calc(op("O04")).current(Some("1")).operand("abc").run(),
        R::Rejected(FieldValueRejection::OperandNotNumber)
    );
    assert_eq!(
        calc(op("O04")).current(Some("abc")).operand("1").run(),
        R::Rejected(FieldValueRejection::CurrentNotNumber)
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:166 "calculateFieldValue > number > rejects a result that overflows"
#[test]
fn calculatefieldvalue_number_rejects_a_result_that_overflows() {
    assert_eq!(
        calc(op("O04"))
            .current(Some("1.7e308"))
            .operand("1.7e308")
            .run(),
        R::Rejected(FieldValueRejection::NonFiniteResult)
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:179 "calculateFieldValue > number > formula reads the current value"
#[test]
fn calculatefieldvalue_number_formula_reads_the_current_value() {
    assert_eq!(
        calc(op("O07"))
            .current(Some("4"))
            .operand("value * 2 + 1")
            .run(),
        set("9")
    );
    assert_eq!(
        calc(op("O07")).current(None).operand("value + 5").run(),
        set("5")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:196 "calculateFieldValue > number > formula rejects an invalid expression"
#[test]
fn calculatefieldvalue_number_formula_rejects_an_invalid_expression() {
    assert_eq!(
        calc(op("O07")).operand("1 +").run(),
        R::Rejected(FieldValueRejection::InvalidExpression)
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:204 "calculateFieldValue > clear works on every type"
#[test]
fn calculatefieldvalue_clear_works_on_every_type() {
    for field_type in CustomFieldType::ALL {
        assert_eq!(calc(op("O06")).field(*field_type).run(), R::Clear);
    }
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:213 "calculateFieldValue > temporal > set now stores an instant for datetime and the local day for date"
#[test]
fn calculatefieldvalue_temporal_set_now_stores_an_instant_for_datetime_and_the_local_day_for_date()
{
    assert_eq!(
        calc(op("O08")).field(CustomFieldType::Datetime).run(),
        set("2026-03-10T05:30:00.000Z")
    );
    assert_eq!(
        calc(op("O08"))
            .field(CustomFieldType::Date)
            .clock(clock(now(), "Asia/Ho_Chi_Minh"))
            .run(),
        set("2026-03-10")
    );
    assert_eq!(
        calc(op("O08"))
            .field(CustomFieldType::Date)
            .clock(clock(
                support::iso_ms("2026-03-10T20:00:00.000Z"),
                "Asia/Ho_Chi_Minh"
            ))
            .run(),
        set("2026-03-11")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:237 "calculateFieldValue > temporal > adds minutes, hours and days to the current value"
#[test]
fn calculatefieldvalue_temporal_adds_minutes_hours_and_days_to_the_current_value() {
    let current = Some("2026-03-10T00:00:00.000Z");
    let base = |code| {
        calc(op(code))
            .field(CustomFieldType::Datetime)
            .current(current)
    };
    assert_eq!(
        base("O10").operand("90").run(),
        set("2026-03-10T01:30:00.000Z")
    );
    assert_eq!(
        base("O11").operand("-2").run(),
        set("2026-03-09T22:00:00.000Z")
    );
    assert_eq!(
        base("O12").operand("3").run(),
        set("2026-03-13T00:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:254 "calculateFieldValue > temporal > an empty value is based on now"
#[test]
fn calculatefieldvalue_temporal_an_empty_value_is_based_on_now() {
    assert_eq!(
        calc(op("O12"))
            .field(CustomFieldType::Datetime)
            .operand("1")
            .run(),
        set("2026-03-11T05:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:264 "calculateFieldValue > temporal > a date field adds whole days in its own zone"
#[test]
fn calculatefieldvalue_temporal_a_date_field_adds_whole_days_in_its_own_zone() {
    assert_eq!(
        calc(op("O12"))
            .field(CustomFieldType::Date)
            .current(Some("2026-03-10T00:00:00+07:00"))
            .operand("1")
            .clock(clock(now(), "Asia/Ho_Chi_Minh"))
            .run(),
        set("2026-03-11")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:277 "calculateFieldValue > temporal > a date stored in another zone keeps its calendar day (never shifts)"
#[test]
fn calculatefieldvalue_temporal_a_date_stored_in_another_zone_keeps_its_calendar_day_never_shifts()
{
    for current_value in [
        "2024-03-10T00:00:00+07:00",
        "2024-03-10T00:00:00-05:00",
        "2024-03-10",
    ] {
        assert_eq!(
            calc(op("O12"))
                .field(CustomFieldType::Date)
                .current(Some(current_value))
                .operand("1")
                .clock(clock(now(), "America/Los_Angeles"))
                .run(),
            set("2024-03-11"),
            "{current_value}"
        );
    }
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:296 "calculateFieldValue > temporal > a date with an invalid stored day is rejected"
#[test]
fn calculatefieldvalue_temporal_a_date_with_an_invalid_stored_day_is_rejected() {
    assert_eq!(
        calc(op("O12"))
            .field(CustomFieldType::Date)
            .current(Some("not-a-date"))
            .operand("1")
            .run(),
        R::Rejected(FieldValueRejection::InvalidDate)
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:307 "calculateFieldValue > temporal > rejects a fractional day count, an unparsable value and out-of-range results"
#[test]
fn calculatefieldvalue_temporal_rejects_a_fractional_day_count_an_unparsable_value_and_out_of_range_results()
 {
    assert_eq!(
        calc(op("O12"))
            .field(CustomFieldType::Date)
            .operand("1.5")
            .run(),
        R::Rejected(FieldValueRejection::OperandNotNumber)
    );
    assert_eq!(
        calc(op("O10"))
            .field(CustomFieldType::Datetime)
            .current(Some("garbage"))
            .operand("1")
            .run(),
        R::Rejected(FieldValueRejection::InvalidDate)
    );
    assert_eq!(
        calc(op("O11"))
            .field(CustomFieldType::Datetime)
            .operand("99999999999")
            .run(),
        R::Rejected(FieldValueRejection::InvalidDate)
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:331 "calculateFieldValue > temporal > falls back to UTC for an unusable timezone instead of throwing"
#[test]
fn calculatefieldvalue_temporal_falls_back_to_utc_for_an_unusable_timezone_instead_of_throwing() {
    assert_eq!(
        calc(op("O08"))
            .field(CustomFieldType::Date)
            .clock(clock(now(), "Not/AZone"))
            .run(),
        set("2026-03-10")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:341 "calculateFieldValue > temporal > date expressions compute from now"
#[test]
fn calculatefieldvalue_temporal_date_expressions_compute_from_now() {
    assert_eq!(
        calc(op("O09"))
            .field(CustomFieldType::Datetime)
            .operand("now + 1 day - 30 min")
            .run(),
        set("2026-03-11T05:00:00.000Z")
    );
    assert_eq!(
        calc(op("O09"))
            .field(CustomFieldType::Datetime)
            .operand("nonsense")
            .run(),
        R::Rejected(FieldValueRejection::InvalidDate)
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:370 "fractional temporal operands > add minutes and hours accept fractions"
#[test]
fn fractional_temporal_operands_add_minutes_and_hours_accept_fractions() {
    let add_to = |operation, operand| {
        calc(operation)
            .field(CustomFieldType::Datetime)
            .current(Some("2026-01-01T00:00:00.000Z"))
            .operand(operand)
            .run()
    };
    assert_eq!(
        add_to(FieldOperationType::AddMinutes, "0.5"),
        set("2026-01-01T00:00:30.000Z")
    );
    assert_eq!(
        add_to(FieldOperationType::AddHours, "0.5"),
        set("2026-01-01T00:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:381 "fractional temporal operands > add days rejects a fraction"
#[test]
fn fractional_temporal_operands_add_days_rejects_a_fraction() {
    assert_eq!(
        calc(FieldOperationType::AddDays)
            .field(CustomFieldType::Datetime)
            .current(Some("2026-01-01T00:00:00.000Z"))
            .operand("0.5")
            .run(),
        R::Rejected(FieldValueRejection::OperandNotNumber)
    );
}

/// Port of: packages/utils/__tests__/field-value-calculator.test.ts:390 "non-finite numbers > a stored Infinity is not a number"
#[test]
fn non_finite_numbers_a_stored_infinity_is_not_a_number() {
    assert_eq!(
        calc(FieldOperationType::Formula)
            .current(Some("Infinity"))
            .operand("1 / value")
            .run(),
        R::Rejected(FieldValueRejection::CurrentNotNumber)
    );
}
