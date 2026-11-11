//! Port of `packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts`.

use chatbotx_jobs::queues::whatsapp_voip::{
    ExpireOutboundDialData, HandleOutboundAnswerData, WhatsappVoipSignalingJobAction,
    WhatsappVoipSignalingJobData, expire_outbound_dial_job_id, outbound_answer_job_id,
    whatsapp_voip_expiry_job_id, whatsapp_voip_signaling_job_id,
};
use serde_json::json;

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:12 "WhatsappVoipSignalingJobAction > registers the outbound (business-initiated) job actions"
#[test]
fn registers_the_outbound_business_initiated_job_actions() {
    assert_eq!(
        WhatsappVoipSignalingJobAction::HandleOutboundAnswer.as_str(),
        "handleOutboundAnswer"
    );
    assert_eq!(
        WhatsappVoipSignalingJobAction::ExpireOutboundDial.as_str(),
        "expireOutboundDial"
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:21 "WhatsappVoipSignalingJobAction > keeps the existing inbound job actions unchanged"
#[test]
fn keeps_the_existing_inbound_job_actions_unchanged() {
    assert_eq!(
        WhatsappVoipSignalingJobAction::HandleConnect.as_str(),
        "handleConnect"
    );
    assert_eq!(
        WhatsappVoipSignalingJobAction::ExpireIfUnanswered.as_str(),
        "expireIfUnanswered"
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:30 "outbound job payload shapes > handleOutboundAnswer payload never carries an sdp field"
#[test]
fn handleoutboundanswer_payload_never_carries_an_sdp_field() {
    let payload = WhatsappVoipSignalingJobData::HandleOutboundAnswer(HandleOutboundAnswerData {
        attempt_id: "attempt-1".to_owned(),
        whatsapp_call_id: "call-1".to_owned(),
        wacid: Some("wamid.ABC".to_owned()),
        workspace_id: "ws-1".to_owned(),
    });

    let value = payload.to_value();
    assert_eq!(value["type"], "handleOutboundAnswer");
    let data = value["data"].as_object().expect("data object");
    assert!(!data.contains_key("sdp"));
    assert!(!data.contains_key("session"));
}

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:45 "outbound job payload shapes > handleOutboundAnswer payload allows an absent wacid (answer/connect race)"
#[test]
fn handleoutboundanswer_payload_allows_an_absent_wacid_answer() {
    let payload = WhatsappVoipSignalingJobData::parse(&json!({
        "type": "handleOutboundAnswer",
        "data": {
            "attemptId": "attempt-1",
            "whatsappCallId": "call-1",
            "workspaceId": "ws-1",
        },
    }))
    .expect("payload without wacid is valid");

    match payload {
        WhatsappVoipSignalingJobData::HandleOutboundAnswer(data) => {
            assert_eq!(data.wacid, None);
        }
        other => panic!("unexpected payload {other:?}"),
    }
}

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:58 "outbound job payload shapes > expireOutboundDial payload never carries an sdp field"
#[test]
fn expireoutbounddial_payload_never_carries_an_sdp_field() {
    let payload = WhatsappVoipSignalingJobData::ExpireOutboundDial(ExpireOutboundDialData {
        attempt_id: "attempt-1".to_owned(),
        whatsapp_call_id: "call-1".to_owned(),
        workspace_id: "ws-1".to_owned(),
        deadline_at: 1_700_000_000_000,
    });

    let value = payload.to_value();
    assert_eq!(value["type"], "expireOutboundDial");
    let data = value["data"].as_object().expect("data object");
    assert!(!data.contains_key("sdp"));
}

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:74 "outbound job id helpers > outboundAnswerJobId is stable and keyed by attemptId"
#[test]
fn outboundanswerjobid_is_stable_and_keyed_by_attemptid() {
    assert_eq!(
        outbound_answer_job_id("attempt-1"),
        "voip-out-answer-attempt-1"
    );
    assert_eq!(
        outbound_answer_job_id("attempt-1"),
        outbound_answer_job_id("attempt-1")
    );
    assert_ne!(
        outbound_answer_job_id("attempt-1"),
        outbound_answer_job_id("attempt-2")
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:84 "outbound job id helpers > expireOutboundDialJobId is stable and keyed by attemptId"
#[test]
fn expireoutbounddialjobid_is_stable_and_keyed_by_attemptid() {
    assert_eq!(
        expire_outbound_dial_job_id("attempt-1"),
        "voip-out-expire-attempt-1"
    );
    assert_eq!(
        expire_outbound_dial_job_id("attempt-1"),
        expire_outbound_dial_job_id("attempt-1")
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts:93 "outbound job id helpers > outbound job ids never collide with the existing inbound job ids"
#[test]
fn outbound_job_ids_never_collide_with_the_existing_inbound_job() {
    assert_ne!(
        outbound_answer_job_id("wacid-1"),
        whatsapp_voip_signaling_job_id("wacid-1")
    );
    assert_ne!(
        expire_outbound_dial_job_id("wacid-1"),
        whatsapp_voip_expiry_job_id("wacid-1")
    );
}
