//! `contact-referral-event/contact-history.ts` (`listContactAdTouches`).

use chatbotx_db::Db;
use chatbotx_db::repo::contact_referral_event::{ContactAdTouch, ContactAdTouchCursor};

use crate::Result;

/// One page of a contact's ad touches, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct AdTouchPage {
    pub data: Vec<ContactAdTouch>,
    /// The last row of the page when another page exists.
    pub next_cursor: Option<ContactAdTouchCursor>,
}

/// `listContactAdTouches`: the contact must be in the workspace
/// (`Contact not found`), then `limit` touches past `cursor`.
pub async fn list_contact_ad_touches(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    limit: i64,
    cursor: Option<ContactAdTouchCursor>,
) -> Result<AdTouchPage> {
    let _ = (db, workspace_id, contact_id, limit, cursor);
    todo!("P3.6-D2 ad referrals stage 2")
}
