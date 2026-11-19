//! Port of `packages/business/__tests__/whatsapp-call-voip-call-service.test.ts`
//! (the call control: offer, reservation, claim, commit, release, end).
//! Adapted: the mocked Redis `SET NX` / CAS calls become the durable KV
//! (`VoipStore`) of a migrated in-memory database; "called with" becomes the
//! stored control. The ring-target cases are in `whatsapp_call_ring_targets.rs`.
use std::sync::Arc;

use chatbotx_business::whatsapp_voip::{
    EndAction, Phase, VoipStore, end_outcome, is_transition_allowed,
};
use chatbotx_db::Db;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::partials::whatsapp_call::WhatsappCallTerminalStatus;

async fn store() -> VoipStore {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    VoipStore::new(Arc::new(db))
}

fn deadline() -> i64 {
    now_ms() + 55_000
}

#[tokio::test]
async fn redelivery_of_an_offer_is_a_no_op() {
    let s = store().await;
    assert!(s.store_offer("w1", "v=0", deadline()).await.unwrap());
    assert!(!s.store_offer("w1", "v=1", deadline()).await.unwrap());
    assert_eq!(s.read_offer("w1").await.unwrap().as_deref(), Some("v=0"));
    s.delete_offer("w1").await.unwrap();
    assert_eq!(s.read_offer("w1").await.unwrap(), None);
}

#[tokio::test]
async fn claims_the_ring_control_as_terminated_and_reports_the_loss_after() {
    let s = store().await;
    assert!(s.claim_unreachable("w1", deadline()).await.unwrap());
    assert!(!s.claim_unreachable("w1", deadline()).await.unwrap());
    // A reservation attempt that follows the claim reports alreadyProgressed.
    assert!(!s.reserve_incoming("w1", deadline()).await.unwrap());
}

#[tokio::test]
async fn creates_one_unclaimed_control_and_observes_it_on_redelivery() {
    let s = store().await;
    assert!(s.reserve_incoming("w1", deadline()).await.unwrap());
    assert!(s.reserve_incoming("w1", deadline()).await.unwrap());
    let control = s.read_control("w1").await.unwrap().unwrap();
    assert_eq!(control.phase, Phase::Reserved);
    assert_eq!(control.reserved_user_id, "");
}

#[tokio::test]
async fn any_live_agent_wins_the_claim_and_a_second_answerer_loses() {
    let s = store().await;
    s.reserve_incoming("w1", deadline()).await.unwrap();
    let fence = s
        .claim_for_answer("w1", "7")
        .await
        .unwrap()
        .expect("first claim wins");
    assert_eq!(s.claim_for_answer("w1", "8").await.unwrap(), None);
    let control = s.read_control("w1").await.unwrap().unwrap();
    assert_eq!(
        (control.phase, control.reserved_user_id.as_str()),
        (Phase::Answering, "7")
    );
    // A fence mismatch is rejected; the matching fence commits.
    assert!(!s.commit_accepted("w1", "wrong").await.unwrap());
    assert!(s.commit_accepted("w1", &fence).await.unwrap());
    assert_eq!(
        s.read_control("w1").await.unwrap().unwrap().phase,
        Phase::Accepted
    );
    // Already accepted: no new claim.
    assert_eq!(s.claim_for_answer("w1", "8").await.unwrap(), None);
}

#[tokio::test]
async fn release_returns_an_answering_claim_to_reserved() {
    let s = store().await;
    s.reserve_incoming("w1", deadline()).await.unwrap();
    let fence = s.claim_for_answer("w1", "7").await.unwrap().unwrap();
    assert!(s.release_claim("w1", &fence).await.unwrap());
    assert_eq!(
        s.read_control("w1").await.unwrap().unwrap().phase,
        Phase::Reserved
    );
}

#[tokio::test]
async fn end_call_derives_the_graph_action_from_the_phase() {
    let s = store().await;
    s.reserve_incoming("w1", deadline()).await.unwrap();
    let ended = s.end_call("w1", false).await.unwrap().unwrap();
    assert_eq!(
        (ended.graph_action, ended.terminal_status),
        (EndAction::Reject, WhatsappCallTerminalStatus::Rejected)
    );
    // Terminated: nothing left to end.
    assert_eq!(s.end_call("w1", true).await.unwrap(), None);

    s.reserve_incoming("w2", deadline()).await.unwrap();
    s.claim_for_answer("w2", "7").await.unwrap().unwrap();
    let ended = s.end_call("w2", false).await.unwrap().unwrap();
    assert_eq!(
        (ended.graph_action, ended.terminal_status),
        (EndAction::Terminate, WhatsappCallTerminalStatus::Failed)
    );
}

#[tokio::test]
async fn an_accepted_call_ends_only_when_allowed() {
    let s = store().await;
    s.reserve_incoming("w1", deadline()).await.unwrap();
    let fence = s.claim_for_answer("w1", "7").await.unwrap().unwrap();
    s.commit_accepted("w1", &fence).await.unwrap();
    assert_eq!(s.end_call("w1", false).await.unwrap(), None);
    let ended = s.end_call("w1", true).await.unwrap().unwrap();
    assert_eq!(ended.terminal_status, WhatsappCallTerminalStatus::Completed);
}

#[test]
fn the_phase_graph_only_moves_forward() {
    assert!(is_transition_allowed(Phase::Reserved, Phase::Answering));
    assert!(is_transition_allowed(Phase::Answering, Phase::Accepted));
    assert!(!is_transition_allowed(Phase::Accepted, Phase::Answering));
    assert!(!is_transition_allowed(Phase::Terminated, Phase::Reserved));
    assert_eq!(end_outcome(Phase::Terminated), None);
}
