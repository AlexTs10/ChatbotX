//! Channel senders kept in this crate: webchat (the guest realtime
//! stream), the API channel (signed callback POST), WhatsApp (Q.I1a), the Meta
//! channels (Q.I1b) and Telegram, TikTok, Zalo (Q.I2).

pub mod api;
pub mod graph;
pub mod instagram;
pub mod messenger;
pub mod messenger_webhook;
pub mod meta;
pub mod meta_webhook;
pub mod telegram;
pub mod threads;
pub mod tiktok;
pub mod webchat;
pub mod whatsapp;
pub mod whatsapp_media;
pub mod whatsapp_steps;
pub mod whatsapp_template;
pub mod whatsapp_webhook;
pub mod zalo;
