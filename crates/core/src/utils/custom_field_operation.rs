//! `packages/utils/src/custom-field-operation.ts`.

use super::custom_field::CustomFieldType;

string_enum! {
    /// `FieldOperationType`: codes persisted in flows and trigger actions.
    pub enum FieldOperationType {
        Set = "O01",
        Append = "O02",
        Prepend = "O03",
        Increase = "O04",
        Decrease = "O05",
        Clear = "O06",
        Formula = "O07",
        SetNow = "O08",
        DateExpression = "O09",
        AddMinutes = "O10",
        AddHours = "O11",
        AddDays = "O12",
    }
}

string_enum! {
    /// `FieldOperandKind`.
    pub enum FieldOperandKind {
        None = "none",
        Typed = "typed",
        Text = "text",
        Number = "number",
        Expression = "expression",
    }
}

/// `FieldOperationDefinition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldOperationDefinition {
    pub api_name: &'static str,
    pub allowed_types: &'static [CustomFieldType],
    pub operand_kind: FieldOperandKind,
    pub requires_operand: bool,
}

const TEXT_TYPES: &[CustomFieldType] = &[CustomFieldType::ShortText, CustomFieldType::LongText];
const TEMPORAL_TYPES: &[CustomFieldType] = &[CustomFieldType::Date, CustomFieldType::Datetime];
const NUMBER: &[CustomFieldType] = &[CustomFieldType::Number];

const fn def(
    api_name: &'static str,
    allowed_types: &'static [CustomFieldType],
    operand_kind: FieldOperandKind,
    requires_operand: bool,
) -> FieldOperationDefinition {
    FieldOperationDefinition {
        api_name,
        allowed_types,
        operand_kind,
        requires_operand,
    }
}

/// `FIELD_OPERATION_DEFINITIONS[operation]`.
pub fn field_operation_definition(operation: FieldOperationType) -> FieldOperationDefinition {
    use FieldOperandKind as K;
    use FieldOperationType as O;
    let all = CustomFieldType::ALL;
    match operation {
        O::Set => def("set", all, K::Typed, false),
        O::Append => def("append", TEXT_TYPES, K::Text, false),
        O::Prepend => def("prepend", TEXT_TYPES, K::Text, false),
        O::Increase => def("increase", NUMBER, K::Number, true),
        O::Decrease => def("decrease", NUMBER, K::Number, true),
        O::Clear => def("clear", all, K::None, false),
        O::Formula => def("formula", NUMBER, K::Expression, true),
        O::SetNow => def("setNow", TEMPORAL_TYPES, K::None, false),
        O::DateExpression => def("dateExpression", TEMPORAL_TYPES, K::Expression, true),
        O::AddMinutes => def("addMinutes", TEMPORAL_TYPES, K::Number, true),
        O::AddHours => def("addHours", TEMPORAL_TYPES, K::Number, true),
        O::AddDays => def("addDays", TEMPORAL_TYPES, K::Number, true),
    }
}

/// `fieldOperationCodes`.
pub fn field_operation_codes() -> &'static [FieldOperationType] {
    FieldOperationType::ALL
}

/// `isFieldOperationAllowed`.
pub fn is_field_operation_allowed(operation: FieldOperationType, kind: CustomFieldType) -> bool {
    field_operation_definition(operation)
        .allowed_types
        .contains(&kind)
}

/// `fieldOperationsForType`.
pub fn field_operations_for_type(kind: CustomFieldType) -> Vec<FieldOperationType> {
    FieldOperationType::ALL
        .iter()
        .copied()
        .filter(|op| is_field_operation_allowed(*op, kind))
        .collect()
}

/// `fieldOperationByApiName`.
pub fn field_operation_by_api_name(api_name: &str) -> Option<FieldOperationType> {
    FieldOperationType::ALL
        .iter()
        .copied()
        .find(|op| field_operation_definition(*op).api_name == api_name)
}

/// `fieldOperationApiNames`.
pub fn field_operation_api_names() -> Vec<&'static str> {
    FieldOperationType::ALL
        .iter()
        .map(|op| field_operation_definition(*op).api_name)
        .collect()
}
