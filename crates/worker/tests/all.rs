//! All integration tests of this crate in one binary (see the guard test).
// Each test file keeps its own copy of its helper modules, as before.
#![allow(clippy::duplicate_mod)]

#[path = "analytics_events.rs"]
mod analytics_events;
#[path = "appointment_google_event.rs"]
mod appointment_google_event;
#[path = "inbound_webhook_worker.rs"]
mod inbound_webhook_worker;
#[path = "meta_catalog_reconciler.rs"]
mod meta_catalog_reconciler;
#[path = "schedule_broadcast.rs"]
mod schedule_broadcast;
#[path = "schedule_recurring.rs"]
mod schedule_recurring;
#[path = "sequence_worker.rs"]
mod sequence_worker;
#[path = "smart_delay.rs"]
mod smart_delay;

// ponytail: one test binary per crate (was one per file: ~200 MB link each,
// the Dell gate bottleneck). Every tests/*.rs file must be listed above.
#[test]
fn every_test_file_is_listed_in_all_rs() {
    let listed = include_str!("all.rs");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let mut missing: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".rs") && n != "all.rs")
        .filter(|n| !listed.contains(&format!("#[path = \"{n}\"]")))
        .collect();
    missing.sort();
    assert!(missing.is_empty(), "add these to tests/all.rs: {missing:?}");
}
