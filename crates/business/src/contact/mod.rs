//! `packages/business/src/contact` (P3.6-D2 slice contacts core): create
//! with inbox, the identifier lookup, the public contact read, update of
//! standard and custom fields, delete, block / unblock and upsert.

pub mod avatar;
pub mod create;
pub mod identifier;
pub mod list;
pub mod phone;
pub mod service;
pub mod update_fields;

pub use create::{
    CreateContactInput, CreateOptions, CreatedContact, PhoneUniqueness, create_with_inbox,
};
pub use identifier::{ContactIdentifier, IdentifierPrefix, parse_contact_identifier};
pub use service::{
    DeleteOutcome, UpsertData, UpsertOutcome, block_and_record, delete, delete_and_record,
    find_public_contact_or_fail, list_by_custom_field_value, resolve_id_by_identifier,
    unblock_and_record, upsert_by_identifier,
};
pub use update_fields::{CustomFieldChange, UpdateOutcome, update_fields_and_custom_fields};

/// `CONTACT_ALREADY_ON_INBOX_MESSAGE`.
pub const CONTACT_ALREADY_ON_INBOX_MESSAGE: &str =
    "This contact already exists on the selected inbox";
