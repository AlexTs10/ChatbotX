//! Port of `packages/utils/__tests__/coordinates.test.ts`.

use chatbotx_core::utils::coordinates::{CoordinatePair, parse_coordinate_pair};

/// Port of: packages/utils/__tests__/coordinates.test.ts:5 "parseCoordinatePair > round-trips a stored pin exactly"
#[test]
fn parsecoordinatepair_round_trips_a_stored_pin_exactly() {
    assert_eq!(
        parse_coordinate_pair("16.0078846304474,108.26602981877288"),
        Some(CoordinatePair {
            latitude: 16.007_884_630_447_4,
            longitude: 108.266_029_818_772_88,
        })
    );
}

/// Port of: packages/utils/__tests__/coordinates.test.ts:12 "parseCoordinatePair > parses negatives and tolerates whitespace"
#[test]
fn parsecoordinatepair_parses_negatives_and_tolerates_whitespace() {
    assert_eq!(
        parse_coordinate_pair(" -33.8688 ,151.2093 "),
        Some(CoordinatePair {
            latitude: -33.8688,
            longitude: 151.2093,
        })
    );
}

/// Port of: packages/utils/__tests__/coordinates.test.ts:19 "parseCoordinatePair > rejects non-coordinates, out-of-range values and single numbers"
#[test]
fn parsecoordinatepair_rejects_non_coordinates_out_of_range_values_and_single_numbers() {
    assert_eq!(parse_coordinate_pair("abc"), None);
    assert_eq!(parse_coordinate_pair("91, 10"), None);
    assert_eq!(parse_coordinate_pair("10, 181"), None);
    assert_eq!(parse_coordinate_pair("16.0078"), None);
}
