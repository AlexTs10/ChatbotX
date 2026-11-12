//! Port of `apps/realtime/__tests__/delivery.test.ts` (P2.8 spec 4.6,
//! R100-R120). The TS fake socket (`sent[]`, `closed`, `end`, `send`
//! returning 1/2 or throwing) is `support::FakeSink`; the uWS topic
//! publishes become frames on registered sockets (`Delivery::dispatch` runs
//! the same per-socket rules as the connection tasks). A `member-revoke`
//! record is `Delivery::revoke_member` (revokes never enter the buffer).

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use chatbotx_rt::TestClock;
use chatbotx_rt::delivery::{Counters, Delivery, MemberSocketData};
use chatbotx_rt::hub::{Entry, Hub, HubConfig};
use chatbotx_rt::protocol::{Envelope, RevokeReason, Seq};
use chatbotx_rt::token::ChatScope;
use serde_json::{Value, json};
use support::{RecordingLog, SendMode, guest_socket, member_data, member_socket, strings};
use tracing::Level;

fn create_delivery() -> (Arc<Counters>, Delivery) {
    let counters = Arc::new(Counters::default());
    (Arc::clone(&counters), Delivery::new(counters))
}

fn envelopes(events: Value) -> Vec<Envelope> {
    serde_json::from_value(events).unwrap()
}

fn envelope(event: Value) -> Envelope {
    serde_json::from_value(event).unwrap()
}

/// `workspaceEntry(events)`: id `1-0`, `workspace-1`.
fn workspace_entry(events: Value) -> Arc<Entry> {
    Entry::workspace_events(Seq::new(1, 0), "workspace-1", envelopes(events))
}

fn assigned(team_ids: &[&str]) -> MemberSocketData {
    MemberSocketData {
        chat_scope: ChatScope::Assigned,
        team_ids: strings(team_ids),
        ..member_data()
    }
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:113 "realtime delivery > publishes every event from a record in one frame"
#[test]
fn realtime_delivery_publishes_every_event_from_a_record_in_one_frame() {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(member_data(), SendMode::Ok);
    delivery.add_workspace_socket(&socket);

    delivery.dispatch(&workspace_entry(json!([
        { "data": { "id": "message-1" }, "eventType": "messageCreated" },
        { "data": { "id": "message-2" }, "eventType": "messageCreated" },
    ])));

    assert_eq!(
        sink.sent(),
        vec![
            json!({
                "batch": [
                    { "data": { "id": "message-1" }, "eventType": "messageCreated" },
                    { "data": { "id": "message-2" }, "eventType": "messageCreated" },
                ],
                "seq": "1-0",
            })
            .to_string()
        ]
    );
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:137 "realtime delivery > records, logs, and resyncs a socket when sending fails"
#[test]
fn realtime_delivery_records_logs_and_resyncs_a_socket_when_sending_fails() {
    let log = RecordingLog::install();
    let (counters, delivery) = create_delivery();
    let (socket, sink) = member_socket(member_data(), SendMode::Fail);

    assert!(!delivery.replay_workspace_socket(
        &socket,
        &[workspace_entry(
            json!([{ "data": {}, "eventType": "messageCreated" }])
        )],
    ));

    assert_eq!(counters.send_failures.load(Ordering::SeqCst), 1);
    assert_eq!(sink.closed(), Some((4002, "send-failed".to_owned())));
    let warn = log.find("Realtime send failed").expect("warn log");
    assert_eq!(warn.level, Level::WARN);
    assert_eq!(warn.field("err"), Some("socket write failed"));
    assert_eq!(warn.field("workspace_id"), Some("workspace-1"));
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:161 "realtime delivery > delivers one filtered copy to an assigned user who is also on the team"
#[test]
fn realtime_delivery_delivers_one_filtered_copy_to_an_assigned_user_who_is_also_on_the_team() {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(assigned(&["team-1"]), SendMode::Ok);
    delivery.add_workspace_socket(&socket);

    delivery.dispatch(&workspace_entry(json!([{
        "data": {},
        "eventType": "messageCreated",
        "route": { "assignedTeamIds": ["team-1"], "assignedUserIds": ["user-1"] },
    }])));

    assert_eq!(sink.sent().len(), 1);
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:179 "realtime delivery > delivers route-less workspace events to assigned members"
#[test]
fn realtime_delivery_delivers_route_less_workspace_events_to_assigned_members() {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(assigned(&[]), SendMode::Ok);
    delivery.add_workspace_socket(&socket);

    delivery.dispatch(&workspace_entry(
        json!([{ "data": {}, "eventType": "messageCreated" }]),
    ));

    assert_eq!(sink.sent().len(), 1);
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:192 "realtime delivery > delivers directed member events to a full-access socket"
#[test]
fn realtime_delivery_delivers_directed_member_events_to_a_full_access_socket() {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(member_data(), SendMode::Ok);
    delivery.add_workspace_socket(&socket);

    delivery.dispatch(&Entry::member_send(
        Seq::new(2, 0),
        "workspace-1",
        "user-1",
        envelope(json!({ "data": {}, "eventType": "whatsappCallTransportIncoming" })),
    ));

    assert_eq!(sink.sent().len(), 1);
}

fn closes_a_targeted_member_socket_with(reason: RevokeReason, expected: (u16, &str)) {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(member_data(), SendMode::Ok);
    delivery.add_workspace_socket(&socket);

    delivery.revoke_member("workspace-1", "user-1", reason, 1);

    assert_eq!(sink.closed(), Some((expected.0, expected.1.to_owned())));
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:210 "realtime delivery > closes a targeted member socket with the deleted code"
#[test]
fn realtime_delivery_closes_a_targeted_member_socket_with_the_deleted_code() {
    closes_a_targeted_member_socket_with(RevokeReason::Deleted, (4001, "revoked"));
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:210 "realtime delivery > closes a targeted member socket with the reauth code"
#[test]
fn realtime_delivery_closes_a_targeted_member_socket_with_the_reauth_code() {
    closes_a_targeted_member_socket_with(RevokeReason::Reauth, (4004, "reauth"));
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:232 "realtime delivery > ignores a stale revoke and closes a socket minted before the revoke"
#[test]
fn realtime_delivery_ignores_a_stale_revoke_and_closes_a_socket_minted_before_the_revoke() {
    let (_, delivery) = create_delivery();
    let (fresh_socket, fresh_sink) = member_socket(
        MemberSocketData {
            iat_ms: 200,
            ..member_data()
        },
        SendMode::Ok,
    );
    let (stale_socket, stale_sink) = member_socket(
        MemberSocketData {
            iat_ms: 100,
            ..member_data()
        },
        SendMode::Ok,
    );
    delivery.add_workspace_socket(&fresh_socket);
    delivery.add_workspace_socket(&stale_socket);

    delivery.revoke_member("workspace-1", "user-1", RevokeReason::Reauth, 150);

    assert_eq!(fresh_sink.closed(), None);
    assert_eq!(stale_sink.closed(), Some((4004, "reauth".to_owned())));
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:254 "realtime delivery > replay never closes on a member-revoke entry — revocation is checked once at connect, not re-derived from stream-entry clocks"
#[test]
fn realtime_delivery_replay_never_closes_on_a_member_revoke_entry_revocation_is_checked_once_at_connect_not_re_derived_from_stream_entry_clocks()
 {
    // Adapted: a revoke is a direct action and never enters the buffer, so
    // a replay window around a revoke holds no revoke to re-derive.
    let clock = Arc::new(TestClock::new(11_000));
    let hub = Hub::new(
        clock,
        HubConfig::default(),
        Delivery::new(Arc::new(Counters::default())),
    );
    hub.append_workspace_events(
        "workspace-1",
        envelopes(json!([{ "data": {}, "eventType": "typing" }])),
    );
    hub.delivery()
        .revoke_member("workspace-1", "user-1", RevokeReason::Deleted, 1);
    hub.append_workspace_events(
        "workspace-1",
        envelopes(json!([{ "data": {}, "eventType": "typing" }])),
    );
    let entries = hub.buffered("workspace-1");
    assert_eq!(entries.len(), 2);
    let (socket, sink) = member_socket(member_data(), SendMode::Ok);

    let result = hub.delivery().replay_workspace_socket(&socket, &entries);

    assert!(result);
    assert_eq!(sink.closed(), None);
}

fn publishes_guest_events(workspace_ids: &[&str], expected_frames: (usize, usize)) {
    let (_, delivery) = create_delivery();
    let (socket_1, sink_1) = guest_socket("workspace-1", "12345", SendMode::Ok);
    let (socket_2, sink_2) = guest_socket("workspace-2", "12345", SendMode::Ok);
    delivery.add_guest_socket(&socket_1);
    delivery.add_guest_socket(&socket_2);

    for (index, workspace_id) in workspace_ids.iter().enumerate() {
        delivery.dispatch(&Entry::guest_event(
            Seq::new(index as u64 + 4, 0),
            workspace_id,
            "12345",
            envelope(json!({ "data": {}, "eventType": "messageCreated" })),
        ));
    }

    assert_eq!((sink_1.sent().len(), sink_2.sent().len()), expected_frames);
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:285 "realtime delivery > publishes a guest event to its workspace-scoped topic"
#[test]
fn realtime_delivery_publishes_a_guest_event_to_its_workspace_scoped_topic() {
    publishes_guest_events(&["workspace-1"], (1, 0));
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:285 "realtime delivery > publishes legacy guest conversation ids independently per workspace"
#[test]
fn realtime_delivery_publishes_legacy_guest_conversation_ids_independently_per_workspace() {
    publishes_guest_events(&["workspace-1", "workspace-2"], (1, 1));
}

fn guest_entry(seq: u64, workspace_id: &str, guest_conversation_id: &str) -> Arc<Entry> {
    Entry::guest_event(
        Seq::new(seq, 0),
        workspace_id,
        guest_conversation_id,
        envelope(json!({ "data": {}, "eventType": "messageCreated" })),
    )
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:315 "realtime delivery > closes a socket on a backpressure drop and does not close it a second time once already closed"
#[test]
fn realtime_delivery_closes_a_socket_on_a_backpressure_drop_and_does_not_close_it_a_second_time_once_already_closed()
 {
    let (_, delivery) = create_delivery();
    let (socket, sink) = guest_socket("workspace-1", "guest-1", SendMode::Dropped);

    delivery.replay_guest_socket(
        &socket,
        &[
            guest_entry(7, "workspace-1", "guest-1"),
            guest_entry(8, "workspace-1", "guest-1"),
        ],
    );

    assert_eq!(sink.closed(), Some((4002, "backpressure-drop".to_owned())));
    assert!(socket.is_closed());
    assert_eq!(sink.ends().len(), 1);
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:348 "realtime delivery > stops replay without reading socket data after send closes the socket"
#[test]
fn realtime_delivery_stops_replay_without_reading_socket_data_after_send_closes_the_socket() {
    let (_, delivery) = create_delivery();
    let (socket, _sink) = guest_socket("workspace-1", "guest-1", SendMode::Dropped);

    assert!(!delivery.replay_guest_socket(&socket, &[guest_entry(9, "workspace-1", "guest-1")]));
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:388 "realtime delivery > continues restricted fan-out after a socket closes during send"
#[test]
fn realtime_delivery_continues_restricted_fan_out_after_a_socket_closes_during_send() {
    let (_, delivery) = create_delivery();
    let (closing_socket, _) = member_socket(assigned(&["team-1"]), SendMode::Dropped);
    let (receiving_socket, receiving_sink) = member_socket(assigned(&["team-1"]), SendMode::Ok);
    delivery.add_workspace_socket(&closing_socket);
    delivery.add_workspace_socket(&receiving_socket);

    delivery.dispatch(&workspace_entry(json!([{
        "data": {},
        "eventType": "messageCreated",
        "route": { "assignedTeamIds": ["team-1"], "assignedUserIds": [] },
    }])));

    assert_eq!(receiving_sink.sent().len(), 1);
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:417 "realtime delivery > resyncWorkspace closes every registered member and guest socket of that workspace, and only that workspace"
#[test]
fn realtime_delivery_resyncworkspace_closes_every_registered_member_and_guest_socket_of_that_workspace_and_only_that_workspace()
 {
    let (_, delivery) = create_delivery();
    let (member, member_sink) = member_socket(member_data(), SendMode::Ok);
    let (other_member, other_member_sink) = member_socket(
        MemberSocketData {
            user_id: "user-2".to_owned(),
            workspace_id: "workspace-2".to_owned(),
            ..member_data()
        },
        SendMode::Ok,
    );
    let (guest, guest_sink) = guest_socket("workspace-1", "guest-1", SendMode::Dropped);
    let (other_guest, other_guest_sink) = guest_socket("workspace-2", "guest-2", SendMode::Dropped);
    delivery.add_workspace_socket(&member);
    delivery.add_workspace_socket(&other_member);
    delivery.add_guest_socket(&guest);
    delivery.add_guest_socket(&other_guest);

    delivery.resync_workspace("workspace-1", "malformed-stream-record");

    assert_eq!(
        member_sink.closed(),
        Some((4002, "malformed-stream-record".to_owned()))
    );
    assert_eq!(
        guest_sink.closed(),
        Some((4002, "malformed-stream-record".to_owned()))
    );
    assert_eq!(other_member_sink.closed(), None);
    assert_eq!(other_guest_sink.closed(), None);
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:451 "realtime delivery > does not deliver a routed event assigned to another team or user to an assigned-scope socket"
#[test]
fn realtime_delivery_does_not_deliver_a_routed_event_assigned_to_another_team_or_user_to_an_assigned_scope_socket()
 {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(assigned(&["team-1"]), SendMode::Ok);
    delivery.add_workspace_socket(&socket);

    delivery.dispatch(&workspace_entry(json!([{
        "data": {},
        "eventType": "messageCreated",
        "route": { "assignedTeamIds": ["team-2"], "assignedUserIds": ["user-2"] },
    }])));

    assert_eq!(sink.sent(), Vec::<String>::new());
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:472 "realtime delivery > delivers nothing to a chatScope: none socket, routed or not"
#[test]
fn realtime_delivery_delivers_nothing_to_a_chatscope_none_socket_routed_or_not() {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(
        MemberSocketData {
            chat_scope: ChatScope::None,
            team_ids: strings(&["team-1"]),
            ..member_data()
        },
        SendMode::Ok,
    );
    delivery.add_workspace_socket(&socket);

    delivery.dispatch(&workspace_entry(json!([
        { "data": {}, "eventType": "messageCreated" },
        {
            "data": {},
            "eventType": "messageCreated",
            "route": { "assignedTeamIds": ["team-1"], "assignedUserIds": ["user-1"] },
        },
    ])));

    assert_eq!(sink.sent(), Vec::<String>::new());
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:491 "realtime delivery > replay applies the same scope filter as live dispatch"
#[test]
fn realtime_delivery_replay_applies_the_same_scope_filter_as_live_dispatch() {
    let (_, delivery) = create_delivery();
    let (assigned_socket, assigned_sink) = member_socket(assigned(&["team-1"]), SendMode::Ok);
    let (none_socket, none_sink) = member_socket(
        MemberSocketData {
            chat_scope: ChatScope::None,
            user_id: "user-2".to_owned(),
            ..member_data()
        },
        SendMode::Ok,
    );
    let entries = [Entry::workspace_events(
        Seq::new(9, 0),
        "workspace-1",
        envelopes(json!([
            {
                "data": { "id": "mine" },
                "eventType": "messageCreated",
                "route": { "assignedTeamIds": ["team-1"], "assignedUserIds": ["user-1"] },
            },
            {
                "data": { "id": "not-mine" },
                "eventType": "messageCreated",
                "route": { "assignedTeamIds": ["team-2"], "assignedUserIds": ["user-2"] },
            },
        ])),
    )];

    delivery.replay_workspace_socket(&assigned_socket, &entries);
    delivery.replay_workspace_socket(&none_socket, &entries);

    assert_eq!(
        assigned_sink.sent(),
        vec![
            json!({
                "batch": [{
                    "data": { "id": "mine" },
                    "eventType": "messageCreated",
                    "route": { "assignedTeamIds": ["team-1"], "assignedUserIds": ["user-1"] },
                }],
                "seq": "9-0",
            })
            .to_string()
        ]
    );
    assert_eq!(none_sink.sent(), Vec::<String>::new());
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:545 "realtime delivery > never replays a guest record from another workspace onto a guest socket, even sharing the same conversation id"
#[test]
fn realtime_delivery_never_replays_a_guest_record_from_another_workspace_onto_a_guest_socket_even_sharing_the_same_conversation_id()
 {
    let (_, delivery) = create_delivery();
    let (socket, sink) = guest_socket("workspace-a", "12345", SendMode::Dropped);

    delivery.replay_guest_socket(&socket, &[guest_entry(10, "workspace-b", "12345")]);

    assert_eq!(sink.closed(), None);
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:567 "realtime delivery > sendCursor sends an empty-batch frame carrying only the seq"
#[test]
fn realtime_delivery_sendcursor_sends_an_empty_batch_frame_carrying_only_the_seq() {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(member_data(), SendMode::Ok);

    delivery.send_cursor(&socket, "5-0");

    assert_eq!(
        sink.sent(),
        vec![json!({ "batch": [], "seq": "5-0" }).to_string()]
    );
}

/// Port of: apps/realtime/__tests__/delivery.test.ts:576 "realtime delivery > sendCursor is a no-op once the socket is already closed"
#[test]
fn realtime_delivery_sendcursor_is_a_no_op_once_the_socket_is_already_closed() {
    let (_, delivery) = create_delivery();
    let (socket, sink) = member_socket(member_data(), SendMode::Ok);
    socket.mark_closed();

    delivery.send_cursor(&socket, "5-0");

    assert_eq!(sink.sent(), Vec::<String>::new());
}
