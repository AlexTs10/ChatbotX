//! Port of `packages/database/__tests__/integration/working-hours-contact-filter.test.ts`
//! (P3.2b slice C1): `isWithinWorkingHours` through `build_contact_where` on a migrated temp
//! file (instead of a rolled-back Postgres transaction). Schedules are open every minute of
//! every day or have no intervals; "now" is the fixed `repo8::NOW` (2026-06-01, UTC), so the
//! date overrides are seeded on that day.

mod common;
#[path = "common/repo8.rs"]
mod repo8;
#[path = "common/repo9.rs"]
mod repo9;

use chatbotx_db::Db;
use chatbotx_db::repo::queries::contact_filter::{ContactWhereInput, build_contact_where};
use chatbotx_db::rusqlite::params;
use common::{TestDb, fresh_db, seed_workspace};
use repo8::{NOW, crit};
use serde_json::json;

const TODAY: &str = "2026-06-01";

struct Fx {
    db: TestDb,
    next: std::sync::atomic::AtomicI64,
}

impl Fx {
    async fn new() -> Fx {
        let db = fresh_db().await;
        seed_workspace(&db, 1, 1).await;
        seed_workspace(&db, 2, 2).await;
        Fx {
            db,
            next: std::sync::atomic::AtomicI64::new(100),
        }
    }

    fn id(&self) -> i64 {
        self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    async fn exec(&self, sql: &'static str, args: Vec<chatbotx_db::rusqlite::types::Value>) {
        self.db
            .write(move |tx| {
                tx.execute(sql, chatbotx_db::rusqlite::params_from_iter(args.iter()))?;
                Ok(())
            })
            .await
            .expect("fixture write");
    }

    async fn user(&self) -> i64 {
        let id = self.id();
        self.exec(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, 'u' || ?1 || '@example.test')"#,
            vec![id.into()],
        )
        .await;
        id
    }

    /// `seedSchedule`: open = 7 intervals 0..1440; `member` joins it.
    async fn schedule(&self, ws: i64, open: bool, member: Option<i64>) -> i64 {
        let id = self.id();
        let name = if open { "Always open" } else { "Always closed" };
        self.exec(
            r#"INSERT INTO "WorkingHours" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
            vec![id.into(), ws.into(), name.to_owned().into()],
        )
        .await;
        if open {
            for weekday in 0..7_i64 {
                let iid = self.id();
                self.exec(
                    r#"INSERT INTO "WorkingHoursInterval" ("id", "workingHoursId", "weekday", "startMinute", "endMinute") VALUES (?1, ?2, ?3, 0, 1440)"#,
                    vec![iid.into(), id.into(), weekday.into()],
                )
                .await;
            }
        }
        if let Some(user) = member {
            let mid = self.id();
            self.exec(
                r#"INSERT INTO "WorkingHoursMember" ("id", "workingHoursId", "workspaceId", "userId") VALUES (?1, ?2, ?3, ?4)"#,
                vec![mid.into(), id.into(), ws.into(), user.into()],
            )
            .await;
        }
        id
    }

    /// `seedContact`: a contact with one conversation (optionally assigned).
    async fn contact(&self, ws: i64, assigned: Option<i64>) -> i64 {
        let (id, conv) = (self.id(), self.id());
        self.db
            .write(move |tx| {
                tx.execute(r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#, params![id, ws])?;
                tx.execute(
                    r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "assignedUserId") VALUES (?1, ?2, ?3, ?4)"#,
                    params![conv, ws, id, assigned],
                )?;
                Ok(())
            })
            .await
            .expect("contact");
        id
    }

    /// `addOverride` on [`TODAY`].
    async fn add_override(&self, ws: i64, schedule: Option<i64>, open: bool) {
        let id = self.id();
        let ranges = if open {
            r#"[{"startMinute":0,"endMinute":1440}]"#
        } else {
            "[]"
        };
        self.db
            .write(move |tx| {
                tx.execute(
                    r#"INSERT INTO "WorkingHoursDateOverride" ("id", "workspaceId", "workingHoursId", "date", "name", "ranges") VALUES (?1, ?2, ?3, ?4, 'Override', ?5)"#,
                    params![id, ws, schedule, TODAY, ranges],
                )?;
                Ok(())
            })
            .await
            .expect("override");
    }
}

async fn matches(
    db: &Db,
    ws: i64,
    contact: i64,
    op: &str,
    value: serde_json::Value,
    tz: Option<&str>,
) -> bool {
    let mut f = json!({
        "operator": "and",
        "conditions": [{ "field": "isWithinWorkingHours", "operator": op, "value": value }],
    });
    if let Some(tz) = tz {
        f["workingHoursTimezone"] = json!(tz);
    }
    let q = build_contact_where(&ContactWhereInput {
        workspace_id: ws,
        keyword: None,
        contact_filter: Some(crit(f)),
        now: NOW,
    });
    repo9::matching(db, &q).await.contains(&contact)
}

// ts:150 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_a_schedule_open_now_matches_in_and_not_in_excludes_it()
 {
    let fx = Fx::new().await;
    let open = fx.schedule(1, true, None).await;
    let c = fx.contact(1, None).await;
    assert!(matches(&fx.db, 1, c, "in", json!([open.to_string()]), None).await);
    assert!(!matches(&fx.db, 1, c, "notIn", json!([open.to_string()]), None).await);
}

// ts:164 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_a_schedule_without_intervals_is_closed()
 {
    let fx = Fx::new().await;
    let closed = fx.schedule(1, false, None).await;
    let c = fx.contact(1, None).await;
    assert!(!matches(&fx.db, 1, c, "in", json!([closed.to_string()]), None).await);
    assert!(matches(&fx.db, 1, c, "notIn", json!([closed.to_string()]), None).await);
}

// ts:178 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_an_unknown_timezone_falls_back_to_utc_instead_of_throwing()
 {
    let fx = Fx::new().await;
    let open = fx.schedule(1, true, None).await;
    let c = fx.contact(1, None).await;
    assert!(
        matches(
            &fx.db,
            1,
            c,
            "in",
            json!([open.to_string()]),
            Some("Not/AZone")
        )
        .await
    );
}

// ts:189 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_a_real_non_utc_timezone_is_accepted()
 {
    let fx = Fx::new().await;
    let open = fx.schedule(1, true, None).await;
    let c = fx.contact(1, None).await;
    assert!(
        matches(
            &fx.db,
            1,
            c,
            "in",
            json!([open.to_string()]),
            Some("Asia/Ho_Chi_Minh")
        )
        .await
    );
}

// ts:207 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_a_schedule_of_another_workspace_never_matches()
 {
    let fx = Fx::new().await;
    let foreign = fx.schedule(2, true, None).await;
    let c = fx.contact(1, None).await;
    assert!(!matches(&fx.db, 1, c, "in", json!([foreign.to_string()]), None).await);
}

// ts:219 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_assigned_agent_on_shift_agent_matches_off_shift_unassigned_do_not_unscheduled_agent_counts_as_on_shift()
 {
    let fx = Fx::new().await;
    let (on, off, free) = (fx.user().await, fx.user().await, fx.user().await);
    fx.schedule(1, true, Some(on)).await;
    fx.schedule(1, false, Some(off)).await;
    let on_c = fx.contact(1, Some(on)).await;
    let off_c = fx.contact(1, Some(off)).await;
    let free_c = fx.contact(1, Some(free)).await;
    let none_c = fx.contact(1, None).await;
    let a = || json!(["assignedAgent"]);
    assert!(matches(&fx.db, 1, on_c, "in", a(), None).await);
    assert!(!matches(&fx.db, 1, off_c, "in", a(), None).await);
    assert!(matches(&fx.db, 1, free_c, "in", a(), None).await);
    assert!(!matches(&fx.db, 1, none_c, "in", a(), None).await);
    assert!(matches(&fx.db, 1, off_c, "notIn", a(), None).await);
    assert!(matches(&fx.db, 1, none_c, "notIn", a(), None).await);
}

// ts:262 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_a_selection_with_an_unknown_schedule_id_and_no_assigned_agent_is_a_no_op()
 {
    let fx = Fx::new().await;
    let c = fx.contact(1, None).await;
    assert!(matches(&fx.db, 1, c, "in", json!(["not-a-number"]), None).await);
}

// ts:303 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_date_overrides_a_workspace_holiday_closes_an_always_open_schedule()
 {
    let fx = Fx::new().await;
    let open = fx.schedule(1, true, None).await;
    let c = fx.contact(1, None).await;
    fx.add_override(1, None, false).await;
    assert!(!matches(&fx.db, 1, c, "in", json!([open.to_string()]), None).await);
    assert!(matches(&fx.db, 1, c, "notIn", json!([open.to_string()]), None).await);
}

// ts:318 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_date_overrides_a_holiday_with_hours_opens_an_always_closed_schedule()
 {
    let fx = Fx::new().await;
    let closed = fx.schedule(1, false, None).await;
    let c = fx.contact(1, None).await;
    fx.add_override(1, None, true).await;
    assert!(matches(&fx.db, 1, c, "in", json!([closed.to_string()]), None).await);
}

// ts:330 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_date_overrides_the_schedule_s_own_override_wins_over_the_workspace_holiday()
 {
    let fx = Fx::new().await;
    let open = fx.schedule(1, true, None).await;
    let c = fx.contact(1, None).await;
    fx.add_override(1, None, false).await;
    fx.add_override(1, Some(open), true).await;
    assert!(matches(&fx.db, 1, c, "in", json!([open.to_string()]), None).await);
}

// ts:343 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_date_overrides_assigned_agent_follows_the_overridden_schedule()
 {
    let fx = Fx::new().await;
    let agent = fx.user().await;
    fx.schedule(1, true, Some(agent)).await;
    let c = fx.contact(1, Some(agent)).await;
    fx.add_override(1, None, false).await;
    assert!(!matches(&fx.db, 1, c, "in", json!(["assignedAgent"]), None).await);
}

// ts:356 ported
#[tokio::test]
async fn is_within_working_hours_contact_filter_against_postgres_date_overrides_another_workspace_s_holiday_is_ignored()
 {
    let fx = Fx::new().await;
    let open = fx.schedule(1, true, None).await;
    let c = fx.contact(1, None).await;
    fx.add_override(2, None, false).await;
    assert!(matches(&fx.db, 1, c, "in", json!([open.to_string()]), None).await);
}
