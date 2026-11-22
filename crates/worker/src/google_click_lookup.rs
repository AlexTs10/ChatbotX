//! `integration/handlers/google-click-lookup.ts` + `google-ads/click-lookup.ts`:
//! the Google click of a contact on a `profileLookup` channel (Zalo
//! `dynamic_param`) is read through the profile API and saved with its ad
//! touch. Two markers per contact inbox keep the call rare: `attempt` (5 min,
//! one lookup at a time, the backoff after a failure) and `done` (24 h, set
//! only after the click was saved). Never fails the message.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use chatbotx_core::utils::google_click::{
    GoogleClickCaptureSource, resolve_google_click_capture_source, without_repeated_google_click,
};
use chatbotx_db::repo::models::ContactInbox;
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::params;

use crate::Deps;

const DONE_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const ATTEMPT_TTL_MS: i64 = 5 * 60 * 1000;
/// `GOOGLE_CLICK_LOOKUP_TIMEOUT_MS`: the receive job must not wait longer.
const TIMEOUT: Duration = Duration::from_millis(5000);

/// Expiry times (ms) by contact inbox id.
#[derive(Default)]
struct Markers {
    done: HashMap<i64, i64>,
    attempt: HashMap<i64, i64>,
}

// ponytail: in-process markers (one binary) instead of the Redis keys, same
// TTLs; a restart forgets them (one extra lookup). Expired entries are swept on
// each claim, so the map holds only the contacts that wrote in the last 24 h.
static MARKERS: LazyLock<Mutex<Markers>> = LazyLock::new(Mutex::default);

/// `claimGoogleClickLookup`: `true` = run now (the attempt is held 5 min).
fn claim(contact_inbox_id: i64, now: i64) -> bool {
    let mut markers = MARKERS.lock().unwrap_or_else(|e| e.into_inner());
    markers.done.retain(|_, until| *until > now);
    markers.attempt.retain(|_, until| *until > now);
    if markers.done.contains_key(&contact_inbox_id) {
        return false;
    }
    markers
        .attempt
        .insert(contact_inbox_id, now + ATTEMPT_TTL_MS)
        .is_none()
}

/// `markGoogleClickLookupDone`: only after the click (if any) was saved.
fn mark_done(contact_inbox_id: i64, now: i64) {
    let mut markers = MARKERS.lock().unwrap_or_else(|e| e.into_inner());
    markers.done.insert(contact_inbox_id, now + DONE_TTL_MS);
}

/// `lookupAndPersistGoogleClick` for a real inbound message (the caller checks
/// `isInboundConversationMessage`). For a contact created by this message it is
/// `persistNewContactGoogleClick` too: the profile is read without the Google
/// Ads gate (the original reads it at creation) and a click makes the contact
/// inbox an `ads` contact (`resolveContactSource` ran before the lookup).
pub(crate) async fn lookup_and_persist(
    deps: &Deps,
    workspace_id: i64,
    contact_inbox: &ContactInbox,
    is_new_contact: bool,
) {
    if resolve_google_click_capture_source(&contact_inbox.channel)
        != Some(GoogleClickCaptureSource::ProfileLookup)
    {
        return;
    }
    // The claim comes first: it bounds the database read below.
    if !claim(contact_inbox.id, now_ms()) {
        return;
    }
    if let Err(reason) = run(deps, workspace_id, contact_inbox, is_new_contact).await {
        tracing::warn!(
            %reason,
            contact_inbox_id = contact_inbox.id,
            channel = %contact_inbox.channel,
            "lookupAndPersistGoogleClick: lookup failed, will retry on a later message"
        );
    }
}

async fn run(
    deps: &Deps,
    workspace_id: i64,
    contact_inbox: &ContactInbox,
    is_new_contact: bool,
) -> Result<(), String> {
    // `integrationGoogleAdsService.hasIntegration`.
    let connected: bool = deps
        .db
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT EXISTS (SELECT 1 FROM "IntegrationGoogleAds" WHERE "workspaceId" = ?1)"#,
                [workspace_id],
                |r| r.get(0),
            )?)
        })
        .await
        .map_err(|e| e.to_string())?;
    if !connected && !is_new_contact {
        return Ok(());
    }
    let (_, ctx) = deps
        .channels
        .resolve(&deps.db, None, workspace_id, contact_inbox)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let token = ctx.auth["tokens"]["accessToken"]
        .as_str()
        .unwrap_or_default();
    let profile = crate::channels::zalo::Zalo::new()
        .get_user_profile(
            token,
            workspace_id,
            &contact_inbox.source_id,
            false,
            Some(TIMEOUT),
        )
        .await
        .map_err(|e| e.message)?;
    // Only a new click moves `googleClickReceivedAt` forward.
    let stored = contact_inbox.referral.as_ref().map(|r| &r.0);
    if let Some(referral) = without_repeated_google_click(stored, profile.referral.as_ref()) {
        let referral = serde_json::to_value(referral).map_err(|e| e.to_string())?;
        if !persist(deps, workspace_id, contact_inbox, referral, is_new_contact).await {
            return Ok(());
        }
    }
    if connected {
        mark_done(contact_inbox.id, now_ms());
    }
    Ok(())
}

/// `persistLookedUpReferral`: the referral and its ad touch in one commit.
/// The database error text may hold the click id, so it is never logged.
async fn persist(
    deps: &Deps,
    workspace_id: i64,
    contact_inbox: &ContactInbox,
    referral: serde_json::Value,
    is_new_contact: bool,
) -> bool {
    let ads =
        is_new_contact && chatbotx_core::utils::google_click::has_google_click(Some(&referral));
    let (id, contact_id, channel) = (
        contact_inbox.id,
        contact_inbox.contact_id,
        contact_inbox.channel.clone(),
    );
    let saved = deps
        .db
        .write(move |tx| {
            let now = now_ms();
            tx.execute(
                r#"UPDATE "ContactInbox" SET
                     "referral" = json_patch(COALESCE("referral", '{}'), ?1), "updatedAt" = ?2,
                     "source" = CASE WHEN ?5 THEN 'ads' ELSE "source" END
                   WHERE "id" = ?3 AND "contactId" = ?4"#,
                params![referral.to_string(), now, id, contact_id, ads],
            )?;
            // The click id keys the touch; a profile lookup has no carrying event.
            chatbotx_business::contact_referral_event::record_ad_touches_contained(
                tx,
                chatbotx_business::contact_referral_event::TouchOwner {
                    workspace_id,
                    contact_id,
                    contact_inbox_id: id,
                    channel: &channel,
                },
                &referral,
                chatbotx_business::contact_referral_event::ReferralOccurrence {
                    occurred_at: now,
                    event_key: None,
                },
            );
            Ok(())
        })
        .await;
    if saved.is_err() {
        tracing::warn!(
            contact_inbox_id = id,
            channel = %contact_inbox.channel,
            "lookupAndPersistGoogleClick: could not save the click, will retry on a later message"
        );
    }
    saved.is_ok()
}

#[cfg(test)]
mod tests {
    //! Port of `packages/business/__tests__/google-ads-click-lookup.test.ts`
    //! (`claimGoogleClickLookup` / `markGoogleClickLookupDone`; the Redis
    //! failure cases have no equivalent: the markers are in-process).
    use super::*;

    fn attempt_of(id: i64) -> Option<i64> {
        MARKERS.lock().unwrap().attempt.get(&id).copied()
    }

    /// Port of: "grants the claim and holds the attempt for the backoff window",
    /// "denies a concurrent claim while another attempt holds the marker",
    /// "denies the claim, without taking an attempt, once the session lookup
    /// is done" and "records the session lookup for 24 hours". One test: the
    /// markers are one process-wide map and each claim sweeps it.
    #[test]
    fn claims_and_done_markers() {
        let (id, t0) = (-91_001, 1_000_000);
        assert!(claim(id, t0));
        assert_eq!(attempt_of(id), Some(t0 + ATTEMPT_TTL_MS));
        assert!(!claim(id, t0 + 1));
        assert!(claim(id, t0 + ATTEMPT_TTL_MS + 1));

        let (done, t1) = (-91_002, t0 + ATTEMPT_TTL_MS + 2);
        mark_done(done, t1);
        assert!(!claim(done, t1 + 1));
        assert_eq!(attempt_of(done), None);
        assert!(!claim(done, t1 + DONE_TTL_MS - 1));
        assert!(claim(done, t1 + DONE_TTL_MS + 1));
    }

    /// `persistNewContactGoogleClick`: "makes a new contact with a looked-up
    /// click an ads contact"; an existing contact keeps its source.
    #[tokio::test]
    async fn a_looked_up_click_makes_a_new_contact_an_ads_contact() {
        let (deps, _) = crate::contact_scan::tests::setup().await;
        deps.db
            .write(|tx| {
                tx.execute_batch(
                    r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (30, 2), (31, 2);
                       INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                       VALUES (300, 30, 30, 4, 'zalo', 'inboundMessage', 'z1'), (301, 31, 31, 4, 'zalo', 'inboundMessage', 'z2')"#,
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let click = serde_json::json!({ "gclid": "g-1" });
        for (id, contact, new) in [(300, 30, true), (301, 31, false)] {
            let row = crate::load_contact_inbox(&deps.db, id, contact)
                .await
                .unwrap()
                .unwrap();
            assert!(persist(&deps, 2, &row, click.clone(), new).await);
        }
        let sources: Vec<String> = deps
            .db
            .read(|c| {
                let mut stmt = c.prepare(
                    r#"SELECT "source" || ':' || json_extract("referral", '$.gclid') FROM "ContactInbox" WHERE "id" IN (300, 301) ORDER BY "id""#,
                )?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
            .unwrap();
        assert_eq!(sources, vec!["ads:g-1", "inboundMessage:g-1"]);
    }
}
