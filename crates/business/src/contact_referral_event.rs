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

/// `ReferralOccurrence`: when a referral arrived and the id of the carrying
/// event (keys a Meta touch without a click id).
#[derive(Debug, Clone, Copy)]
pub struct ReferralOccurrence<'a> {
    pub occurred_at: i64,
    pub event_key: Option<&'a str>,
}

/// One ad touch (`AdTouchValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct AdTouch {
    pub provider: &'static str,
    pub occurred_at: i64,
    pub ad_id: Option<String>,
    pub campaign_id: Option<String>,
    pub ad_group_id: Option<String>,
    pub click_id_type: Option<String>,
    pub click_id: Option<String>,
    pub r#ref: Option<String>,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub dedup_key: String,
}

fn text(referral: &serde_json::Value, key: &str) -> Option<String> {
    referral
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn non_empty(referral: &serde_json::Value, key: &str) -> Option<String> {
    text(referral, key).filter(|s| !s.is_empty())
}

/// `resolveAdReferral` is non-null: a click id, or an ad id from a paid source.
fn is_paid_ad(referral: &serde_json::Value) -> bool {
    non_empty(referral, "ctwaClid").is_some()
        || (!referral["adId"].is_null() && text(referral, "source").as_deref() == Some("ADS"))
}

/// `toAdTouches`: one Meta and one Google touch at most; none for an organic
/// referral.
pub fn to_ad_touches(referral: &serde_json::Value, occurrence: ReferralOccurrence) -> Vec<AdTouch> {
    let mut out = Vec::new();
    let ctwa = non_empty(referral, "ctwaClid");
    if is_paid_ad(referral) && (ctwa.is_some() || occurrence.event_key.is_some()) {
        let ad_id = non_empty(referral, "adId");
        let dedup_key = match &ctwa {
            Some(clid) => format!("ctwa:{clid}"),
            // `ad:${referral.adId}`: a missing id prints as "undefined".
            None => format!(
                "ad:{}:{}",
                text(referral, "adId").unwrap_or_else(|| "undefined".into()),
                occurrence.event_key.unwrap_or_default()
            ),
        };
        out.push(AdTouch {
            provider: "meta",
            occurred_at: occurrence.occurred_at,
            ad_id,
            campaign_id: None,
            ad_group_id: None,
            click_id_type: ctwa.as_ref().map(|_| "ctwa".to_owned()),
            click_id: ctwa,
            r#ref: text(referral, "ref"),
            source: text(referral, "source"),
            source_url: text(referral, "sourceUrl"),
            metadata: Some(serde_json::json!({
                "adTitle": referral.get("adTitle").cloned().unwrap_or_default(),
                "adBody": referral.get("adBody").cloned().unwrap_or_default(),
                "sourcePlatform": referral.get("sourcePlatform").cloned().unwrap_or_default(),
            })),
            dedup_key,
        });
    }
    if let Some((kind, click_id)) =
        chatbotx_core::utils::google_click::read_google_click(Some(referral))
    {
        let occurred_at = non_empty(referral, "googleClickReceivedAt")
            .and_then(|at| chatbotx_core::js::parse_date(&at))
            .unwrap_or(occurrence.occurred_at);
        out.push(AdTouch {
            provider: "google",
            occurred_at,
            ad_id: non_empty(referral, "googleAdId"),
            campaign_id: text(referral, "googleCampaignId"),
            ad_group_id: text(referral, "googleAdGroupId"),
            click_id_type: Some(kind.as_str().to_owned()),
            dedup_key: format!("{}:{click_id}", kind.as_str()),
            click_id: Some(click_id),
            r#ref: None,
            source: None,
            source_url: None,
            metadata: None,
        });
    }
    out
}

/// The contact inbox an ad touch belongs to.
#[derive(Debug, Clone, Copy)]
pub struct TouchOwner<'a> {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub channel: &'a str,
}

/// `recordAdTouches`: idempotent per touch; each touch with a numeric ad id
/// also records the contact's `adClicked` activity.
pub fn record_ad_touches(
    conn: &chatbotx_db::rusqlite::Connection,
    owner: TouchOwner,
    referral: &serde_json::Value,
    occurrence: ReferralOccurrence,
) -> chatbotx_db::repo::Result<()> {
    use chatbotx_db::repo::contact_activity::{ContactActivityRecord, upsert_activities};
    use chatbotx_db::repo::partials::contact_activity::positive_bigint;
    use chatbotx_db::rusqlite::types::Value as Sql;
    let touches = to_ad_touches(referral, occurrence);
    let opt = |v: &Option<String>| v.clone().map_or(Sql::Null, Sql::Text);
    for t in &touches {
        let values: chatbotx_db::repo::patch::Patch = vec![
            ("workspaceId", Sql::Integer(owner.workspace_id)),
            ("contactId", Sql::Integer(owner.contact_id)),
            ("contactInboxId", Sql::Integer(owner.contact_inbox_id)),
            ("channel", Sql::Text(owner.channel.to_owned())),
            ("type", Sql::Text("ad".into())),
            ("provider", Sql::Text(t.provider.into())),
            ("occurredAt", Sql::Integer(t.occurred_at)),
            ("adId", opt(&t.ad_id)),
            ("campaignId", opt(&t.campaign_id)),
            ("adGroupId", opt(&t.ad_group_id)),
            ("clickIdType", opt(&t.click_id_type)),
            ("clickId", opt(&t.click_id)),
            ("ref", opt(&t.r#ref)),
            ("source", opt(&t.source)),
            ("sourceUrl", opt(&t.source_url)),
            (
                "metadata",
                t.metadata
                    .as_ref()
                    .map_or(Sql::Null, |m| Sql::Text(m.to_string())),
            ),
            ("dedupKey", Sql::Text(t.dedup_key.clone())),
        ];
        chatbotx_db::repo::contact_referral_event::insert_ignore_duplicate(conn, &values)?;
    }
    let records: Vec<ContactActivityRecord> = touches
        .iter()
        .filter_map(|t| {
            let reference_id = positive_bigint(t.ad_id.as_deref()?)?;
            Some(ContactActivityRecord {
                activity_type: "adClicked".into(),
                contact_id: owner.contact_id,
                occurred_at: t.occurred_at,
                parent_reference_id: 0,
                reference_id,
                reference_type: "ad".into(),
            })
        })
        .collect();
    upsert_activities(conn, owner.workspace_id, &records)?;
    Ok(())
}

/// `recordAdTouchesContained`: a failure is rolled back to a savepoint and
/// logged, so the carrying message is never failed by the touch log.
pub fn record_ad_touches_contained(
    conn: &chatbotx_db::rusqlite::Connection,
    owner: TouchOwner,
    referral: &serde_json::Value,
    occurrence: ReferralOccurrence,
) {
    let result = conn
        .execute_batch("SAVEPOINT ad_touch")
        .map_err(chatbotx_db::repo::Error::from)
        .and_then(|()| record_ad_touches(conn, owner, referral, occurrence));
    let end = if result.is_ok() {
        "RELEASE ad_touch"
    } else {
        "ROLLBACK TO ad_touch; RELEASE ad_touch"
    };
    if let Err(error) = result {
        tracing::error!(
            ?error,
            workspace_id = owner.workspace_id,
            contact_inbox_id = owner.contact_inbox_id,
            "updateTracking: could not record the ad touch"
        );
    }
    let _ = conn.execute_batch(end);
}

#[cfg(test)]
mod ad_touch_tests {
    use super::*;
    use serde_json::json;

    const AT: ReferralOccurrence = ReferralOccurrence {
        occurred_at: 5,
        event_key: Some("m1"),
    };

    #[test]
    fn meta_and_google_touches() {
        let ctwa = to_ad_touches(&json!({"ctwaClid": "c1", "adId": "9", "source": "ADS"}), AT);
        assert_eq!(ctwa.len(), 1);
        assert_eq!(ctwa[0].dedup_key, "ctwa:c1");
        let ad = to_ad_touches(&json!({"adId": "9", "source": "ADS"}), AT);
        assert_eq!(ad[0].dedup_key, "ad:9:m1");
        assert!(
            to_ad_touches(
                &json!({"adId": "9", "source": "ADS"}),
                ReferralOccurrence {
                    occurred_at: 5,
                    event_key: None
                }
            )
            .is_empty()
        );
        assert!(to_ad_touches(&json!({"ref": "x", "source": "SHORTLINK"}), AT).is_empty());
        let both = to_ad_touches(&json!({"ctwaClid": "c1", "gclid": "g1"}), AT);
        assert_eq!(both[1].dedup_key, "gclid:g1");
        assert_eq!(both[1].occurred_at, 5);
    }
}
