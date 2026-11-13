//! `contact/utils.ts` `parseContactIdentifier`.

use crate::Result;

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

/// `parseContactIdentifier`: `404 notFound` for a bad format or prefix and
/// for a non-numeric `id:`.
pub fn parse_contact_identifier(identifier: &str) -> Result<ContactIdentifier> {
    let _ = identifier;
    todo!("P3.6-D2")
}
