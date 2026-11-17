use chatbotx_db::rusqlite::params;
use serde_json::json;

use super::*;
use crate::contact_scan::tests::setup;

fn at(text: &str) -> i64 {
    js::parse_date(text).unwrap()
}

fn cond(trigger_type: &str, time_value: Option<Value>, time_type: Option<&str>) -> DtCondition {
    DtCondition {
        trigger_type: trigger_type.into(),
        time_value,
        time_type: time_type.map(str::to_owned),
        field: "20".into(),
        ..DtCondition::default()
    }
}

#[test]
fn the_calculator_follows_the_original() {
    let v = Some("2026-11-17 10:00:00");
    let after = cond("after", Some(json!(30)), Some("minutes"));
    assert!(matches(v, &after, "UTC", at("2026-11-17T10:31:00Z")));
    // No lower bound: the execution record keeps it to one run.
    assert!(matches(v, &after, "UTC", at("2026-11-17T09:00:00Z")));
    assert!(!matches(v, &after, "UTC", at("2026-11-17T10:32:00Z")));

    let before = cond("before", Some(json!("1")), Some("days"));
    assert!(matches(v, &before, "UTC", at("2026-11-16T10:30:00Z")));
    assert!(!matches(v, &before, "UTC", at("2026-11-16T10:31:00Z")));
    assert!(!matches(
        v,
        &cond("before", Some(json!(0)), Some("days")),
        "UTC",
        at("2026-11-16T10:00:00Z")
    ));
    assert!(!matches(v, &cond("after", Some(json!(2)), None), "UTC", 0));

    // A bare day is UTC midnight shown in the zone (01:00 in Berlin).
    let day = Some("2026-11-17");
    let mut nine = cond("atTheDayOf", None, None);
    nine.at = Some(json!("9"));
    assert!(matches(
        day,
        &nine,
        "Europe/Berlin",
        at("2026-11-17T08:15:00Z")
    ));
    assert!(!matches(day, &nine, "UTC", at("2026-11-17T08:15:00Z")));
    // The condition zone wins over the workspace zone.
    nine.timezone = Some("Europe/Berlin".into());
    assert!(matches(day, &nine, "UTC", at("2026-11-17T08:15:00Z")));
    let own_hour = cond("atTheDayOf", None, None);
    assert!(matches(
        day,
        &own_hour,
        "Europe/Berlin",
        at("2026-11-17T00:05:00Z")
    ));
    assert!(!matches(
        day,
        &own_hour,
        "Europe/Berlin",
        at("2026-11-17T08:15:00Z")
    ));

    assert!(!matches(Some("not a date"), &after, "UTC", 0));
    assert!(!matches(Some(""), &after, "UTC", 0));
    assert!(!matches(None, &after, "UTC", 0));
}

#[tokio::test]
async fn the_sweeps_run_each_trigger_and_webhook_once_per_contact() {
    let (deps, jobs) = setup().await;
    let value = iso(now_ms() - 5 * 60_000);
    deps.db
        .write(move |tx| {
            for sql in [
                r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (30, 2), (31, 2)"#,
                r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (20, 'Due', 'datetime', 2), (21, 'Other', 'datetime', 2)"#,
                r#"INSERT INTO "Trigger" ("id", "name", "actions", "workspaceId") VALUES (40, 'T', '[]', 2), (41, 'Both', '[]', 2)"#,
                r#"INSERT INTO "Webhook" ("id", "name", "url", "workspaceId") VALUES (50, 'W', 'https://hook.example.test', 2)"#,
                r#"INSERT INTO "Condition" ("triggerId", "type", "sourceId", "value") VALUES
                     (40, 'dateTimeBasedTrigger', '20', '{"triggerType":"after","timeValue":5,"timeType":"minutes"}'),
                     (41, 'dateTimeBasedTrigger', '20', '{"triggerType":"after","timeValue":5,"timeType":"minutes"}'),
                     (41, 'dateTimeBasedTrigger', '21', '{"triggerType":"after","timeValue":5,"timeType":"minutes"}')"#,
                r#"INSERT INTO "Condition" ("webhookId", "type", "sourceId", "value") VALUES
                     (50, 'dateTimeBasedTrigger', '21', '{"triggerType":"after","timeValue":5,"timeType":"minutes"}'),
                     (50, 'dateTimeBasedTrigger', '20', '{"triggerType":"after","timeValue":5,"timeType":"minutes"}')"#,
            ] {
                tx.execute(sql, [])?;
            }
            // Contact 30 has both fields; 31 only field 20.
            tx.execute(
                r#"INSERT INTO "ContactCustomField" ("value", "contactId", "customFieldId") VALUES (?1, 30, 20), (?1, 30, 21), (?1, 31, 20)"#,
                params![value],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let pairs = |table: &'static str, column: &'static str| {
        let db = deps.db.clone();
        async move {
            db.read(move |c| {
                let mut stmt = c.prepare(&format!(
                    r#"SELECT "{column}", "contactId" FROM "{table}" ORDER BY 1, 2"#
                ))?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap()
        }
    };

    scan_triggers(&deps).await.unwrap();
    scan_triggers(&deps).await.unwrap();
    // Trigger 41 needs both fields: only contact 30 has them.
    assert_eq!(
        pairs("TriggerExecution", "triggerId").await,
        vec![(40, 30), (40, 31), (41, 30)]
    );

    scan_webhooks(&deps).await.unwrap();
    scan_webhooks(&deps).await.unwrap();
    assert_eq!(
        pairs("WebhookExecution", "webhookId").await,
        vec![(50, 30), (50, 31)]
    );
    let names: Vec<String> = jobs
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    assert_eq!(names, vec!["evaluate-webhooks", "evaluate-webhooks"]);

    cleanup(&deps, false).await.unwrap();
    assert_eq!(pairs("TriggerExecution", "triggerId").await.len(), 3);
}
