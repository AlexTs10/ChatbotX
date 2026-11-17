//! `GET|POST /integrations/{type}/webhook[/c/{credentialId}]` for the Meta
//! channels (P1.21, Q.I1b; `app/integrations/[...integration]/webhook.ts`):
//! resolves the platform credential (credential path, broker host, tenant
//! custom domain, or the single-tenant platform row), runs the channel's
//! webhook handler and queues its jobs. A credential-scoped route only
//! queues jobs for inboxes its owner owns.

use std::collections::HashMap;
use std::sync::Arc;

use chatbotx_business::platform_credential::{self, DecryptOptions, DecryptedCredential};
use chatbotx_core::encryption::Encryption;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::OptionalExtension;
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use serde_json::{Value, json};

use super::messenger_webhook::{self, Outcome};
use super::whatsapp_webhook::{WebhookJob, WebhookReply};

/// The Meta channels this route serves.
pub const TYPES: [&str; 2] = ["messenger", "instagram"];

pub struct MetaWebhook {
    pub db: Arc<Db>,
    pub jobs: Arc<dyn Enqueuer>,
    pub encryption: Option<Arc<Encryption>>,
    pub is_cloud: bool,
    /// `new URL(getBrokerOrigin()).hostname`.
    pub broker_host: Option<String>,
}

/// `getBrokerOrigin()` host: `NEXT_PUBLIC_BROKER_URL ?? NEXT_PUBLIC_BUILDER_URL`.
pub fn broker_host_from_env() -> Option<String> {
    let origin = std::env::var("NEXT_PUBLIC_BROKER_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("NEXT_PUBLIC_BUILDER_URL").ok())?;
    url::Url::parse(&origin).ok()?.host_str().map(str::to_owned)
}

/// `ENCRYPTION_KEY` keys (`None` = credentials cannot be read).
pub fn encryption_from_env() -> Option<Arc<Encryption>> {
    chatbotx_core::encryption::Keys::from_env(&chatbotx_core::env::ProcessEnv)
        .ok()
        .map(|keys| Arc::new(Encryption::new(keys)))
}

fn json_error(status: u16, message: &str) -> WebhookReply {
    WebhookReply {
        status,
        content_type: "application/json",
        body: json!({ "message": message }).to_string(),
    }
}

fn text(body: String) -> WebhookReply {
    WebhookReply {
        status: 200,
        content_type: "text/plain; charset=utf-8",
        body,
    }
}

fn not_configured() -> WebhookReply {
    json_error(404, "Integration is not configured")
}

/// One webhook request.
pub struct MetaRequest<'a> {
    pub kind: &'static str,
    pub credential_id: Option<&'a str>,
    pub is_get: bool,
    pub query: Option<&'a str>,
    /// `x-domain` header.
    pub domain: &'a str,
    /// `x-hub-signature-256` header.
    pub signature: Option<&'a str>,
    pub body: &'a [u8],
}

impl MetaWebhook {
    pub async fn handle(&self, req: MetaRequest<'_>) -> WebhookReply {
        let kind = req.kind;
        let Some(encryption) = self.encryption.clone() else {
            tracing::debug!("Integration {kind} is not configured (no ENCRYPTION_KEY)");
            return not_configured();
        };
        let (credential, owner) = match self
            .resolve_credential(&encryption, kind, req.credential_id, req.domain)
            .await
        {
            Ok(Some(found)) => found,
            Ok(None) => return not_configured(),
            Err(e) => {
                tracing::error!(err = %e, integration_type = kind, "Integration handleRequest failed");
                return json_error(400, &e);
            }
        };
        let jobs_of = match kind {
            "instagram" => super::instagram::webhook_jobs,
            _ => messenger_webhook::webhook_jobs,
        };
        let outcome = messenger_webhook::handle_request_with(
            req.is_get,
            req.query,
            req.signature,
            req.body,
            credential.text("verifyToken"),
            credential.text("clientSecret").unwrap_or(""),
            jobs_of,
        );
        let jobs = match outcome {
            Ok(Outcome::Challenge(challenge)) => return text(challenge),
            Ok(Outcome::Jobs(jobs)) => jobs,
            Err(message) => {
                tracing::error!(err = %message, integration_type = kind, "Integration handleRequest failed");
                return json_error(400, &message);
            }
        };
        if let Err(e) = self.enqueue(jobs, owner).await {
            let message = messenger_webhook::event_error(&e);
            tracing::error!(err = %message, integration_type = kind, "Integration handleRequest failed");
            return json_error(400, &message);
        }
        text("ok".into())
    }

    /// The credential and, for `/c/{id}`, the owner whose inboxes it serves.
    async fn resolve_credential(
        &self,
        encryption: &Arc<Encryption>,
        kind: &'static str,
        credential_id: Option<&str>,
        domain: &str,
    ) -> Result<Option<(DecryptedCredential, Option<i64>)>, String> {
        if let Some(id) = credential_id {
            if !self.is_cloud {
                return Ok(None);
            }
            let Ok(id) = id.parse::<i64>() else {
                return Ok(None);
            };
            let encryption = Arc::clone(encryption);
            let credential = self
                .db
                .read(move |c| {
                    platform_credential::find_decrypted_for_webhook(c, &encryption, id, kind)
                        .map_err(|e| chatbotx_db::DbError::other(e.message()))
                })
                .await
                .map_err(|e| e.to_string())?;
            return Ok(credential.and_then(|c| c.user_id.map(|owner| (c, Some(owner)))));
        }
        let user = if !self.is_cloud || self.broker_host.as_deref() == Some(domain) {
            None
        } else {
            let custom = if domain.is_empty() {
                None
            } else {
                chatbotx_business::tenant::find_active_domain(&self.db, domain)
                    .await
                    .map_err(|e| e.message().to_string())?
            };
            let Some(custom) = custom else {
                tracing::debug!(
                    integration_type = kind,
                    domain,
                    "No active custom domain for integration webhook"
                );
                return Ok(None);
            };
            let tenant = chatbotx_business::tenant::find_by_id(&self.db, custom.tenant_id)
                .await
                .map_err(|e| e.message().to_string())?;
            match tenant {
                Some(t) if t.status == "active" && t.owner_id.is_some() => t.owner_id,
                _ => {
                    tracing::debug!(
                        integration_type = kind,
                        domain,
                        "Tenant disabled for integration webhook"
                    );
                    return Ok(None);
                }
            }
        };
        let encryption = Arc::clone(encryption);
        let options = DecryptOptions {
            require_own_credential: user.is_some(),
            strict: false,
        };
        let credential = self
            .db
            .read(move |c| {
                platform_credential::find_decrypted(c, &encryption, user, kind, options)
                    .map_err(|e| chatbotx_db::DbError::other(e.message()))
            })
            .await
            .map_err(|e| e.to_string())?;
        if credential.is_none() {
            tracing::debug!("Integration {kind} is not configured");
        }
        Ok(credential.map(|c| (c, None)))
    }
}

/// `createCredentialScopedQueue`'s owner check: the job's
/// `integrationType` + `integrationIdentifier` must name an inbox of a
/// workspace the owner owns.
async fn belongs_to_owner(db: &Db, data: &Value, owner: i64) -> Result<bool, String> {
    let routing = data.get("data").and_then(|d| {
        Some((
            d.get("integrationType")?.as_str()?.to_owned(),
            d.get("integrationIdentifier")?.as_str()?.to_owned(),
        ))
    });
    let Some((kind, identifier)) = routing else {
        tracing::error!(
            owner,
            "Ignoring credential-scoped webhook job without ownership routing fields"
        );
        return Ok(false);
    };
    let Some((table, column)) = crate::channel::identifier_column(&kind) else {
        return Ok(false);
    };
    let found = db
        .read(move |c| {
            Ok(c.query_row(
                &format!(
                    r#"SELECT w."ownerId" FROM "{table}" t JOIN "Inbox" i ON i."id" = t."inboxId"
                       JOIN "Workspace" w ON w."id" = i."workspaceId" WHERE t."{column}" = ?1 LIMIT 1"#
                ),
                [&identifier],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?)
        })
        .await
        .map_err(|e| e.to_string())?;
    match found {
        Some(inbox_owner) if inbox_owner == Some(owner) => Ok(true),
        Some(_) => {
            tracing::warn!(
                owner,
                "Ignoring credential-scoped webhook for another owner's inbox"
            );
            Ok(false)
        }
        None => {
            tracing::warn!(
                owner,
                "Ignoring credential-scoped webhook for an unresolved inbox"
            );
            Ok(false)
        }
    }
}

impl MetaWebhook {
    async fn enqueue(&self, jobs: Vec<WebhookJob>, owner: Option<i64>) -> Result<(), String> {
        let mut checked: HashMap<String, bool> = HashMap::new();
        for job in jobs {
            if let Some(owner) = owner {
                let key = format!(
                    "{}:{}",
                    job.data
                        .pointer("/data/integrationType")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    job.data
                        .pointer("/data/integrationIdentifier")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                );
                let allowed = match checked.get(&key) {
                    Some(allowed) => *allowed,
                    None => {
                        let allowed = belongs_to_owner(&self.db, &job.data, owner).await?;
                        checked.insert(key, allowed);
                        allowed
                    }
                };
                if !allowed {
                    continue;
                }
            }
            // ponytail: `enqueueIntegrationJob` routes echoes to the `low`
            // queue; Rust runs no low worker, so every job goes to `integration`.
            self.jobs
                .add(
                    QueueName::Integration,
                    job.name,
                    job.data,
                    JobOptions {
                        job_id: (!job.job_id.is_empty()).then_some(job.job_id),
                        ..JobOptions::default()
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
