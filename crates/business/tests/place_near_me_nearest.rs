//! Port of `packages/business/src/place-near-me/__tests__/nearest.test.ts`.

use chatbotx_business::place_near_me::{
    Coordinates, PlaceCoordinates, haversine_km, pick_nearest_place,
};

fn place(id: i64, latitude: f64, longitude: f64) -> PlaceCoordinates {
    PlaceCoordinates {
        id,
        name: format!("Place {id}"),
        latitude,
        longitude,
    }
}

fn point(latitude: f64, longitude: f64) -> Coordinates {
    Coordinates {
        latitude,
        longitude,
    }
}

#[test]
fn haversine_km_hanoi_to_hcmc_is_about_1137_km() {
    let km = haversine_km(point(21.0285, 105.8542), point(10.8231, 106.6297));
    // `toBeCloseTo(1137, -1)`: |km - 1137| < 10^1 / 2.
    assert!((km - 1137.0).abs() < 5.0, "{km}");
}

#[test]
fn haversine_km_same_point_is_0() {
    let p = point(16.007_884_630_447_4, 108.266_029_818_772_88);
    assert_eq!(haversine_km(p, p), 0.0);
}

#[test]
fn pick_nearest_place_picks_the_closest() {
    let nearest = pick_nearest_place(
        point(10.8, 106.7),
        &[
            place(1, 21.0285, 105.8542),
            place(2, 10.8231, 106.6297),
            place(3, 16.0544, 108.2022),
        ],
    )
    .expect("a place");
    assert_eq!(nearest.id, 2);
    assert!(nearest.distance_km > 0.0);
}

#[test]
fn pick_nearest_place_breaks_ties_by_lower_id() {
    let nearest = pick_nearest_place(point(0.0, 0.0), &[place(10, 1.0, 0.0), place(9, -1.0, 0.0)]);
    assert_eq!(nearest.map(|p| p.id), Some(9));
}

#[test]
fn pick_nearest_place_returns_null_for_empty() {
    assert_eq!(pick_nearest_place(point(0.0, 0.0), &[]), None);
}

#[test]
fn pick_nearest_place_antimeridian() {
    let nearest = pick_nearest_place(
        point(0.0, 179.9),
        &[place(1, 0.0, 170.0), place(2, 0.0, -179.9)],
    );
    assert_eq!(nearest.map(|p| p.id), Some(2));
}
