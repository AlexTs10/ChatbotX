//! Port of `packages/event-bus/__tests__/configured-buses.test.ts`
//! (spec 5.3 "configured-buses"): the four configured `EventBusConfig`s.

use chatbotx_jobs::events::config::{
    BusKind, bus_for_event_type, dashboard_bus_config, error_log_bus_config, flow_bus_config,
    flow_event_types, message_bus_config, message_event_types,
};

/// Port of: packages/event-bus/__tests__/configured-buses.test.ts:27 "configured event buses > configures stream keys, groups, retention, and schemas"
#[test]
fn configures_stream_keys_groups_retention_and_schemas() {
    let message = message_bus_config();
    assert_eq!(message.consumer_group, "message-events-group");
    assert_eq!(message.max_len, 100_000);
    assert_eq!(message.stream_key, "events:message");

    let flow = flow_bus_config();
    assert_eq!(flow.consumer_group, "flow-events-group");
    assert_eq!(flow.max_len, 100_000);
    assert_eq!(flow.stream_key, "flow:events");

    let dashboard = dashboard_bus_config();
    assert_eq!(dashboard.consumer_group, "analytics-dashboard-events-group");
    assert_eq!(dashboard.max_len, 500_000);
    assert_eq!(dashboard.stream_key, "events:analytics-dashboard");

    // Sized from a byte budget, not copied: an error-log `detail` can be 8KB.
    let error_log = error_log_bus_config();
    assert_eq!(error_log.consumer_group, "error-log-events-group");
    assert_eq!(error_log.max_len, 50_000);
    assert_eq!(error_log.stream_key, "events:error-log");
}

/// Port of: packages/event-bus/__tests__/configured-buses.test.ts:52 "configured event buses > enables selective retry only for the idempotent dashboard and error-log streams"
#[test]
fn enables_selective_retry_only_for_the_idempotent_dashboard() {
    let dashboard = dashboard_bus_config();
    assert_eq!(dashboard.dead_letter_max_len, 100_000);
    assert_eq!(
        dashboard.dead_letter_stream_key,
        "events:analytics-dashboard:dead"
    );
    assert_eq!(dashboard.enable_selective_retry, Some(true));
    assert_eq!(dashboard.max_deliveries, 5);

    let error_log = error_log_bus_config();
    assert_eq!(error_log.dead_letter_max_len, 10_000);
    assert_eq!(error_log.dead_letter_stream_key, "events:error-log:dead");
    assert_eq!(error_log.enable_selective_retry, Some(true));
    assert_eq!(error_log.max_deliveries, 5);

    assert_eq!(message_bus_config().enable_selective_retry, None);
    assert_eq!(flow_bus_config().enable_selective_retry, None);
}

/// Port of: packages/event-bus/__tests__/configured-buses.test.ts:69 "configured event buses > maps every message and flow event type to its bus"
#[test]
fn maps_every_message_and_flow_event_type_to_its_bus() {
    // Guard against a vacuous pass: the flow-config enums are not empty.
    assert!(!message_event_types().is_empty());
    assert!(!flow_event_types().is_empty());
    for event_type in message_event_types() {
        assert_eq!(
            bus_for_event_type(event_type),
            Some(BusKind::Message),
            "{event_type}"
        );
    }
    for event_type in flow_event_types() {
        assert_eq!(
            bus_for_event_type(event_type),
            Some(BusKind::Flow),
            "{event_type}"
        );
    }
}
