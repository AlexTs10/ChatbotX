//! `integrations/webchat/src/handlers/{message,conversation}.ts` (outgoing
//! side): the guest realtime stream is the channel.

use chatbotx_jobs::BoxFuture;
use chatbotx_rt::{Envelope, EventType};
use serde_json::{Value, json};

use crate::channel::{
    Channel, ChannelCtx, ChannelResult, SendFlowStep, SendMessage, SendResult, SendTyping,
};

pub struct Webchat;

/// `ctx.platform.publishGuestRealtimeEvent(sourceId, ...)`: best effort,
/// a failure is logged and never fails the send.
fn publish_guest(ctx: &ChannelCtx, source_id: &str, event_type: EventType, data: Value) {
    let Some(realtime) = &ctx.realtime else {
        return;
    };
    if let Err(err) = realtime.publish_guest_event(
        &ctx.workspace_id.to_string(),
        source_id,
        Envelope::new(event_type, data),
    ) {
        tracing::error!(err = %err, contact_source_id = source_id, "Failed to publish guest realtime event");
    }
}

fn accepted() -> BoxFuture<'static, ChannelResult<SendResult>> {
    Box::pin(std::future::ready(Ok(SendResult {
        message_ids: Vec::new(),
        sent_count: 1,
    })))
}

impl Channel for Webchat {
    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        publish_guest(
            ctx,
            &data.contact.contact_inbox.source_id,
            EventType::MessageCreated,
            data.message,
        );
        accepted()
    }

    /// The worker publishes the stored flow message to the guest itself
    /// (`send-flow-step.ts`); this still counts one accepted message.
    fn send_flow_step(
        &self,
        _ctx: &ChannelCtx,
        _data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        accepted()
    }

    fn send_typing(
        &self,
        ctx: &ChannelCtx,
        data: SendTyping,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        publish_guest(
            ctx,
            &data.contact.contact_inbox.source_id,
            EventType::Typing,
            json!({ "typing": data.typing }),
        );
        Box::pin(std::future::ready(Ok(())))
    }
}
