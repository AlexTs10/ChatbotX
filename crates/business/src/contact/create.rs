//! `contact/create-with-inbox.ts` `createContactWithInbox`.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Contact, ContactInbox};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};

use super::CONTACT_ALREADY_ON_INBOX_MESSAGE;
use super::phone::normalize_phone_to_e164;
use crate::{BusinessError, Result, tx};

/// `CreateContactInput` (the parsed `createContactRequest`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateContactInput {
    pub phone_number: Option<String>,
    /// WhatsApp BSUID; ignored on every other channel.
    pub source_user_id: Option<String>,
    pub email: String,
    /// The channel user id (`contactId`).
    pub contact_id: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub gender: String,
    pub channel: String,
    pub inbox_id: i64,
}

/// `ContactPhoneUniqueness`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PhoneUniqueness {
    /// A phone number may exist once per workspace (the default).
    #[default]
    Workspace,
    /// Identity is scoped to the inbox.
    Inbox,
}

/// The optional arguments (`source` default `direct`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOptions {
    pub source: String,
    pub phone_uniqueness: PhoneUniqueness,
}

impl Default for CreateOptions {
    fn default() -> CreateOptions {
        CreateOptions {
            source: "direct".to_string(),
            phone_uniqueness: PhoneUniqueness::Workspace,
        }
    }
}

/// The created rows.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedContact {
    pub contact: Contact,
    pub contact_inbox: ContactInbox,
}

pub(crate) fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

pub(crate) fn new_id() -> i64 {
    chatbotx_core::utils::id::create_id()
        .parse()
        .unwrap_or_default()
}

/// `randomString(10)` (remeda): `[A-Za-z0-9]`.
pub(crate) fn random_string(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = vec![0u8; len];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .expect("system random");
    bytes
        .iter()
        .map(|b| CHARS[usize::from(*b) % CHARS.len()] as char)
        .collect()
}

/// `normalizeContactInboxSourceId` + `deriveContactInboxSourceId` +
/// `resolveNewContactInboxSourceId`.
pub(crate) fn new_source_id(
    channel: &str,
    channel_user_id: Option<&str>,
    phone_e164: Option<&str>,
    email: Option<&str>,
) -> String {
    let derived = match channel {
        "whatsapp" => phone_e164
            .filter(|p| !p.is_empty())
            .map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>())
            .filter(|digits| !digits.is_empty()),
        "smtp" => email.filter(|e| !e.is_empty()).map(str::to_string),
        "webchat" | "omnichannel" => None,
        "threads" => channel_user_id.map(str::to_lowercase),
        _ => channel_user_id.map(str::to_string),
    };
    match derived {
        Some(id) => id,
        None if channel == "webchat" => format!("{}{}", random_string(10), new_id()),
        None => String::new(),
    }
}

/// The contact columns of an insert.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewContact {
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub gender: Option<String>,
}

/// The contact inbox columns of an insert.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewContactInbox<'a> {
    pub inbox_id: i64,
    pub channel: &'a str,
    pub source: &'a str,
    pub source_id: &'a str,
    pub source_user_id: Option<&'a str>,
}

/// `contactService.insert` + the contact inbox + `cancelByInboxSource` +
/// the conversation (the `createContactWithoutMac` callback).
pub(crate) fn insert_with_inbox(
    tx: &Connection,
    workspace_id: i64,
    data: &NewContact,
    inbox: &NewContactInbox<'_>,
) -> Result<CreatedContact> {
    let NewContactInbox {
        inbox_id,
        channel,
        source,
        source_id,
        source_user_id,
    } = *inbox;
    let contact_id = new_id();
    tx.execute(
        r#"INSERT INTO "Contact" ("id", "workspaceId", "email", "phoneNumber", "firstName", "lastName", "gender")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
        params![
            contact_id,
            workspace_id,
            data.email,
            data.phone_number,
            data.first_name,
            data.last_name,
            data.gender
        ],
    )
    .map_err(repo_err)?;
    let contact_inbox_id = new_id();
    tx.execute(
        r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "sourceUserId")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)"#,
        params![contact_inbox_id, contact_id, inbox_id, channel, source, source_id, source_user_id],
    )
    .map_err(repo_err)?;
    tx.execute(
        r#"DELETE FROM "MessageCleanup" WHERE "inboxId" = ?1 AND "sourceId" = ?2"#,
        params![inbox_id, source_id],
    )
    .map_err(repo_err)?;
    tx.execute(
        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (?1, ?2, ?3)"#,
        params![new_id(), workspace_id, contact_id],
    )
    .map_err(repo_err)?;
    let contact = load::<Contact>(tx, contact_id)?;
    let contact_inbox = load::<ContactInbox>(tx, contact_inbox_id)?;
    Ok(CreatedContact {
        contact,
        contact_inbox,
    })
}

pub(crate) fn load<M: chatbotx_db::repo::Model>(conn: &Connection, id: i64) -> Result<M> {
    let sql = format!("{} WHERE \"id\" = ?1", M::select_sql());
    conn.query_row(&sql, [id], |row| M::from_row(row))
        .map_err(repo_err)
}

/// `findLatestBySource` (workspace-scoped through the inbox).
fn source_taken(
    tx: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    source_id: &str,
) -> Result<bool> {
    tx.query_row(
        r#"SELECT 1 FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId"
           WHERE ci."inboxId" = ?1 AND ci."sourceId" = ?2 AND i."workspaceId" = ?3 LIMIT 1"#,
        params![inbox_id, source_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(repo_err)
}

/// `contactService.findByPhone`.
pub(crate) fn phone_taken(tx: &Connection, workspace_id: i64, phone: &str) -> Result<bool> {
    tx.query_row(
        r#"SELECT 1 FROM "Contact" WHERE "workspaceId" = ?1 AND "phoneNumber" = ?2 LIMIT 1"#,
        params![workspace_id, phone],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(repo_err)
}

/// `createContactWithInbox`: the contact, its contact inbox and an open
/// conversation in one transaction.
pub async fn create_with_inbox(
    db: &Db,
    workspace_id: i64,
    input: CreateContactInput,
    options: CreateOptions,
) -> Result<CreatedContact> {
    tx::write(db, move |tx| {
        let inbox_channel: String = tx
            .query_row(
                r#"SELECT "channel" FROM "Inbox" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                params![workspace_id, input.inbox_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?
            .ok_or_else(|| BusinessError::not_found("Inbox not found"))?;
        if input.channel != inbox_channel {
            return Err(BusinessError::validation(
                "inboxId",
                "Selected inbox does not match the selected source",
            ));
        }
        let target_country: Option<String> = tx
            .query_row(
                r#"SELECT "targetCountry" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?
            .ok_or_else(|| BusinessError::validation("phoneNumber", "Workspace not found"))?;
        let is_whatsapp = inbox_channel == "whatsapp";
        let source_user_id = if is_whatsapp {
            input
                .source_user_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        } else {
            None
        };
        let has_phone = input.phone_number.as_deref().is_some_and(|p| !p.is_empty());
        let is_bsuid_keyed = source_user_id.is_some() && !has_phone;
        let mut phone = input.phone_number.clone();
        if (is_whatsapp && !is_bsuid_keyed) || has_phone {
            let e164 = normalize_phone_to_e164(
                input.phone_number.as_deref().unwrap_or(""),
                target_country.as_deref(),
            )
            .ok_or_else(|| {
                BusinessError::validation("phoneNumber", "Please include the country code (e.g. +84)")
            })?;
            phone = Some(e164);
        }
        let source_id = match (&source_user_id, is_bsuid_keyed) {
            (Some(bsuid), true) => bsuid.clone(),
            _ => new_source_id(
                &inbox_channel,
                input.contact_id.as_deref(),
                phone.as_deref(),
                Some(&input.email),
            ),
        };
        if let Some(phone) = phone.as_deref().filter(|p| !p.is_empty())
            && options.phone_uniqueness == PhoneUniqueness::Workspace
            && phone_taken(tx, workspace_id, phone)?
        {
            return Err(BusinessError::validation("phoneNumber", "Phone number is exists"));
        }
        if inbox_channel != "webchat" && source_taken(tx, workspace_id, input.inbox_id, &source_id)? {
            let field = if is_whatsapp {
                if is_bsuid_keyed { "sourceUserId" } else { "phoneNumber" }
            } else if inbox_channel == "smtp" {
                "email"
            } else {
                "contactId"
            };
            return Err(BusinessError::validation(field, CONTACT_ALREADY_ON_INBOX_MESSAGE));
        }
        if let Some(bsuid) = &source_user_id {
            let taken = tx
                .query_row(
                    r#"SELECT 1 FROM "ContactInbox" WHERE "inboxId" = ?1 AND "sourceUserId" = ?2 LIMIT 1"#,
                    params![input.inbox_id, bsuid],
                    |_| Ok(()),
                )
                .optional()
                .map_err(repo_err)?
                .is_some();
            if taken {
                return Err(BusinessError::validation(
                    "sourceUserId",
                    CONTACT_ALREADY_ON_INBOX_MESSAGE,
                ));
            }
        }
        let data = NewContact {
            email: Some(input.email.clone()),
            phone_number: phone,
            first_name: input.first_name.clone(),
            last_name: input.last_name.clone(),
            gender: Some(input.gender.clone()),
        };
        insert_with_inbox(
            tx,
            workspace_id,
            &data,
            &NewContactInbox {
                inbox_id: input.inbox_id,
                channel: &inbox_channel,
                source: &options.source,
                source_id: &source_id,
                source_user_id: source_user_id.as_deref(),
            },
        )
    })
    .await
}
