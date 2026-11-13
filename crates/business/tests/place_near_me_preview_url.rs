//! Port of `packages/business/src/place-near-me/__tests__/preview-url.test.ts`.

use chatbotx_business::place_near_me::build_google_maps_preview_url;

#[test]
fn build_google_maps_preview_url_keeps_full_coordinate_precision() {
    assert_eq!(
        build_google_maps_preview_url(16.007_884_630_447_4, 108.266_029_818_772_88),
        "https://www.google.com/maps/search/?api=1&query=16.0078846304474%2C108.26602981877288"
    );
}

#[test]
fn build_google_maps_preview_url_keeps_negative_coordinates() {
    assert_eq!(
        build_google_maps_preview_url(-33.8688, 151.2093),
        "https://www.google.com/maps/search/?api=1&query=-33.8688%2C151.2093"
    );
}
