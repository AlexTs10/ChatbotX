//! `place-near-me/nearest.ts`.

const EARTH_RADIUS_KM: f64 = 6371.0;

/// A point in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
}

/// A place candidate (`Omit<NearestPlace, "distanceKm">`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceCoordinates {
    pub id: i64,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// `NearestPlace`.
#[derive(Debug, Clone, PartialEq)]
pub struct NearestPlace {
    pub id: i64,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub distance_km: f64,
}

fn to_radians(degrees: f64) -> f64 {
    (degrees * std::f64::consts::PI) / 180.0
}

/// `haversineKm`: great-circle distance in kilometres.
pub fn haversine_km(a: Coordinates, b: Coordinates) -> f64 {
    todo!("P3.6-D1")
}

/// `pickNearestPlace`: the closest place to `origin`; equal distances go to the lower id.
pub fn pick_nearest_place(
    origin: Coordinates,
    places: &[PlaceCoordinates],
) -> Option<NearestPlace> {
    todo!("P3.6-D1")
}
