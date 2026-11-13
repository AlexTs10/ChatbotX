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
    let d_lat = to_radians(b.latitude - a.latitude);
    let d_lng = to_radians(b.longitude - a.longitude);
    let h = (d_lat / 2.0).sin().powi(2)
        + to_radians(a.latitude).cos() * to_radians(b.latitude).cos() * (d_lng / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().min(1.0).asin()
}

/// `pickNearestPlace`: the closest place to `origin`; equal distances go to the lower id.
pub fn pick_nearest_place(
    origin: Coordinates,
    places: &[PlaceCoordinates],
) -> Option<NearestPlace> {
    let mut nearest: Option<NearestPlace> = None;
    for place in places {
        let distance_km = haversine_km(
            origin,
            Coordinates {
                latitude: place.latitude,
                longitude: place.longitude,
            },
        );
        let better = match &nearest {
            None => true,
            Some(best) => {
                distance_km < best.distance_km
                    || (distance_km == best.distance_km && place.id < best.id)
            }
        };
        if better {
            nearest = Some(NearestPlace {
                id: place.id,
                name: place.name.clone(),
                latitude: place.latitude,
                longitude: place.longitude,
                distance_km,
            });
        }
    }
    nearest
}
