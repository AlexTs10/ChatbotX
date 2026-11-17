//! Channel senders kept in this crate: webchat (the guest realtime
//! stream), the API channel (signed callback POST) and WhatsApp (Q.I1a).

pub mod api;
pub mod telegram;
pub mod tiktok;
pub mod webchat;
pub mod whatsapp;
pub mod whatsapp_media;
pub mod whatsapp_steps;
pub mod whatsapp_template;
pub mod whatsapp_webhook;
pub mod zalo;
