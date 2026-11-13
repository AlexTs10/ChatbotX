//! `contact/create-with-inbox.ts` `createContactWithInbox`.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Contact, ContactInbox};

use crate::Result;

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

/// `createContactWithInbox`: the contact, its contact inbox and an open
/// conversation in one transaction.
pub async fn create_with_inbox(
    db: &Db,
    workspace_id: i64,
    input: CreateContactInput,
    options: CreateOptions,
) -> Result<CreatedContact> {
    let _ = (db, workspace_id, input, options);
    todo!("P3.6-D2")
}
