//! Port of `packages/flow-config/__tests__/whatsapp-location.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn whatsapplocationstepschema_accepts_a_step_built_by_the_default_factory() {
    let step =
        fc::whatsapp_location_step_default_fn(&json!({ "latitude": 15.56, "longitude": 108.48 }));

    assert!(
        fc::whatsapp_location_step_schema()
            .safe_parse(&step)
            .success
    );
}

#[test]
fn whatsapplocationstepschema_rejects_the_unconfigured_default_of_0_0() {
    let step = fc::whatsapp_location_step_default_fn(&json!({}));

    assert!(
        !fc::whatsapp_location_step_schema()
            .safe_parse(&step)
            .success
    );
}

#[test]
fn whatsapplocationstepschema_rejects_out_of_range_coordinates_o() {
    for coords in [
        json!({ "latitude": 91, "longitude": 0 }),
        json!({ "latitude": -91, "longitude": 0 }),
        json!({ "latitude": 0, "longitude": 181 }),
        json!({ "latitude": 0, "longitude": -181 }),
    ] {
        let step = fc::whatsapp_location_step_default_fn(&coords);

        assert!(
            !fc::whatsapp_location_step_schema()
                .safe_parse(&step)
                .success,
            "{coords}"
        );
    }
}

fn valid_form() -> Value {
    json!({
        "name": "Mường Thanh",
        "address": "Tam Kỳ",
        "latitude": "15.561419980411108",
        "longitude": "108.4867823491902",
    })
}

#[test]
fn whatsapplocationformschema_accepts_the_values_from_the_design_mock_up() {
    assert!(
        fc::whatsapp_location_form_schema()
            .safe_parse(&valid_form())
            .success
    );
}

#[test]
fn whatsapplocationformschema_allows_blank_name_and_address() {
    let result = fc::whatsapp_location_form_schema()
        .safe_parse(&spread(&valid_form(), json!({ "name": "", "address": "" })));

    assert!(result.success);
}

#[test]
fn whatsapplocationformschema_rejects_latitude_j() {
    for latitude in ["", "abc", "91", "NaN"] {
        assert!(
            !fc::whatsapp_location_form_schema()
                .safe_parse(&spread(&valid_form(), json!({ "latitude": latitude })))
                .success,
            "{latitude}"
        );
    }
}

#[test]
fn whatsapplocationformschema_accepts_a_decimal_comma_and_rejects_exponent_notation() {
    assert!(
        fc::whatsapp_location_form_schema()
            .safe_parse(&spread(&valid_form(), json!({ "latitude": "10,5" })))
            .success
    );
    assert!(
        !fc::whatsapp_location_form_schema()
            .safe_parse(&spread(&valid_form(), json!({ "latitude": "1e1" })))
            .success
    );
}

#[test]
fn whatsapplocationformschema_rejects_a_longitude_beyond_180() {
    assert!(
        !fc::whatsapp_location_form_schema()
            .safe_parse(&spread(&valid_form(), json!({ "longitude": "200" })))
            .success
    );
}

#[test]
fn iswhatsapplocationunset_treats_only_the_0_0_default_as_unset() {
    assert!(fc::is_whatsapp_location_unset(
        &json!({ "latitude": 0, "longitude": 0 })
    ));
    assert!(!fc::is_whatsapp_location_unset(
        &json!({ "latitude": 0, "longitude": 10 })
    ));
    assert!(!fc::is_whatsapp_location_unset(
        &json!({ "latitude": 10, "longitude": 0 })
    ));
}

#[test]
fn iswhatsapplocationunset_the_dialog_rejects_0_0_but_accepts_a_point_on_the_equator() {
    let base = json!({ "name": "", "address": "" });

    assert!(
        !fc::whatsapp_location_form_schema()
            .safe_parse(&spread(&base, json!({ "latitude": "0", "longitude": "0" })))
            .success
    );
    assert!(
        fc::whatsapp_location_form_schema()
            .safe_parse(&spread(
                &base,
                json!({ "latitude": "0", "longitude": "32.5" })
            ))
            .success
    );
}

fn messages_for(values: Value) -> Vec<String> {
    fc::whatsapp_location_form_schema()
        .safe_parse(&values)
        .messages()
}

fn dialog_valid() -> Value {
    json!({ "name": "", "address": "", "latitude": "10", "longitude": "20" })
}

#[test]
fn whatsapp_location_dialog_errors_and_coordinate_text_reports_a_code_per_invalid_field() {
    let errors = fc::whatsapp_location_form_errors();
    assert_eq!(
        messages_for(spread(&dialog_valid(), json!({ "latitude": "91" }))),
        vec![errors["invalidLatitude"].as_str().unwrap()]
    );
    assert_eq!(
        messages_for(spread(&dialog_valid(), json!({ "longitude": "" }))),
        vec![errors["invalidLongitude"].as_str().unwrap()]
    );
    assert_eq!(
        messages_for(spread(&dialog_valid(), json!({ "name": rep("x", 1001) }))),
        vec![errors["tooLong"].as_str().unwrap()]
    );
    assert_eq!(
        messages_for(spread(
            &dialog_valid(),
            json!({ "latitude": "0", "longitude": "0" })
        )),
        vec![errors["locationRequired"].as_str().unwrap()]
    );
}

#[test]
fn whatsapp_location_dialog_errors_and_coordinate_text_round_trips_d_through_the_dialog_text() {
    for value in [
        1e-7,
        -1.5e-7,
        1e-21,
        5e-324,
        15.561_419_980_411_108,
        -122.161_535_820_493_94,
        90.0,
        -0.5,
        0.0,
    ] {
        let text = fc::format_whatsapp_location_coordinate(value);

        assert!(!text.to_lowercase().contains('e'), "{text}");
        assert_eq!(
            fc::parse_whatsapp_location_coordinate(&text),
            Some(value),
            "{text}"
        );
    }
}
