//! Channel senders kept in this crate: webchat (the guest realtime
//! stream), the API channel (signed callback POST) and WhatsApp (Q.I1a).

pub mod api;
pub mod webchat;
pub mod whatsapp;
pub mod whatsapp_webhook;
