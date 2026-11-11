//! Port of `packages/worker-config/__tests__/whatsapp-identity-change-job.test.ts`.

use chatbotx_jobs::queues::whatsapp_identity::{
    WhatsappIdentityChangeData, WhatsappIdentityChangeJob, WhatsappIdentityChangeKind,
    WhatsappIdentityChangePayload,
};

/// Port of: packages/worker-config/__tests__/whatsapp-identity-change-job.test.ts:8 "WhatsApp identity-change integration job > exposes the action and typed normalized payload"
#[test]
fn exposes_the_action_and_typed_normalized_payload() {
    // The typed struct literal keeps the compile-time `satisfies` check.
    let job = WhatsappIdentityChangeJob {
        data: WhatsappIdentityChangeData {
            integration_type: "whatsapp".to_owned(),
            integration_identifier: "phone-1".to_owned(),
            payload: WhatsappIdentityChangePayload {
                phone_number_id: "phone-1".to_owned(),
                message_id: "wamid.1".to_owned(),
                change: WhatsappIdentityChangeKind::UserIdChanged {
                    user_id: "bsuid-new".to_owned(),
                    previous_phone: "84900000001".to_owned(),
                    new_phone: "84900000002".to_owned(),
                },
            },
        },
    };

    assert_eq!(job.job_type(), "whatsappIdentityChange");
    assert_eq!(job.to_value()["type"], "whatsappIdentityChange");
}
