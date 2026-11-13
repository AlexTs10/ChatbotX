//! `contact/utils.ts` `parseContactIdentifier`.

use crate::{BusinessError, Result};

/// The identifier prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierPrefix {
    Id,
    Email,
    Phone,
}

/// A parsed `id:` / `email:` / `phone:` identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactIdentifier {
    pub prefix: IdentifierPrefix,
    pub value: String,
    /// `where.id` (an `id:` identifier).
    pub id: Option<i64>,
}

impl ContactIdentifier {
    /// `where.email`.
    pub fn email(&self) -> Option<&str> {
        (self.prefix == IdentifierPrefix::Email).then_some(self.value.as_str())
    }

    /// `where.phoneNumber`.
    pub fn phone(&self) -> Option<&str> {
        (self.prefix == IdentifierPrefix::Phone).then_some(self.value.as_str())
    }
}

/// `parseContactIdentifier`: `404 notFound` for a bad format or prefix and
/// for a non-numeric `id:`.
pub fn parse_contact_identifier(identifier: &str) -> Result<ContactIdentifier> {
    let Some((prefix, value)) = identifier.split_once(':') else {
        return Err(BusinessError::not_found("Invalid identifier format"));
    };
    if value.is_empty() {
        return Err(BusinessError::not_found("Invalid identifier format"));
    }
    let prefix = match prefix {
        "id" => IdentifierPrefix::Id,
        "email" => IdentifierPrefix::Email,
        "phone" => IdentifierPrefix::Phone,
        _ => {
            return Err(BusinessError::not_found(
                "Invalid identifier format. Use id:, email:, or phone: prefix",
            ));
        }
    };
    let id = if prefix == IdentifierPrefix::Id {
        // `/^\d+$/`; a value beyond i64 cannot be a stored id.
        if !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(BusinessError::not_found("Contact not found"));
        }
        Some(
            value
                .parse::<i64>()
                .map_err(|_| BusinessError::not_found("Contact not found"))?,
        )
    } else {
        None
    };
    Ok(ContactIdentifier {
        prefix,
        value: value.to_string(),
        id,
    })
}
