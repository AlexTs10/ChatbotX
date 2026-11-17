//! The heavy-step runner end to end on the worker fixture: the flow waits,
//! the heavy job records the outcome, the resume routes on it. The provider
//! calls are not reached (no integration row), so no HTTP fake is needed.

use serde_json::{Value, json};

use super::*;
use crate::steps::test_support::*;

const SUCCESS: &str = "81";
const ERROR: &str = "82";
const SKIP: &str = "83";

fn generate_step() -> Value {
    json!({ "id": "1", "stepType": "aiGenerateImage", "provider": "openai", "model": "gpt-image-1",
            "prompt": " a cat ", "quality": "auto", "size": "auto", "outputFieldId": "30",
            "states": [{ "id": SUCCESS, "stateType": "success" }, { "id": ERROR, "stateType": "error" }] })
}

async fn start(fx: &Fixture, step: Value) {
    fx.exec(r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (30, 'out', 'shortText', 2)"#)
        .await;
    fx.node(
        json!([step]),
        json!([{ "sourceHandle": SUCCESS, "target": "n2" }, { "sourceHandle": ERROR, "target": "n3" }, { "sourceHandle": SKIP, "target": "n2" }]),
    )
    .await;
    fx.run(json!({ "metadata": { "type": "flow" } }))
        .await
        .unwrap();
}

/// The resume job for `step` after the heavy job finished.
async fn resume(fx: &Fixture) {
    let resumes = fx.jobs.named(RESUME_HEAVY_STEP);
    let last = resumes.last().expect("a resume job");
    resume_heavy_step(&fx.deps, &last.data["data"])
        .await
        .unwrap();
}

fn routed_to(fx: &Fixture, node: &str) -> bool {
    fx.jobs.send_flows().iter().any(|job| job["nodeId"] == node)
}

#[tokio::test]
async fn the_flow_waits_then_a_provider_error_routes_to_error() {
    let fx = Fixture::new().await;
    start(&fx, generate_step()).await;

    let heavy = fx.jobs.named("aiGenerateImage");
    assert_eq!(heavy.len(), 1);
    let job_id = heavy[0].opts.job_id.clone().unwrap();
    assert!(job_id.starts_with("heavy-aiGenerateImage-"));
    assert_eq!(heavy[0].data["data"]["step"]["prompt"], "a cat");
    assert_eq!(heavy[0].data["data"]["continuation"]["nodeId"], "n1");
    let fallback = fx.jobs.named(RESUME_HEAVY_STEP);
    assert_eq!(fallback.len(), 1);
    assert_eq!(
        fallback[0].opts.delay_ms,
        Some(heavy_completion_wait_timeout_ms(
            HeavyJobAction::AiGenerateImage,
            wait_timeout_ms()
        ))
    );
    assert_eq!(fallback[0].data["data"]["startFromStepId"], "1");
    assert!(!routed_to(&fx, "n2") && !routed_to(&fx, "n3"));

    // No OpenAI integration: an expected error, recorded, then resumed.
    let mut job = job(heavy[0].data.clone());
    job.job_id = job_id.clone();
    let result = handle_step_job(&fx.deps, &job).await.unwrap();
    assert_eq!(result["status"], "error");
    assert_eq!(result["errorMessage"], "AI integration not found");
    let resumes = fx.jobs.named(RESUME_HEAVY_STEP);
    assert_eq!(resumes.len(), 2);
    assert_eq!(
        resumes[1].opts.job_id.as_deref(),
        Some(format!("heavy-resume-{job_id}").as_str())
    );

    resume(&fx).await;
    assert!(routed_to(&fx, "n3") && !routed_to(&fx, "n2"));
    // The fallback is now a no-op: the outcome is resumed.
    let before = fx.jobs.all().len();
    resume_heavy_step(&fx.deps, &fallback[0].data["data"])
        .await
        .unwrap();
    assert_eq!(fx.jobs.all().len(), before);
    // A retried heavy job skips (terminal outcome).
    assert_eq!(handle_step_job(&fx.deps, &job).await.unwrap(), Value::Null);
}

#[tokio::test]
async fn a_success_writes_the_field_once_and_routes_to_success() {
    let fx = Fixture::new().await;
    start(&fx, generate_step()).await;
    let heavy = fx.jobs.named("aiGenerateImage");
    let key = heavy[0].data["data"]["outcomeKey"]
        .as_str()
        .unwrap()
        .to_owned();
    let conversation = load_conversation(&fx.deps.db, CONVERSATION)
        .await
        .unwrap()
        .unwrap();
    let kv = kv(&fx.deps);
    let result = HeavyStepResult::Success {
        output_values: vec![HeavyOutputValue {
            custom_field_id: "30".into(),
            value: "https://cdn.test/a.png".into(),
        }],
    };
    complete(&fx.deps, &kv, &key, &result, &conversation)
        .await
        .unwrap();
    // A late duplicate completion writes nothing more.
    let other = HeavyStepResult::Success {
        output_values: vec![HeavyOutputValue {
            custom_field_id: "30".into(),
            value: "https://cdn.test/b.png".into(),
        }],
    };
    complete(&fx.deps, &kv, &key, &other, &conversation)
        .await
        .unwrap();
    assert_eq!(
        fx.text(r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = 30"#)
            .await
            .as_deref(),
        Some("https://cdn.test/a.png")
    );
    resume(&fx).await;
    assert!(routed_to(&fx, "n2") && !routed_to(&fx, "n3"));
}

#[tokio::test]
async fn a_segment_skip_routes_to_skip_and_invalid_steps_error() {
    let fx = Fixture::new().await;
    let step = json!({ "id": "1", "stepType": "aiSegmentImage", "inputFieldId": "31", "target": "car",
                       "matchSelection": "largest", "outputFieldId": "30",
                       "states": [{ "id": SUCCESS, "stateType": "success" }, { "id": SKIP, "stateType": "skip" }, { "id": ERROR, "stateType": "error" }] });
    start(&fx, step).await;
    let heavy = fx.jobs.named("aiSegmentImage");
    assert_eq!(heavy.len(), 1);
    let key = heavy[0].data["data"]["outcomeKey"]
        .as_str()
        .unwrap()
        .to_owned();
    let conversation = load_conversation(&fx.deps.db, CONVERSATION)
        .await
        .unwrap()
        .unwrap();
    complete(
        &fx.deps,
        &kv(&fx.deps),
        &key,
        &HeavyStepResult::Skip,
        &conversation,
    )
    .await
    .unwrap();
    resume(&fx).await;
    assert!(routed_to(&fx, "n2") && !routed_to(&fx, "n3"));

    // A step that fails its schema routes to error at once.
    let fx = Fixture::new().await;
    let mut bad = generate_step();
    bad["prompt"] = json!("  ");
    start(&fx, bad).await;
    assert!(fx.jobs.named("aiGenerateImage").is_empty());
    assert!(routed_to(&fx, "n3"));
}

#[test]
fn crop_selection_and_details_follow_the_original() {
    let b = |object_id, left, top, right, bottom, confidence| provider::SegmentationBox {
        object_id,
        left,
        top,
        right,
        bottom,
        confidence,
    };
    let boxes = [
        b(1, 0, 0, 2, 2, Some(0.5)),
        b(2, 4, 4, 9, 9, None),
        b(3, 1, 1, 6, 6, None),
    ];
    let (crop, selected) = select_crop(&boxes, false);
    assert_eq!((crop, selected[0].object_id), ((4, 4, 9, 9), 2));
    let (all, selected_all) = select_crop(&boxes, true);
    assert_eq!((all, selected_all.len()), ((0, 0, 9, 9), 3));
    assert_eq!(
        details_json(3, &selected_all[..1], all, (10, 10)),
        r#"{"count":3,"boxes":[{"left":0,"top":0,"right":2,"bottom":2,"confidence":0.5}],"crop":{"left":0,"top":0,"right":9,"bottom":9},"source":{"width":10,"height":10}}"#
    );
    assert_eq!(openai_image_quality("gpt-image-1", "hd"), "high");
    assert_eq!(openai_image_quality("dall-e-3", "md"), "standard");
    assert!(openai_edit_image_quality("x").is_err());
    assert_eq!(
        http_url(Some(" https://a.test/x.png ")).as_deref(),
        Some("https://a.test/x.png")
    );
    assert_eq!(http_url(Some("ftp://a.test/x")), None);
}
