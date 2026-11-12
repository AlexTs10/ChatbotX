//! `packages/utils/src/field-value-calculator`: one field operation to the
//! value to persist (pure; I/O belongs to the business layer).

pub mod date_expression;
pub mod math_expression;

use std::collections::HashMap;

use self::date_expression::{DateClock, DateUnit, add_date_unit, evaluate_date_expression};
use self::math_expression::evaluate_math_expression;
use super::custom_field::CustomFieldType;
use super::custom_field_operation::{
    FieldOperationType, field_operation_definition, is_field_operation_allowed,
};
use super::datetime::{date_part_of, is_real_calendar_date, resolve_filter_timezone};
use super::recurrence::{local_date_of, resolve_zoned_instant};
use crate::js;

string_enum! {
    /// `FieldValueRejection`.
    pub enum FieldValueRejection {
        OperationNotAllowed = "operationNotAllowed",
        OperandRequired = "operandRequired",
        OperandNotNumber = "operandNotNumber",
        CurrentNotNumber = "currentNotNumber",
        InvalidExpression = "invalidExpression",
        InvalidDate = "invalidDate",
        NonFiniteResult = "nonFiniteResult",
    }
}

/// `FieldValueCalculationInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldValueCalculationInput<'a> {
    pub field_type: CustomFieldType,
    pub operation: FieldOperationType,
    /// `""` and `None` both mean empty.
    pub current_value: Option<&'a str>,
    /// Already `{{variable}}`-resolved.
    pub operand: &'a str,
    pub clock: DateClock,
}

/// `FieldValueCalculationResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValueCalculationResult {
    Set(String),
    Clear,
    Unchanged,
    Rejected(FieldValueRejection),
}

use FieldValueCalculationResult as R;

fn parse_number(text: &str) -> Option<f64> {
    let value = js::to_number(text);
    (!js::trim(text).is_empty() && value.is_finite()).then_some(value)
}

fn current_number(current: Option<&str>) -> Option<f64> {
    match current {
        None | Some("") => Some(0.0),
        Some(text) => parse_number(text),
    }
}

fn finite_number_result(value: f64) -> FieldValueCalculationResult {
    if value.is_finite() {
        R::Set(js::number_to_string(value))
    } else {
        R::Rejected(FieldValueRejection::NonFiniteResult)
    }
}

fn temporal_result(
    field_type: CustomFieldType,
    instant: Option<i64>,
    time_zone: &str,
) -> FieldValueCalculationResult {
    match instant {
        None => R::Rejected(FieldValueRejection::InvalidDate),
        Some(at) => match field_type {
            CustomFieldType::Date => R::Set(local_date_of(at, time_zone)),
            _ => {
                js::to_iso_string(at).map_or(R::Rejected(FieldValueRejection::InvalidDate), R::Set)
            }
        },
    }
}

fn temporal_base(
    field_type: CustomFieldType,
    current: Option<&str>,
    clock: &DateClock,
) -> Option<i64> {
    let current = match current {
        None | Some("") => return Some(clock.now_ms),
        Some(c) => c,
    };
    if field_type == CustomFieldType::Date {
        let day = date_part_of(current);
        return is_real_calendar_date(day)
            .then(|| resolve_zoned_instant(day, "00:00", &clock.time_zone));
    }
    js::parse_date(current)
}

fn add_to_temporal(
    input: &FieldValueCalculationInput<'_>,
    unit: DateUnit,
    whole: bool,
) -> FieldValueCalculationResult {
    let amount = match parse_number(input.operand) {
        Some(a) if !whole || a.fract() == 0.0 => a,
        _ => return R::Rejected(FieldValueRejection::OperandNotNumber),
    };
    let base = temporal_base(input.field_type, input.current_value, &input.clock);
    let result = base.and_then(|b| add_date_unit(b, unit, amount, &input.clock.time_zone));
    temporal_result(input.field_type, result, &input.clock.time_zone)
}

/// `calculateFieldValue`.
pub fn calculate_field_value(
    input: &FieldValueCalculationInput<'_>,
) -> FieldValueCalculationResult {
    let clock = DateClock {
        now_ms: input.clock.now_ms,
        time_zone: resolve_filter_timezone(Some(&input.clock.time_zone)),
    };
    let input = FieldValueCalculationInput {
        clock,
        ..input.clone()
    };
    if !is_field_operation_allowed(input.operation, input.field_type) {
        return R::Rejected(FieldValueRejection::OperationNotAllowed);
    }
    if field_operation_definition(input.operation).requires_operand
        && js::trim(input.operand).is_empty()
    {
        return R::Rejected(FieldValueRejection::OperandRequired);
    }
    let concatenate = |join: fn(&str, &str) -> String| {
        if input.operand.is_empty() {
            R::Unchanged
        } else {
            R::Set(join(input.current_value.unwrap_or(""), input.operand))
        }
    };
    let shift = |sign: f64| {
        let current = current_number(input.current_value);
        let Some(amount) = parse_number(input.operand) else {
            return R::Rejected(FieldValueRejection::OperandNotNumber);
        };
        let Some(current) = current else {
            return R::Rejected(FieldValueRejection::CurrentNotNumber);
        };
        finite_number_result(current + sign * amount)
    };
    match input.operation {
        FieldOperationType::Set => R::Set(input.operand.to_owned()),
        FieldOperationType::Append => concatenate(|current, operand| format!("{current}{operand}")),
        FieldOperationType::Prepend => {
            concatenate(|current, operand| format!("{operand}{current}"))
        }
        FieldOperationType::Increase => shift(1.0),
        FieldOperationType::Decrease => shift(-1.0),
        FieldOperationType::Clear => R::Clear,
        FieldOperationType::Formula => {
            let Some(current) = current_number(input.current_value) else {
                return R::Rejected(FieldValueRejection::CurrentNotNumber);
            };
            let variables = HashMap::from([("value".to_owned(), current)]);
            match evaluate_math_expression(input.operand, &variables) {
                Some(value) => finite_number_result(value),
                None => R::Rejected(FieldValueRejection::InvalidExpression),
            }
        }
        FieldOperationType::SetNow => temporal_result(
            input.field_type,
            Some(input.clock.now_ms),
            &input.clock.time_zone,
        ),
        FieldOperationType::DateExpression => temporal_result(
            input.field_type,
            evaluate_date_expression(input.operand, &input.clock),
            &input.clock.time_zone,
        ),
        FieldOperationType::AddMinutes => add_to_temporal(&input, DateUnit::Minute, false),
        FieldOperationType::AddHours => add_to_temporal(&input, DateUnit::Hour, false),
        FieldOperationType::AddDays => add_to_temporal(&input, DateUnit::Day, true),
    }
}
