//! `condition-evaluator.test.ts`. The mocked `findValue` / `findBy` are DB
//! rows (custom field 20 on contact 3); ids are numeric strings.

use serde_json::{Map, Value, json};

use super::{Condition, EvalContext, evaluate};
use crate::steps::test_support::Fixture;

fn ms(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).unwrap()
}

fn cond(r#type: &str, source_id: Option<&str>, operator: Option<&str>, value: Value) -> Condition {
    Condition {
        r#type: r#type.into(),
        source_id: source_id.map(Into::into),
        operator: operator.map(Into::into),
        value: (!value.is_null()).then_some(value),
    }
}

async fn eval_at(f: &Fixture, c: &Condition, meta: Value, tz: &str, now: i64) -> bool {
    let meta: Map<String, Value> = meta.as_object().cloned().unwrap_or_default();
    evaluate(
        &f.deps.db,
        c,
        &EvalContext {
            metadata: &meta,
            contact_id: "3",
            workspace_timezone: Some(tz),
            now_ms: now,
        },
    )
    .await
    .unwrap()
}

async fn eval(f: &Fixture, c: &Condition, meta: Value) -> bool {
    eval_at(f, c, meta, "UTC", 0).await
}

async fn field(f: &Fixture, field_type: &'static str, value: Option<&'static str>) {
    f.sql(move |tx| {
        tx.execute(
            r#"INSERT OR REPLACE INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (20, 'cf', ?1, 2)"#,
            [field_type],
        )?;
        tx.execute(r#"DELETE FROM "ContactCustomField""#, [])?;
        if let Some(value) = value {
            tx.execute(
                r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (21, 3, 20, ?1)"#,
                [value],
            )?;
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn contact_info_updated() {
    let f = Fixture::new().await;
    let meta = json!({ "infoType": "phone", "newValue": "+84912345678" });
    assert!(
        eval(
            &f,
            &cond("contactInfoUpdated", Some("phone"), None, Value::Null),
            meta.clone()
        )
        .await
    );
    assert!(
        !eval(
            &f,
            &cond("contactInfoUpdated", Some("email"), None, Value::Null),
            meta.clone()
        )
        .await
    );
    assert!(
        !eval(
            &f,
            &cond("contactInfoUpdated", None, None, Value::Null),
            meta
        )
        .await
    );
    // still matches tag conditions through the shared source-id check
    assert!(
        eval(
            &f,
            &cond("tagApplied", Some("tag-1"), None, Value::Null),
            json!({ "tagId": "tag-1" })
        )
        .await
    );
}

#[tokio::test]
async fn datetime_timezone() {
    let f = Fixture::new().await;
    field(&f, "datetime", Some("2026-07-11T02:00:00.000Z")).await;
    let now = ms("2026-07-11T14:00:00.000Z");
    let at = |tz: Option<&str>| {
        let mut v = json!({ "triggerType": "atTheDayOf", "at": "21" });
        if let Some(tz) = tz {
            v["timezone"] = json!(tz);
        }
        cond("dateTimeBasedTrigger", Some("20"), None, v)
    };
    // the condition's captured timezone wins over the workspace zone
    assert!(eval_at(&f, &at(Some("Asia/Ho_Chi_Minh")), json!({}), "UTC", now).await);
    assert!(!eval_at(&f, &at(Some("UTC")), json!({}), "UTC", now).await);
    // falls back to the workspace timezone
    assert!(eval_at(&f, &at(None), json!({}), "Asia/Ho_Chi_Minh", now).await);
}

#[tokio::test]
async fn datetime_date_type_anchor() {
    let f = Fixture::new().await;
    field(&f, "date", Some("2026-07-11T00:00:00+07:00")).await;
    let c = |tz: &str| {
        cond(
            "dateTimeBasedTrigger",
            Some("20"),
            None,
            json!({ "triggerType": "atTheDayOf", "timezone": tz }),
        )
    };
    // start of the day (hour 0) in the captured zone
    assert!(
        eval_at(
            &f,
            &c("Asia/Ho_Chi_Minh"),
            json!({}),
            "UTC",
            ms("2026-07-10T17:00:00.000Z")
        )
        .await
    );
    // not the end of the day (hour 23)
    assert!(
        !eval_at(
            &f,
            &c("Asia/Ho_Chi_Minh"),
            json!({}),
            "UTC",
            ms("2026-07-11T16:00:00.000Z")
        )
        .await
    );
    // an invalid captured zone degrades to UTC
    assert!(
        eval_at(
            &f,
            &c("Not/AZone"),
            json!({}),
            "UTC",
            ms("2026-07-10T17:00:00.000Z")
        )
        .await
    );
}

#[tokio::test]
async fn custom_field_operator_vocabulary() {
    let f = Fixture::new().await;
    let check = |op: &'static str, value: Value, field_type: &'static str, new_value: Value| {
        let f = &f;
        async move {
            field(f, field_type, None).await;
            eval(
                f,
                &cond("customFieldValueChanged", Some("20"), Some(op), value),
                json!({ "customFieldId": "20", "newValue": new_value }),
            )
            .await
        }
    };
    assert!(check("eq", json!({ "text": "vip" }), "shortText", json!("vip")).await);
    assert!(
        !check(
            "eq",
            json!({ "text": "vip" }),
            "shortText",
            json!("regular")
        )
        .await
    );
    assert!(
        check(
            "ne",
            json!({ "text": "vip" }),
            "shortText",
            json!("regular")
        )
        .await
    );
    assert!(check("isNotEmpty", Value::Null, "shortText", json!("vip")).await);
    assert!(check("isEmpty", Value::Null, "shortText", json!("")).await);
    assert!(
        check(
            "notContains",
            json!({ "text": "xyz" }),
            "shortText",
            json!("abcdef")
        )
        .await
    );
    assert!(
        check(
            "eq",
            json!({ "text": "2026-07-11T00:00:00+07:00", "timezone": "Asia/Ho_Chi_Minh" }),
            "date",
            json!("2026-07-11T00:00:00+07:00"),
        )
        .await
    );
}

#[tokio::test]
async fn whatsapp_call_conditions() {
    let f = Fixture::new().await;
    for t in [
        "incomingCall",
        "missedAudioCall",
        "callEnded",
        "callRecorded",
        "callTranscribed",
    ] {
        assert!(
            eval(
                &f,
                &cond(t, None, None, Value::Null),
                json!({ "callId": "call-1" })
            )
            .await,
            "{t}"
        );
    }
    // a condition the screen does not offer still does not match
    assert!(!eval(&f, &cond("bogusEvent", None, None, Value::Null), json!({})).await);
}

fn product(id: Option<&str>) -> Value {
    json!({
        "catalogReference": { "source": "metaCatalog", "catalogId": "catalog-1", "retailerId": format!("sku-{id:?}") },
        "product": id.map(|id| json!({ "productId": id, "name": "n", "imageUrl": null, "price": 1, "currency": "USD" })),
    })
}

#[tokio::test]
async fn catalog_conditions() {
    let f = Fixture::new().await;
    let order = json!({ "products": [
        { "productId": "product-1", "categoryIds": [], "quantity": 1 },
        { "productId": null, "categoryIds": [], "quantity": 1 },
    ] });
    let cart_item = json!({
        "product": { "productId": "product-1", "variantId": null, "name": "Shirt", "categoryIds": ["category-1", "sub-1"] },
        "quantity": 1, "change": 1,
    });
    for (t, data) in [
        ("productOrdered", &order),
        ("productAddedToCart", &cart_item),
        ("productRemovedFromCart", &cart_item),
    ] {
        for (source, expected) in [
            (None, true),
            (Some("product-1"), true),
            (Some("product-2"), false),
        ] {
            assert_eq!(
                eval(&f, &cond(t, source, None, Value::Null), data.clone()).await,
                expected,
                "{t} {source:?}"
            );
        }
    }
    for (source, expected) in [
        (None, true),
        (Some("category-1"), true),
        (Some("sub-1"), true),
        (Some("category-2"), false),
    ] {
        assert_eq!(
            eval(
                &f,
                &cond("categoryAddedToCart", source, None, Value::Null),
                cart_item.clone()
            )
            .await,
            expected
        );
    }
    for t in [
        "newOrder",
        "orderAccepted",
        "orderShipped",
        "orderConcluded",
        "orderCancelled",
        "orderPaid",
        "orderRefunded",
        "cartAbandoned",
    ] {
        assert!(
            eval(&f, &cond(t, None, None, Value::Null), json!({})).await,
            "{t}"
        );
    }

    let cart = json!({ "products": [
        merge(product(Some("product-1")), json!({ "quantity": 1 })),
        merge(product(None), json!({ "quantity": 1 })),
    ] });
    let inquiry = json!({ "product": product(Some("product-1")) });
    for (t, data) in [
        ("shoppingCartSent", &cart),
        ("userAskedAboutProduct", &inquiry),
    ] {
        assert!(eval(&f, &cond(t, None, None, Value::Null), data.clone()).await);
        assert!(
            eval(
                &f,
                &cond(t, Some("product-1"), None, Value::Null),
                data.clone()
            )
            .await
        );
        assert!(
            !eval(
                &f,
                &cond(t, Some("product-2"), None, Value::Null),
                data.clone()
            )
            .await
        );
        let unresolved = if t == "shoppingCartSent" {
            json!({ "products": [merge(product(None), json!({ "quantity": 1 }))] })
        } else {
            json!({ "product": product(None) })
        };
        assert!(
            !eval(
                &f,
                &cond(t, Some("product-1"), None, Value::Null),
                unresolved
            )
            .await
        );
        assert!(
            !eval(
                &f,
                &cond(t, Some("product-1"), None, Value::Null),
                json!({})
            )
            .await
        );
    }
}

fn merge(mut a: Value, b: Value) -> Value {
    for (k, v) in b.as_object().unwrap() {
        a[k] = v.clone();
    }
    a
}

#[tokio::test]
async fn pipeline_item_conditions() {
    let f = Fixture::new().await;
    let meta = json!({ "pipelineId": "p-1", "stageId": "s-2" });
    for t in [
        "pipelineItemCreated",
        "pipelineItemMovedToStage",
        "pipelineItemValueChanged",
        "pipelineItemStatusChanged",
        "pipelineItemPriorityChanged",
    ] {
        assert!(eval(&f, &cond(t, None, None, Value::Null), meta.clone()).await);
    }
    assert!(
        eval(
            &f,
            &cond("pipelineItemCreated", Some("p-1"), None, Value::Null),
            meta.clone()
        )
        .await
    );
    assert!(
        !eval(
            &f,
            &cond("pipelineItemCreated", Some("p-9"), None, Value::Null),
            meta.clone()
        )
        .await
    );
    let moved = "pipelineItemMovedToStage";
    assert!(
        eval(
            &f,
            &cond(moved, None, None, json!({ "stageId": "s-2" })),
            meta.clone()
        )
        .await
    );
    assert!(
        !eval(
            &f,
            &cond(moved, None, None, json!({ "stageId": "s-3" })),
            meta.clone()
        )
        .await
    );
    assert!(
        eval(
            &f,
            &cond(moved, None, None, json!({ "stageId": null })),
            meta
        )
        .await
    );
}
