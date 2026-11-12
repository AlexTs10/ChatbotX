//! Port of `packages/utils/__tests__/request.test.ts`. A `Request` becomes
//! `RequestParts` (URL + `http::HeaderMap`); `process.env.FORCE_PUBLIC_HTTPS`
//! becomes the `force_https` argument, read with `force_public_https(env)`.

use chatbotx_core::utils::request::{
    RequestParts, force_public_https, get_public_host_from_request,
    get_public_protocol_from_request, get_public_url_from_request,
    get_raw_public_host_from_request,
};
use http::{HeaderMap, HeaderName, HeaderValue};

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

/// Port of: packages/utils/__tests__/request.test.ts:20 "getPublicProtocolFromRequest > forces HTTPS before inspecting forwarded protocol headers"
#[test]
fn getpublicprotocolfromrequest_forces_https_before_inspecting_forwarded_protocol_headers() {
    let force = force_public_https(&[("FORCE_PUBLIC_HTTPS", "true")]);
    let h = headers(&[
        ("forwarded", "for=192.0.2.1;proto=http"),
        ("x-forwarded-proto", "http"),
    ]);
    let request = RequestParts {
        url: "http://internal.test/path",
        headers: &h,
    };

    assert_eq!(get_public_protocol_from_request(&request, force), "https");
}

/// Port of: packages/utils/__tests__/request.test.ts:32 "getPublicProtocolFromRequest > uses the first valid Forwarded protocol when forcing is disabled"
#[test]
fn getpublicprotocolfromrequest_uses_the_first_valid_forwarded_protocol_when_forcing_is_disabled() {
    let force = force_public_https(&[("FORCE_PUBLIC_HTTPS", "false")]);
    let h = headers(&[
        (
            "forwarded",
            "for=192.0.2.1;proto=http, for=192.0.2.2;proto=https",
        ),
        ("x-forwarded-proto", "https"),
    ]);
    let request = RequestParts {
        url: "https://internal.test/path",
        headers: &h,
    };

    assert_eq!(get_public_protocol_from_request(&request, force), "http");
}

/// Port of: packages/utils/__tests__/request.test.ts:44 "getPublicProtocolFromRequest > falls back to X-Forwarded-Proto and then the request URL"
#[test]
fn getpublicprotocolfromrequest_falls_back_to_x_forwarded_proto_and_then_the_request_url() {
    let force = force_public_https(&chatbotx_core::env::EmptyEnv);
    let h = headers(&[("x-forwarded-proto", "https")]);
    assert_eq!(
        get_public_protocol_from_request(
            &RequestParts {
                url: "http://internal.test",
                headers: &h
            },
            force
        ),
        "https"
    );
    let none = HeaderMap::new();
    assert_eq!(
        get_public_protocol_from_request(
            &RequestParts {
                url: "http://internal.test",
                headers: &none
            },
            force
        ),
        "http"
    );
}

/// Port of: packages/utils/__tests__/request.test.ts:61 "getRawPublicHostFromRequest > prefers the Forwarded header's host over everything else"
#[test]
fn getrawpublichostfromrequest_prefers_the_forwarded_header_s_host_over_everything_else() {
    let h = headers(&[
        (
            "forwarded",
            "for=192.0.2.1;host=forwarded.example.com;proto=https",
        ),
        ("x-forwarded-host", "xfh.example.com"),
        ("host", "plain.example.com"),
    ]);
    let request = RequestParts {
        url: "http://internal.test",
        headers: &h,
    };

    assert_eq!(
        get_raw_public_host_from_request(&request).as_deref(),
        Some("forwarded.example.com")
    );
}

/// Port of: packages/utils/__tests__/request.test.ts:73 "getRawPublicHostFromRequest > falls back to X-Forwarded-Host, taking only the first of a comma-separated list"
#[test]
fn getrawpublichostfromrequest_falls_back_to_x_forwarded_host_taking_only_the_first_of_a_comma_separated_list()
 {
    let h = headers(&[
        ("x-forwarded-host", "first.example.com, second.example.com"),
        ("host", "plain.example.com"),
    ]);
    let request = RequestParts {
        url: "http://internal.test",
        headers: &h,
    };

    assert_eq!(
        get_raw_public_host_from_request(&request).as_deref(),
        Some("first.example.com")
    );
}

/// Port of: packages/utils/__tests__/request.test.ts:84 "getRawPublicHostFromRequest > falls back to the plain Host header when nothing is forwarded"
#[test]
fn getrawpublichostfromrequest_falls_back_to_the_plain_host_header_when_nothing_is_forwarded() {
    let h = headers(&[("host", "plain.example.com:8443")]);
    let request = RequestParts {
        url: "http://internal.test",
        headers: &h,
    };

    assert_eq!(
        get_raw_public_host_from_request(&request).as_deref(),
        Some("plain.example.com:8443")
    );
}

/// Port of: packages/utils/__tests__/request.test.ts:92 "getRawPublicHostFromRequest > returns null when no host can be resolved at all — no invented default"
#[test]
fn getrawpublichostfromrequest_returns_null_when_no_host_can_be_resolved_at_all_no_invented_default()
 {
    let h = HeaderMap::new();
    let request = RequestParts {
        url: "http://internal.test",
        headers: &h,
    };

    assert_eq!(get_raw_public_host_from_request(&request), None);
}

/// Port of: packages/utils/__tests__/request.test.ts:98 "getRawPublicHostFromRequest > lowercases and trims whatever it resolves"
#[test]
fn getrawpublichostfromrequest_lowercases_and_trims_whatever_it_resolves() {
    let h = headers(&[("host", "  Plain.Example.COM  ")]);
    let request = RequestParts {
        url: "http://internal.test",
        headers: &h,
    };

    assert_eq!(
        get_raw_public_host_from_request(&request).as_deref(),
        Some("plain.example.com")
    );
}

/// Port of: packages/utils/__tests__/request.test.ts:108 "getPublicHostFromRequest > keeps its existing localhost:3123 default when nothing resolves"
#[test]
fn getpublichostfromrequest_keeps_its_existing_localhost_3123_default_when_nothing_resolves() {
    let h = HeaderMap::new();
    let request = RequestParts {
        url: "http://internal.test",
        headers: &h,
    };

    assert_eq!(get_public_host_from_request(&request), "localhost:3123");
}

/// Port of: packages/utils/__tests__/request.test.ts:114 "getPublicHostFromRequest > still resolves a real host the same way it always did"
#[test]
fn getpublichostfromrequest_still_resolves_a_real_host_the_same_way_it_always_did() {
    let h = headers(&[("host", "plain.example.com")]);
    let request = RequestParts {
        url: "http://internal.test",
        headers: &h,
    };

    assert_eq!(get_public_host_from_request(&request), "plain.example.com");
}

/// Port of: packages/utils/__tests__/request.test.ts:124 "getPublicUrlFromRequest > applies forced HTTPS to the returned public URL"
#[test]
fn getpublicurlfromrequest_applies_forced_https_to_the_returned_public_url() {
    let force = force_public_https(&[("FORCE_PUBLIC_HTTPS", "true")]);
    let h = HeaderMap::new();
    let url = get_public_url_from_request(
        &RequestParts {
            url: "http://internal.test/callback",
            headers: &h,
        },
        force,
    )
    .unwrap();

    assert_eq!(url.scheme(), "https");
}
