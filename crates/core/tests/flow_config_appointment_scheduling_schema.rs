//! Port of `packages/flow-config/__tests__/appointment-scheduling-schema.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn base_step() -> Value {
    json!({
        "id": "1",
        "stepType": "appointmentScheduling",
        "mode": "checkAvailability",
        "calendarId": "1",
        "outputCustomFieldId": "output-field",
        "states": [
            { "id": "1", "stateType": "success" },
            { "id": "2", "stateType": "error" },
        ],
    })
}

#[test]
fn appointment_scheduling_schemas_checkavailability_requires_outputcustomfieldid_and_keeps_ai_as_a_one_click_toggle()
 {
    let result = fc::appointment_scheduling_step_schema().safe_parse(&base_step());

    assert!(result.success);
    let data = result.data.unwrap();
    if data["mode"] != "checkAvailability" {
        return;
    }

    assert_eq!(data["resultUsedByAI"], false);
    for key in ["provider", "model", "temperature", "maxOutputTokens"] {
        assert!(data.get(key).is_none(), "{key}");
    }
}

#[test]
fn appointment_scheduling_schemas_checkavailability_rejects_legacy_start_end_only_config() {
    let step = spread(
        &without(&base_step(), &["outputCustomFieldId"]),
        json!({ "startDateFieldId": "start-field", "endDateFieldId": "end-field" }),
    );
    assert!(
        !fc::appointment_scheduling_step_schema()
            .safe_parse(&step)
            .success
    );
}

#[test]
fn appointment_scheduling_schemas_bookfromcustomfield_requires_datetimefieldid() {
    let step = json!({
        "id": "1",
        "stepType": "appointmentScheduling",
        "mode": "bookFromCustomField",
        "calendarId": "1",
        "states": [
            { "id": "1", "stateType": "success" },
            { "id": "2", "stateType": "error" },
        ],
    });

    assert!(
        !fc::appointment_scheduling_step_schema()
            .safe_parse(&step)
            .success
    );
    assert!(
        fc::appointment_scheduling_step_schema()
            .safe_parse(&spread(&step, json!({ "dateTimeFieldId": "date-field" })))
            .success
    );
}

#[test]
fn appointment_scheduling_schemas_checkavailabilityfromcustomfield_requires_outputcustomfieldid_regardless_of_resultusedbyai()
 {
    let step = spread(
        &without(&base_step(), &["outputCustomFieldId"]),
        json!({
            "mode": "checkAvailabilityFromCustomField",
            "startDateFieldId": "start-field",
            "endDateFieldId": "end-field",
        }),
    );

    assert!(
        !fc::appointment_scheduling_step_schema()
            .safe_parse(&spread(&step, json!({ "resultUsedByAI": false })))
            .success
    );
    assert!(
        fc::appointment_scheduling_step_schema()
            .safe_parse(&spread(
                &step,
                json!({ "resultUsedByAI": false, "outputCustomFieldId": "output-field" })
            ))
            .success
    );
}

#[test]
fn appointment_scheduling_schemas_parses_appointment_availability_range_metadata() {
    assert!(
        fc::metadata_schema()
            .safe_parse(&json!({
                "type": "appointmentAvailabilityRangeSelection",
                "stepId": "step-1",
                "contactInboxId": "contact-inbox-1",
                "startDate": "2026-08-10T09:00:00.000",
                "endDate": "2026-08-12T17:00:00.000",
            }))
            .success
    );

    assert!(
        fc::metadata_schema()
            .safe_parse(
                &json!({ "type": "appointmentAvailabilityRangeSkipped", "stepId": "step-1" })
            )
            .success
    );
}
