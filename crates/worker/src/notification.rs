//! The `notification` queue (`notification/worker.ts`): Expo push for a new
//! message or an assigned conversation.
//!
//! ponytail: `notifyNewOrder` waits for the e-commerce order domain (no Rust
//! producer yet); the job is logged and skipped.

use std::sync::Mutex;

use chatbotx_db::rusqlite::OptionalExtension as _;
use chatbotx_jobs::{BoxFuture, JobCtx, JobError};
use serde_json::{Value, json};

use crate::{Deps, db_fail, fail, is_blocked_workspace, str_of};

/// The Expo send API (`expo.sendPushNotificationsAsync` of one chunk):
/// returns one ticket per message.
pub trait PushSender: Send + Sync {
    fn send<'a>(&'a self, messages: &'a [Value]) -> BoxFuture<'a, Result<Vec<Value>, String>>;
    /// `chunkPushNotifications` size.
    fn chunk_limit(&self) -> usize {
        CHUNK_LIMIT
    }
}

/// `new Expo({ accessToken })` over HTTPS.
pub struct ExpoPush {
    client: reqwest::Client,
    access_token: Option<String>,
}

impl ExpoPush {
    /// `getExpoClient()`: `None` when `EXPO_PUSH_ENABLED` is false.
    pub fn from_env() -> Option<Self> {
        if !push_enabled(std::env::var("EXPO_PUSH_ENABLED").ok().as_deref()) {
            tracing::info!("EXPO_PUSH_ENABLED is false — push notifications disabled");
            return None;
        }
        Some(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .ok()?,
            access_token: std::env::var("EXPO_ACCESS_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
        })
    }
}

impl PushSender for ExpoPush {
    fn send<'a>(&'a self, messages: &'a [Value]) -> BoxFuture<'a, Result<Vec<Value>, String>> {
        Box::pin(async move {
            let mut request = self
                .client
                .post("https://exp.host/--/api/v2/push/send")
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .body(Value::from(messages.to_vec()).to_string());
            if let Some(token) = &self.access_token {
                request = request.bearer_auth(token);
            }
            let response = request.send().await.map_err(|e| e.to_string())?;
            let status = response.status();
            let body: Value =
                serde_json::from_str(&response.text().await.map_err(|e| e.to_string())?)
                    .map_err(|e| format!("Expo push response is not JSON ({status}): {e}"))?;
            if let Some(errors) = body.get("errors") {
                return Err(format!("Expo push failed: {errors}"));
            }
            match body.get("data").and_then(Value::as_array) {
                Some(tickets) if tickets.len() == messages.len() => Ok(tickets.clone()),
                _ => Err(format!(
                    "Expo push returned an unexpected response ({status})"
                )),
            }
        })
    }
}

/// `EXPO_PUSH_ENABLED` (`z.stringbool().default(true)`).
pub fn push_enabled(value: Option<&str>) -> bool {
    value.is_none_or(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off" | "n" | "disabled"
        )
    })
}

/// Records each chunk; the queued tickets answer in order (default: ok).
#[derive(Default)]
pub struct RecordingPush {
    pub sent: Mutex<Vec<Vec<Value>>>,
    pub tickets: Mutex<Vec<Result<Vec<Value>, String>>>,
    /// The chunk size (0 = Expo's 100).
    pub chunk: usize,
}

impl PushSender for RecordingPush {
    fn send<'a>(&'a self, messages: &'a [Value]) -> BoxFuture<'a, Result<Vec<Value>, String>> {
        self.sent.lock().unwrap().push(messages.to_vec());
        let next = {
            let mut queued = self.tickets.lock().unwrap();
            (!queued.is_empty()).then(|| queued.remove(0))
        };
        let result = next
            .unwrap_or_else(|| Ok(messages.iter().map(|_| json!({ "status": "ok" })).collect()));
        Box::pin(async move { result })
    }

    fn chunk_limit(&self) -> usize {
        if self.chunk == 0 {
            CHUNK_LIMIT
        } else {
            self.chunk
        }
    }
}

/// `PUSH_NOTIFICATION_CHUNK_LIMIT`.
const CHUNK_LIMIT: usize = 100;

/// `Expo.isExpoPushToken`.
pub fn is_expo_push_token(token: &str) -> bool {
    let bracketed = (token.starts_with("ExponentPushToken[")
        || token.starts_with("ExpoPushToken["))
        && token.ends_with(']');
    let uuid_like = token.len() == 36
        && token.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_alphanumeric()
            }
        });
    bracketed || uuid_like
}

struct Strings {
    new_message: &'static str,
    shared_location: &'static str,
    sent_link: &'static str,
    sent_attachment: &'static str,
    sent_attachments: &'static str,
    assigned_conversation: &'static str,
    #[allow(dead_code)] // notifyNewOrder
    new_order: &'static str,
}

/// `notification/lib/strings.ts`.
const STRINGS: &[(&str, Strings)] = &[
    (
        "en",
        Strings {
            new_message: "New message",
            shared_location: "Shared a location",
            sent_link: "Sent a link",
            sent_attachment: "Sent an attachment",
            sent_attachments: "Sent {} attachments",
            assigned_conversation: "You were assigned a conversation",
            new_order: "New order #{}",
        },
    ),
    (
        "vi",
        Strings {
            new_message: "Tin nhắn mới",
            shared_location: "Đã chia sẻ vị trí",
            sent_link: "Đã gửi một liên kết",
            sent_attachment: "Đã gửi một tệp đính kèm",
            sent_attachments: "Đã gửi {} tệp đính kèm",
            assigned_conversation: "Bạn đã được gán một cuộc trò chuyện",
            new_order: "Đơn hàng mới #{}",
        },
    ),
    (
        "ar",
        Strings {
            new_message: "رسالة جديدة",
            shared_location: "شارك موقعًا",
            sent_link: "أرسل رابطًا",
            sent_attachment: "أرسل مرفقًا",
            sent_attachments: "أرسل {} مرفقات",
            assigned_conversation: "تم تعيين محادثة لك",
            new_order: "طلب جديد #{}",
        },
    ),
    (
        "da",
        Strings {
            new_message: "Ny besked",
            shared_location: "Delte en placering",
            sent_link: "Sendte et link",
            sent_attachment: "Sendte en vedhæftet fil",
            sent_attachments: "Sendte {} vedhæftede filer",
            assigned_conversation: "Du blev tildelt en samtale",
            new_order: "Ny ordre #{}",
        },
    ),
    (
        "de",
        Strings {
            new_message: "Neue Nachricht",
            shared_location: "Hat einen Standort geteilt",
            sent_link: "Hat einen Link gesendet",
            sent_attachment: "Hat einen Anhang gesendet",
            sent_attachments: "Hat {} Anhänge gesendet",
            assigned_conversation: "Dir wurde eine Unterhaltung zugewiesen",
            new_order: "Neue Bestellung #{}",
        },
    ),
    (
        "es",
        Strings {
            new_message: "Nuevo mensaje",
            shared_location: "Compartió una ubicación",
            sent_link: "Envió un enlace",
            sent_attachment: "Envió un archivo adjunto",
            sent_attachments: "Envió {} archivos adjuntos",
            assigned_conversation: "Se te asignó una conversación",
            new_order: "Nuevo pedido #{}",
        },
    ),
    (
        "fi",
        Strings {
            new_message: "Uusi viesti",
            shared_location: "Jakoi sijainnin",
            sent_link: "Lähetti linkin",
            sent_attachment: "Lähetti liitteen",
            sent_attachments: "Lähetti {} liitettä",
            assigned_conversation: "Sinulle osoitettiin keskustelu",
            new_order: "Uusi tilaus #{}",
        },
    ),
    (
        "fr",
        Strings {
            new_message: "Nouveau message",
            shared_location: "A partagé une position",
            sent_link: "A envoyé un lien",
            sent_attachment: "A envoyé une pièce jointe",
            sent_attachments: "A envoyé {} pièces jointes",
            assigned_conversation: "Une conversation vous a été assignée",
            new_order: "Nouvelle commande #{}",
        },
    ),
    (
        "he",
        Strings {
            new_message: "הודעה חדשה",
            shared_location: "שיתף מיקום",
            sent_link: "שלח קישור",
            sent_attachment: "שלח קובץ מצורף",
            sent_attachments: "שלח {} קבצים מצורפים",
            assigned_conversation: "הוקצתה לך שיחה",
            new_order: "הזמנה חדשה #{}",
        },
    ),
    (
        "id",
        Strings {
            new_message: "Pesan baru",
            shared_location: "Membagikan lokasi",
            sent_link: "Mengirim tautan",
            sent_attachment: "Mengirim lampiran",
            sent_attachments: "Mengirim {} lampiran",
            assigned_conversation: "Anda ditugaskan sebuah percakapan",
            new_order: "Pesanan baru #{}",
        },
    ),
    (
        "it",
        Strings {
            new_message: "Nuovo messaggio",
            shared_location: "Ha condiviso una posizione",
            sent_link: "Ha inviato un link",
            sent_attachment: "Ha inviato un allegato",
            sent_attachments: "Ha inviato {} allegati",
            assigned_conversation: "Ti è stata assegnata una conversazione",
            new_order: "Nuovo ordine #{}",
        },
    ),
    (
        "ja",
        Strings {
            new_message: "新しいメッセージ",
            shared_location: "位置情報を共有しました",
            sent_link: "リンクを送信しました",
            sent_attachment: "添付ファイルを送信しました",
            sent_attachments: "{}件の添付ファイルを送信しました",
            assigned_conversation: "会話が割り当てられました",
            new_order: "新しい注文 #{}",
        },
    ),
    (
        "nl",
        Strings {
            new_message: "Nieuw bericht",
            shared_location: "Heeft een locatie gedeeld",
            sent_link: "Heeft een link verzonden",
            sent_attachment: "Heeft een bijlage verzonden",
            sent_attachments: "Heeft {} bijlagen verzonden",
            assigned_conversation: "Er is een gesprek aan je toegewezen",
            new_order: "Nieuwe bestelling #{}",
        },
    ),
    (
        "pt-BR",
        Strings {
            new_message: "Nova mensagem",
            shared_location: "Compartilhou uma localização",
            sent_link: "Enviou um link",
            sent_attachment: "Enviou um anexo",
            sent_attachments: "Enviou {} anexos",
            assigned_conversation: "Uma conversa foi atribuída a você",
            new_order: "Novo pedido #{}",
        },
    ),
    (
        "pt-PT",
        Strings {
            new_message: "Nova mensagem",
            shared_location: "Partilhou uma localização",
            sent_link: "Enviou uma ligação",
            sent_attachment: "Enviou um anexo",
            sent_attachments: "Enviou {} anexos",
            assigned_conversation: "Foi-lhe atribuída uma conversa",
            new_order: "Nova encomenda #{}",
        },
    ),
    (
        "ro",
        Strings {
            new_message: "Mesaj nou",
            shared_location: "A distribuit o locație",
            sent_link: "A trimis un link",
            sent_attachment: "A trimis un atașament",
            sent_attachments: "A trimis {} atașamente",
            assigned_conversation: "Ți-a fost atribuită o conversație",
            new_order: "Comandă nouă #{}",
        },
    ),
    (
        "sv",
        Strings {
            new_message: "Nytt meddelande",
            shared_location: "Delade en plats",
            sent_link: "Skickade en länk",
            sent_attachment: "Skickade en bilaga",
            sent_attachments: "Skickade {} bilagor",
            assigned_conversation: "Du tilldelades en konversation",
            new_order: "Ny beställning #{}",
        },
    ),
    (
        "tr",
        Strings {
            new_message: "Yeni mesaj",
            shared_location: "Bir konum paylaştı",
            sent_link: "Bir bağlantı gönderdi",
            sent_attachment: "Bir ek gönderdi",
            sent_attachments: "{} ek gönderdi",
            assigned_conversation: "Size bir görüşme atandı",
            new_order: "Yeni sipariş #{}",
        },
    ),
    (
        "zh-CN",
        Strings {
            new_message: "新消息",
            shared_location: "分享了位置",
            sent_link: "发送了一个链接",
            sent_attachment: "发送了一个附件",
            sent_attachments: "发送了 {} 个附件",
            assigned_conversation: "已为您分配一个对话",
            new_order: "新订单 #{}",
        },
    ),
    (
        "zh-TW",
        Strings {
            new_message: "新訊息",
            shared_location: "分享了位置",
            sent_link: "傳送了一個連結",
            sent_attachment: "傳送了一個附件",
            sent_attachments: "傳送了 {} 個附件",
            assigned_conversation: "已為您指派一個對話",
            new_order: "新訂單 #{}",
        },
    ),
];

/// `t(language)`: an unknown language is English.
fn strings(language: Option<&str>) -> &'static Strings {
    STRINGS
        .iter()
        .find(|(locale, _)| Some(*locale) == language)
        .map_or(&STRINGS[0].1, |(_, s)| s)
}

/// `buildNotificationContent`.
pub fn notification_content(
    job: &Value,
    contact_full_name: Option<&str>,
    language: Option<&str>,
) -> (String, String) {
    let s = strings(language);
    let title = contact_full_name.unwrap_or(s.new_message).to_owned();
    if str_of(job, "type") == Some("notifyConversationAssigned") {
        return (title, s.assigned_conversation.to_owned());
    }
    let data = &job["data"];
    let attachments = data
        .get("attachmentCount")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let body = match str_of(data, "messageText").filter(|t| !t.is_empty()) {
        Some(text) => text.to_owned(),
        None => match str_of(data, "contentType") {
            Some("location") => s.shared_location.to_owned(),
            Some("refLink") => s.sent_link.to_owned(),
            _ if attachments == 1.0 => s.sent_attachment.to_owned(),
            _ if attachments > 1.0 => s
                .sent_attachments
                .replace("{}", &chatbotx_core::js::number_to_string(attachments)),
            _ => String::new(),
        },
    };
    (title, body)
}

/// The queue processor (`withBlockedOwnerGuard`, then the job type).
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let workspace = str_of(&job.data["data"], "workspaceId").unwrap_or("");
    if is_blocked_workspace(&deps.db, workspace).await? {
        tracing::info!(
            workspace_id = workspace,
            "Skipping workspace job for frozen workspace"
        );
        return Ok(Value::Null);
    }
    if str_of(&job.data, "type") == Some("notifyNewOrder") {
        tracing::warn!("notifyNewOrder: the order domain is not ported, skipping");
        return Ok(Value::Null);
    }
    send_push_for_notification_job(&deps, &job.data).await?;
    Ok(Value::Null)
}

/// `sendPushForNotificationJob`.
pub async fn send_push_for_notification_job(deps: &Deps, job: &Value) -> Result<(), JobError> {
    let Some(push) = deps.push.clone() else {
        return Ok(());
    };
    let data = &job["data"];
    let id = |k: &str| str_of(data, k).and_then(|v| v.parse::<i64>().ok());
    let (Some(workspace), Some(conversation)) = (id("workspaceId"), id("conversationId")) else {
        return Err(fail("Conversation not found"));
    };
    let assigned = id("assignedUserId");
    let exclude = id("excludeUserId");
    let is_assigned_job = str_of(job, "type") == Some("notifyConversationAssigned");
    let loaded = deps
        .db
        .read(move |c| {
            let Some((contact, assignee)) = c
                .query_row(
                    r#"SELECT "contactId", "assignedUserId" FROM "Conversation" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    [conversation, workspace],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
                )
                .optional()?
            else {
                return Ok(None);
            };
            let mut users: Vec<i64> = if is_assigned_job {
                assigned.into_iter().collect()
            } else if let Some(user) = assignee {
                vec![user]
            } else {
                let mut stmt = c.prepare(r#"SELECT "userId" FROM "WorkspaceMember" WHERE "workspaceId" = ?1"#)?;
                stmt.query_map([workspace], |r| r.get(0))?.collect::<Result<_, _>>()?
            };
            if !is_assigned_job && let Some(exclude) = exclude {
                users.retain(|u| *u != exclude);
            }
            let name: Option<String> = c
                .query_row(r#"SELECT "fullName" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#, [contact, workspace], |r| r.get(0))
                .optional()?
                .flatten();
            let language: Option<String> = c
                .query_row(r#"SELECT "language" FROM "Workspace" WHERE "id" = ?1"#, [workspace], |r| r.get(0))
                .optional()?;
            Ok(Some((users, name, language)))
        })
        .await
        .map_err(db_fail)?;
    let Some((users, name, language)) = loaded else {
        return Err(fail("Conversation not found"));
    };
    let (title, body) = notification_content(job, name.as_deref(), language.as_deref());
    let payload = json!({
        "workspaceId": data["workspaceId"],
        "conversationId": data["conversationId"],
        "messageId": data.get("messageId").cloned().unwrap_or(json!("")),
    });
    deliver_push_to_users(deps, push.as_ref(), users, &title, &body, payload).await
}

async fn delete_tokens(deps: &Deps, tokens: Vec<String>) -> Result<(), JobError> {
    if tokens.is_empty() {
        return Ok(());
    }
    deps.db
        .write(move |tx| {
            for token in &tokens {
                tx.execute(
                    r#"DELETE FROM "UserDeviceToken" WHERE "token" = ?1"#,
                    [token],
                )?;
            }
            Ok(())
        })
        .await
        .map_err(db_fail)
}

/// `deliverPushToUsers`: prunes malformed and unregistered tokens; fails
/// only when every chunk failed.
pub async fn deliver_push_to_users(
    deps: &Deps,
    push: &dyn PushSender,
    users: Vec<i64>,
    title: &str,
    body: &str,
    data: Value,
) -> Result<(), JobError> {
    if users.is_empty() {
        return Ok(());
    }
    let tokens = deps
        .db
        .read(move |c| {
            let ids = Value::from(users).to_string();
            let mut stmt = c.prepare(r#"SELECT "token" FROM "UserDeviceToken" WHERE "userId" IN (SELECT value FROM json_each(?1))"#)?;
            let rows = stmt.query_map([ids], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(db_fail)?;
    let (valid, invalid): (Vec<String>, Vec<String>) =
        tokens.into_iter().partition(|t| is_expo_push_token(t));
    delete_tokens(deps, invalid).await?;
    if valid.is_empty() {
        return Ok(());
    }
    let messages: Vec<Value> = valid
        .iter()
        .map(|to| json!({ "to": to, "title": title, "body": body, "data": data, "sound": "default", "channelId": "default", "priority": "high" }))
        .collect();
    let chunks: Vec<&[Value]> = messages.chunks(push.chunk_limit()).collect();
    let mut stale = Vec::new();
    let mut failed = 0;
    for chunk in &chunks {
        match push.send(chunk).await {
            Ok(tickets) => {
                for (ticket, message) in tickets.iter().zip(chunk.iter()) {
                    if str_of(ticket, "status") == Some("error")
                        && ticket.pointer("/details/error").and_then(Value::as_str)
                            == Some("DeviceNotRegistered")
                        && let Some(to) = str_of(message, "to")
                    {
                        stale.push(to.to_owned());
                    }
                }
            }
            Err(err) => {
                failed += 1;
                tracing::warn!(err = %err, "Expo push chunk failed");
            }
        }
    }
    if failed == chunks.len() {
        return Err(fail(format!(
            "All {} Expo push chunk(s) failed to send",
            chunks.len()
        )));
    }
    if !stale.is_empty() {
        let count = stale.len();
        delete_tokens(deps, stale).await?;
        tracing::info!(count, "pruned stale device push tokens");
    }
    Ok(())
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod tests;
