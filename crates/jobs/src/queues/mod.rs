//! Per-queue payload types, job actions, job-id helpers and job options
//! (`packages/worker-config/src/queues/*`).

pub mod ai_agent;
pub mod ai_file;
pub mod broadcast;
pub mod coexist;
pub mod default_jobs;
pub mod heavy;
pub mod inbound_webhook;
pub mod integration;
pub mod marketing_message;
pub mod resolve_step_input;
pub mod stored_image;
pub mod template;
pub mod whatsapp_call;
pub mod whatsapp_identity;
pub mod whatsapp_voip;
