//! Magic link and ref link analytics (Q.D11 slice 8b): the seed and the
//! expectations of the parity group p1-03 `analytics-links.test.ts`, for
//! both link tables. K1 clicks twice on 08-01 (and once before the range),
//! K2 late on 08-01 (08-02 in Ho Chi Minh City), K3 on 08-03, K4 only
//! before the range, and one click of a contact inbox that no longer exists.

mod common;

use chatbotx_business::analytics::links::{self, Link, LinkContact, LinkContacts};
use chatbotx_db::rusqlite::{Connection, params};
use common::{OTHER_WS, WS, setup, two_workspaces};

const LINK: i64 = 70;
const GONE_INBOX: i64 = 9_999;

fn ms(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).expect("iso")
}

fn range() -> (i64, i64) {
    (ms("2025-08-01T00:00:00Z"), ms("2025-08-03T23:59:59Z"))
}

async fn seeded(link: Link) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        let table = match link {
            Link::Magic => {
                tx.execute(
                    r#"INSERT INTO "MagicLink" ("id", "workspaceId", "name", "url") VALUES (?1, ?2, 'promo', 'https://shop.parity.test/sale')"#,
                    params![LINK, WS],
                )?;
                "MagicLinkStat"
            }
            Link::Ref => {
                common::flow(tx, 5, WS)?;
                common::reflink_row(tx, LINK, WS, "refLink", "ref", 5)?;
                "RefLinkStat"
            }
        };
        common::inbox(tx, 11, WS, "messenger")?;
        for k in 1..=4 {
            common::contact(tx, 100 + k, WS)?;
            common::contact_inbox(tx, 200 + k, 100 + k, 11, "messenger", &format!("psid-{k}"))?;
            common::conversation(tx, 300 + k, WS, 100 + k)?;
        }
        let click = |tx: &Connection, k: i64, at: &str| {
            let (contact, inbox) = if k == 0 { (8_888, GONE_INBOX) } else { (100 + k, 200 + k) };
            tx.execute(
                &format!(
                    r#"INSERT INTO "{table}" ("workspaceId", "linkId", "contactId", "contactInboxId", "occurredAt") VALUES (?1, ?2, ?3, ?4, ?5)"#
                ),
                params![WS, LINK, contact, inbox, ms(at)],
            )
        };
        click(tx, 1, "2025-07-31T23:59:59Z")?;
        click(tx, 1, "2025-08-01T10:00:00Z")?;
        click(tx, 1, "2025-08-01T12:00:00Z")?;
        click(tx, 2, "2025-08-01T23:30:00Z")?;
        click(tx, 0, "2025-08-02T00:00:00Z")?;
        click(tx, 3, "2025-08-03T00:30:00Z")?;
        click(tx, 4, "2025-07-20T00:00:00Z")?;
        Ok(())
    })
    .await;
    db
}

async fn read<R: Send + 'static>(
    db: &chatbotx_db::Db,
    f: impl FnOnce(&Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> R {
    db.read(move |c| f(c).map_err(|e| chatbotx_db::DbError::other(e.to_string())))
        .await
        .unwrap()
}

fn contact_row(k: i64, at: &str) -> LinkContact {
    LinkContact {
        contact_id: (100 + k).to_string(),
        contact_inbox_id: (200 + k).to_string(),
        source_id: Some(format!("psid-{k}")),
        channel: Some("messenger".into()),
        conversation_id: (300 + k).to_string(),
        occurred_at: at.into(),
    }
}

/// A click whose contact inbox is gone: the row keeps only its time.
fn gone_row(at: &str) -> LinkContact {
    LinkContact {
        contact_id: String::new(),
        contact_inbox_id: String::new(),
        source_id: None,
        channel: None,
        conversation_id: String::new(),
        occurred_at: at.into(),
    }
}

fn days(rows: &[(&str, i64)]) -> Vec<(String, i64)> {
    rows.iter().map(|(d, c)| (d.to_string(), *c)).collect()
}

#[tokio::test]
async fn stats_count_clicks_per_day_in_utc_and_a_local_timezone() {
    for link in [Link::Magic, Link::Ref] {
        let db = seeded(link).await;
        let db: &chatbotx_db::Db = &db;
        let stats = |ws: i64, id: i64, tz: &'static str| {
            read(db, move |c| {
                links::stats_by_day(c, link, ws, id, range(), tz)
            })
        };
        assert_eq!(
            stats(WS, LINK, "UTC").await,
            days(&[("2025-08-01", 3), ("2025-08-02", 1), ("2025-08-03", 1)]),
            "{link:?}"
        );
        assert_eq!(
            stats(WS, LINK, "Asia/Ho_Chi_Minh").await,
            days(&[("2025-08-01", 2), ("2025-08-02", 2), ("2025-08-03", 1)]),
            "{link:?}"
        );
        // An unknown or foreign link has no rows.
        assert!(stats(OTHER_WS, LINK, "UTC").await.is_empty());
        assert!(stats(WS, 1, "UTC").await.is_empty());
    }
}

#[tokio::test]
async fn contacts_list_the_contact_inboxes_newest_first_within_the_range() {
    for link in [Link::Magic, Link::Ref] {
        let db = seeded(link).await;
        let db: &chatbotx_db::Db = &db;
        let list = |page: i64, per_page: i64, window: Option<(i64, i64)>| {
            read(db, move |c| {
                links::contacts(c, link, WS, Some(LINK), page, per_page, window)
            })
        };
        assert_eq!(
            list(1, 50, Some(range())).await,
            LinkContacts {
                data: vec![
                    contact_row(3, "2025-08-03T00:30:00.000Z"),
                    gone_row("2025-08-02T00:00:00.000Z"),
                    contact_row(2, "2025-08-01T23:30:00.000Z"),
                    contact_row(1, "2025-08-01T10:00:00.000Z"),
                ],
                total: 4,
                page: 1,
                page_count: 1,
            },
            "{link:?}"
        );
        // Without a full range every click counts.
        assert_eq!(
            list(1, 50, None).await,
            LinkContacts {
                data: vec![
                    contact_row(3, "2025-08-03T00:30:00.000Z"),
                    gone_row("2025-08-02T00:00:00.000Z"),
                    contact_row(2, "2025-08-01T23:30:00.000Z"),
                    contact_row(1, "2025-07-31T23:59:59.000Z"),
                    contact_row(4, "2025-07-20T00:00:00.000Z"),
                ],
                total: 5,
                page: 1,
                page_count: 1,
            }
        );
        assert_eq!(
            list(2, 3, Some(range())).await,
            LinkContacts {
                data: vec![contact_row(1, "2025-08-01T10:00:00.000Z")],
                total: 4,
                page: 2,
                page_count: 2,
            }
        );
        assert_eq!(
            list(5, 3, Some(range())).await,
            LinkContacts {
                data: vec![],
                total: 4,
                page: 5,
                page_count: 2,
            }
        );
    }
}

#[tokio::test]
async fn contacts_of_an_unknown_or_foreign_link_are_an_empty_first_page() {
    for link in [Link::Magic, Link::Ref] {
        let db = seeded(link).await;
        let empty = LinkContacts {
            data: vec![],
            total: 0,
            page: 1,
            page_count: 0,
        };
        for (ws, id) in [(OTHER_WS, Some(LINK)), (WS, Some(1)), (WS, None)] {
            let got = read(&db, move |c| {
                links::contacts(c, link, ws, id, 3, 50, Some(range()))
            })
            .await;
            assert_eq!(got, empty, "{link:?} {ws} {id:?}");
        }
    }
}
