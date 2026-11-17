//! Channel senders kept in this crate: webchat (the guest realtime
//! stream), the API channel (signed callback POST), WhatsApp (Q.I1a) and
//! the Meta channels (Q.I1b).

pub mod api;
pub mod graph;
pub mod instagram;
pub mod messenger;
pub mod messenger_webhook;
pub mod meta;
pub mod meta_webhook;
pub mod webchat;
pub mod whatsapp;
pub mod whatsapp_webhook;
