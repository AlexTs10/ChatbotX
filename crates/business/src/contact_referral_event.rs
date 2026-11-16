//! `contact-referral-event/contact-history.ts` (`listContactAdTouches`).

use chatbotx_db::Db;
use chatbotx_db::repo::contact_referral_event::{ContactAdTouch, ContactAdTouchCursor};

use crate::contact::service::find_or_fail;
use crate::{Result, tx};

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
    tx::read(db, move |conn| {
        find_or_fail(conn, workspace_id, contact_id)?;
        let mut data = chatbotx_db::repo::contact_referral_event::list_ad_touches_by_contact(
            conn,
            workspace_id,
            contact_id,
            limit,
            cursor,
        )?;
        let has_more = i64::try_from(data.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            data.truncate(usize::try_from(limit).unwrap_or(0));
        }
        let next_cursor = if has_more {
            data.last().map(|last| ContactAdTouchCursor {
                occurred_at: last.occurred_at,
                id: last.id,
            })
        } else {
            None
        };
        Ok(AdTouchPage { data, next_cursor })
    })
    .await
}
