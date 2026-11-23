//! `runRef` (`integration/handlers/ref.ts`): a contact arrived through a
//! ref (flow / draft link, ref link or QR code, minigame share, leaderboard
//! referral). The ref is unsigned: every lookup is scoped to the
//! conversation's workspace, and a stale or crafted ref is a logged skip.
//!
//! ponytail: a poll ref (`pl_`) is skipped: the poll chat runtime (poll card
//! send through the rich-response store) is not ported.

use std::sync::Arc;

use chatbotx_business::events::Event;
use chatbotx_business::referral::RefConfig;
use chatbotx_core::utils::encode::decode_base62;
use chatbotx_db::repo::models::{ContactInbox, Conversation, Minigame};
use chatbotx_db::repo::{find_one, iso, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::{Deps, db_fail, fail, id_of, load_contact_inbox, load_conversation, str_of};

pub const RUN_REF: &str = "runRef";

/// `decodeRef` (an empty id segment is no ref).
pub fn decode_ref(r#ref: &str) -> Option<RefConfig> {
    let id = |s: &str| -> Option<i64> {
        (!s.is_empty())
            .then(|| decode_base62(s).ok()?.parse().ok())
            .flatten()
    };
    let pair = |rest: &str| -> Option<(i64, i64)> {
        let mut parts = rest.split('_');
        Some((id(parts.next()?)?, id(parts.next()?)?))
    };
    if let Some(rest) = r#ref.strip_prefix("f_") {
        let mut parts = rest.split('_');
        let flow_id = id(parts.next()?)?;
        let node_id = parts
            .next()
            .filter(|n| !n.is_empty())
            .and_then(|n| decode_base62(n).ok());
        return Some(RefConfig::Flow { flow_id, node_id });
    }
    if let Some(rest) = r#ref.strip_prefix("d_") {
        return Some(RefConfig::Draft { flow_id: id(rest)? });
    }
    if let Some(rest) = r#ref.strip_prefix("mg_") {
        let (minigame_id, referrer_contact_id) = pair(rest)?;
        return Some(RefConfig::MinigameShare {
            minigame_id,
            referrer_contact_id,
        });
    }
    if let Some(rest) = r#ref.strip_prefix("lb_") {
        let (leaderboard_id, contact_id) = pair(rest)?;
        return Some(RefConfig::Leaderboard {
            leaderboard_id,
            contact_id,
        });
    }
    if let Some(rest) = r#ref.strip_prefix("pl_") {
        return Some(RefConfig::Poll { poll_id: id(rest)? });
    }
    if let Some(name) = r#ref.strip_prefix("qr_") {
        return Some(RefConfig::QrCode { name: name.into() });
    }
    Some(RefConfig::Reflink {
        name: r#ref.to_owned(),
    })
}

/// `MINIGAME_SHARE_CHANNELS` (`canShareMinigame`): the ref channels but webchat.
fn can_share_minigame(channel: &str) -> bool {
    channel != "webchat" && chatbotx_business::inbox::links::can_receive_ref(channel)
}

struct RefRun<'a> {
    deps: &'a Deps,
    conversation: Conversation,
    contact_inbox: ContactInbox,
}

impl RefRun<'_> {
    fn ws(&self) -> i64 {
        self.conversation.workspace_id
    }

    /// The flow of this workspace (`findOrFail(flowModel)`), else `None`.
    async fn flow_exists(&self, flow_id: i64) -> Result<bool, JobError> {
        let ws = self.ws();
        self.deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                        params![flow_id, ws],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            })
            .await
            .map_err(db_fail)
    }

    /// `integrationQueue.add(sendFlow, ...)` with `origin: "channel"`.
    async fn send_flow(
        &self,
        flow_id: i64,
        flow_version_id: Option<i64>,
        node_id: Option<&str>,
    ) -> Result<(), JobError> {
        let mut data = json!({
            "conversationId": self.conversation.id.to_string(),
            "contactInboxId": self.contact_inbox.id.to_string(),
            "flowId": flow_id.to_string(),
            "origin": "channel",
        });
        if let Some(version) = flow_version_id {
            data["flowVersionId"] = json!(version.to_string());
        }
        if let Some(node) = node_id {
            data["nodeId"] = json!(node);
        }
        self.deps
            .jobs
            .add(
                QueueName::Integration,
                crate::flow::SEND_FLOW,
                json!({ "type": crate::flow::SEND_FLOW, "data": data }),
                JobOptions::default(),
            )
            .await
            .map(|_| ())
            .map_err(db_fail)
    }

    /// Returns whether a flow was dispatched (`emitSuccess`).
    async fn dispatch(&self, config: RefConfig, is_new_contact: bool) -> Result<bool, JobError> {
        let ws = self.ws();
        match config {
            RefConfig::Draft { flow_id } => {
                let version = self
                    .deps
                    .db
                    .read(move |conn| {
                        Ok(conn
                            .query_row(
                                r#"SELECT v."id" FROM "FlowVersion" v JOIN "Flow" f ON f."id" = v."flowId"
                                   WHERE v."flowId" = ?1 AND v."isDraft" = 1 AND f."workspaceId" = ?2 LIMIT 1"#,
                                params![flow_id, ws],
                                |r| r.get::<_, i64>(0),
                            )
                            .optional()?)
                    })
                    .await
                    .map_err(db_fail)?;
                let Some(version) = version else {
                    return Err(fail("Flow version not found"));
                };
                self.send_flow(flow_id, Some(version), None).await?;
                Ok(true)
            }
            RefConfig::Flow { flow_id, node_id } => {
                if !self.flow_exists(flow_id).await? {
                    return Err(fail("Flow not found"));
                }
                self.send_flow(flow_id, None, node_id.as_deref()).await?;
                Ok(true)
            }
            RefConfig::MinigameShare {
                minigame_id,
                referrer_contact_id,
            } => self.minigame_share(minigame_id, referrer_contact_id).await,
            RefConfig::Leaderboard {
                leaderboard_id,
                contact_id,
            } => self.leaderboard(leaderboard_id, contact_id).await,
            RefConfig::Poll { poll_id } => {
                tracing::warn!(
                    poll_id,
                    "Poll ref: the poll chat runtime is not ported; skipped"
                );
                Ok(false)
            }
            config @ (RefConfig::Reflink { .. } | RefConfig::QrCode { .. }) => {
                self.reflink(&config, is_new_contact).await?;
                Ok(true)
            }
        }
    }

    async fn minigame_share(&self, minigame_id: i64, referrer: i64) -> Result<bool, JobError> {
        let ws = self.ws();
        let minigame: Option<Minigame> = self
            .deps
            .db
            .read(move |conn| {
                find_one(conn, r#""id" = ?1"#, &[&minigame_id]).map_err(crate::repo_err)
            })
            .await
            .map_err(db_fail)?;
        let Some(minigame) = minigame.filter(|m| m.workspace_id == ws) else {
            tracing::warn!(
                minigame_id,
                "Minigame share ref: minigame not found in workspace"
            );
            return Ok(false);
        };
        if !minigame.enabled
            || !chatbotx_business::minigame::play::is_within_play_window(
                &minigame.general_settings.0,
                now_ms(),
            )
        {
            tracing::warn!(
                minigame_id,
                "Minigame share ref: minigame is disabled or outside its play window"
            );
            return Ok(false);
        }
        let settings = &minigame.player_settings.0;
        let sharing_flow = settings.get("sharingFlowId").and_then(id_of);
        let sharing_node = str_of(settings, "sharingNodeId").filter(|n| !n.is_empty());
        let mut dispatched = false;
        if let (Some(flow_id), Some(node_id)) = (sharing_flow, sharing_node) {
            match self.flow_exists(flow_id).await {
                Ok(true) => match self.send_flow(flow_id, None, Some(node_id)).await {
                    Ok(()) => dispatched = true,
                    Err(err) => {
                        tracing::warn!(err = %err, "Minigame share ref: sharing flow unavailable")
                    }
                },
                Ok(false) => {
                    tracing::warn!(flow_id, "Minigame share ref: sharing flow unavailable")
                }
                Err(err) => {
                    tracing::warn!(err = %err, "Minigame share ref: sharing flow unavailable")
                }
            }
        }
        // Credited even when no flow ran: the referral is about the arrival.
        let play = chatbotx_business::minigame::play::PlayDeps {
            db: &self.deps.db,
            jobs: Some(Arc::clone(&self.deps.jobs)),
            random: Arc::new(chatbotx_core::utils::id::math_random),
        };
        if let Err(err) = chatbotx_business::minigame::play::credit_shared_link_referral(
            &play,
            &minigame,
            self.conversation.contact_id,
            Some(self.contact_inbox.id),
            referrer,
        )
        .await
        {
            tracing::warn!(err = %err, "Failed to credit minigame share referral");
        }
        Ok(dispatched)
    }

    async fn leaderboard(&self, leaderboard_id: i64, referrer: i64) -> Result<bool, JobError> {
        let ws = self.ws();
        let board =
            chatbotx_business::leaderboard::service::find_unscoped(&self.deps.db, leaderboard_id)
                .await
                .ok()
                .flatten();
        let Some(board) = board.filter(|b| b.workspace_id == ws) else {
            tracing::warn!(leaderboard_id, "Leaderboard ref: not in workspace");
            return Ok(false);
        };
        if !can_share_minigame(&self.contact_inbox.channel)
            || board
                .inbox_id
                .is_some_and(|inbox| inbox != self.contact_inbox.inbox_id)
        {
            tracing::warn!(leaderboard_id, channel = %self.contact_inbox.channel, "Leaderboard ref: arrival channel cannot earn referral credit");
            return Ok(false);
        }
        if let Err(err) = chatbotx_business::leaderboard::scoring::credit_referral(
            &self.deps.db,
            ws,
            board.id,
            self.conversation.contact_id,
            referrer,
        )
        .await
        {
            tracing::warn!(err = %err, leaderboard_id, "Failed to credit leaderboard referral");
        }
        let Some(flow_id) = board.referral_welcome_flow_id else {
            return Ok(false);
        };
        match self.flow_exists(flow_id).await {
            Ok(true) => match self.send_flow(flow_id, None, None).await {
                Ok(()) => Ok(true),
                Err(err) => {
                    tracing::warn!(err = %err, "Leaderboard ref: welcome flow unavailable");
                    Ok(false)
                }
            },
            _ => {
                tracing::warn!(flow_id, "Leaderboard ref: welcome flow unavailable");
                Ok(false)
            }
        }
    }

    /// `handleReflink`: the ref-link stat (`flow:ref`), the flow, the
    /// referral trigger and the optional custom field.
    async fn reflink(&self, config: &RefConfig, is_new_contact: bool) -> Result<(), JobError> {
        let ws = self.ws();
        let name = chatbotx_business::referral::encode_ref(config);
        let lookup = name.clone();
        let reflink = self
            .deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "id", "flowId", "customFieldId" FROM "Reflink" WHERE "name" = ?1 AND "workspaceId" = ?2"#,
                        params![lookup, ws],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)),
                    )
                    .optional()?)
            })
            .await
            .map_err(db_fail)?;
        let Some((reflink_id, flow_id, custom_field_id)) = reflink else {
            return Err(fail("Reflink not found"));
        };
        let (conversation, contact_inbox) = (&self.conversation, &self.contact_inbox);
        if let Some(buses) = self.deps.events.as_ref() {
            let event = json!({
                "context": {
                    "workspaceId": ws.to_string(),
                    "contactId": conversation.contact_id.to_string(),
                    "conversationId": conversation.id.to_string(),
                    "channel": contact_inbox.channel,
                    "contactInboxId": contact_inbox.id.to_string(),
                },
                "action": { "refId": reflink_id.to_string(), "refType": "entryPoint" },
                "occurredAt": iso(now_ms()),
            });
            if let Some(sent) = chatbotx_jobs::events::emit(buses, "flow:ref", event)
                && let Err(err) = sent.await
            {
                tracing::warn!(err = %err, "flow:ref emit failed");
            }
        }
        self.send_flow(flow_id, None, None).await?;
        let ref_name = match config {
            RefConfig::QrCode { name } | RefConfig::Reflink { name } => name.clone(),
            _ => name,
        };
        let event = chatbotx_business::events::trigger::contact_referred(
            is_new_contact,
            &ws.to_string(),
            &conversation.contact_id.to_string(),
            &ref_name,
            &reflink_id.to_string(),
            Some(&contact_inbox.id.to_string()),
        );
        crate::order_payment::bus(self.deps)
            .publish(Event::Trigger(event))
            .await;
        if let Some(field_id) = custom_field_id
            && let Err(err) = chatbotx_business::contact_custom_field::set_values(
                &self.deps.db,
                chatbotx_business::contact_custom_field::SetValuesInput {
                    workspace_id: ws,
                    contact_id: conversation.contact_id,
                    fields: vec![(field_id, ref_name)],
                    ..Default::default()
                },
            )
            .await
        {
            tracing::warn!(err = %err, reflink_id, "Reflink: failed to save the ref name to the custom field");
        }
        Ok(())
    }
}

/// `runRef`.
pub async fn run_ref(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let conversation_id = data
        .get("conversationId")
        .and_then(id_of)
        .ok_or_else(|| fail("Conversation not found"))?;
    let conversation = load_conversation(&deps.db, conversation_id)
        .await?
        .ok_or_else(|| fail("Conversation not found"))?;
    let contact_inbox_id = data
        .get("contactInboxId")
        .and_then(id_of)
        .ok_or_else(|| fail("Contact inbox not found"))?;
    let contact_inbox = load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id)
        .await?
        .ok_or_else(|| fail("Contact inbox not found"))?;
    let Some(config) = str_of(data, "ref").and_then(decode_ref) else {
        return Ok(());
    };
    let run = RefRun {
        deps,
        conversation,
        contact_inbox,
    };
    let start = now_ms();
    let result = run
        .dispatch(config, data["isNewContact"] == json!(true))
        .await;
    let Some(message_id) = str_of(data, "messageId") else {
        return result.map(|_| ());
    };
    let fields = match &result {
        Ok(false) => return Ok(()),
        Ok(true) => json!({
            "hasResponse": true, "responseType": "flow", "routeType": "flow", "result": "success",
            "aiProvider": "none",
            "metadata": { "latency": now_ms() - start, "triggerContext": {
                "triggerSource": "worker", "triggerHandler": "runRef", "triggerType": "contact_ref" } },
        }),
        Err(_) => json!({
            "hasResponse": false, "responseType": "flow", "routeType": "flow", "result": "fallback",
            "aiProvider": "none",
            "metadata": { "latency": now_ms() - start, "fallbackReason": "handler_error_to_fallback",
                "triggerContext": { "triggerSource": "worker", "triggerHandler": "runRef",
                    "triggerType": "contact_ref_failed" } },
        }),
    };
    crate::events::emit_bot_received(
        deps.events.as_ref(),
        run.conversation.workspace_id,
        run.conversation.id,
        &run.contact_inbox,
        message_id,
        fields,
    )
    .await;
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatbotx_business::referral::encode_ref;

    #[test]
    fn decode_ref_reads_what_encode_ref_writes() {
        for config in [
            RefConfig::Flow {
                flow_id: 42,
                node_id: Some("1234567".into()),
            },
            RefConfig::Flow {
                flow_id: 7,
                node_id: None,
            },
            RefConfig::Draft { flow_id: 9 },
            RefConfig::MinigameShare {
                minigame_id: 3,
                referrer_contact_id: 4,
            },
            RefConfig::Leaderboard {
                leaderboard_id: 5,
                contact_id: 6,
            },
            RefConfig::Poll { poll_id: 8 },
            RefConfig::QrCode {
                name: "spring".into(),
            },
            RefConfig::Reflink {
                name: "promo".into(),
            },
        ] {
            assert_eq!(decode_ref(&encode_ref(&config)), Some(config));
        }
        assert_eq!(decode_ref("mg__x"), None);
        assert_eq!(decode_ref("pl_"), None);
    }
}
