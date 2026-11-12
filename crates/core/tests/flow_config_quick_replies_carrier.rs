//! Port of `packages/flow-config/__tests__/quick-replies-carrier.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn quick_replies() -> Value {
    json!([fc::button_step_default_fn(&json!({ "label": "Yes" }))])
}

fn image_step() -> Value {
    spread(
        &fc::send_image_step_default_fn(),
        json!({ "url": "https://example.com/image.png" }),
    )
}

#[test]
fn sendmessage_quick_replies_accepts_an_image_only_node_with_quick_replies_for_s_builder_does_not_gate_on_carrier_eligibility()
 {
    for channel in [
        "omnichannel",
        "telegram",
        "whatsapp",
        "zalo",
        "messenger",
        "instagram",
        "tiktok",
    ] {
        let node = fc::send_message_node_default_fn(&json!({
            "nodeProps": {},
            "detailProps": {
                "beforeStep": fc::choose_channel_step_default_fn(&json!({ "channel": channel })),
                "steps": [image_step()],
                "quickReplies": quick_replies(),
            },
        }));

        let result = fc::send_message_node_schema().safe_parse(&node);

        assert!(result.success, "{channel}");
    }
}

#[test]
fn sendmessage_quick_replies_returns_carrier_eligibility_by_channel_used_by_the_worker_to_decide_whether_a_step_actually_receives_quick_replies()
 {
    let text_step = fc::send_text_step_default_fn(&json!({ "text": "Choose" }));
    let text_with_buttons_step = fc::send_text_step_default_fn(&json!({
        "text": "Choose",
        "buttons": [fc::button_step_default_fn(&json!({ "label": "Existing" }))],
    }));
    let image_step = image_step();
    let video_step = spread(
        &fc::send_video_step_default_fn(),
        json!({ "url": "https://example.com/video.mp4" }),
    );
    let audio_step =
        fc::send_audio_step_default_fn(&json!({ "url": "https://example.com/audio.mp3" }));
    let file_step = spread(
        &fc::send_file_step_default_fn(),
        json!({ "url": "https://example.com/file.pdf" }),
    );
    let gif_step = spread(
        &fc::send_gif_step_default_fn(),
        json!({ "url": "https://example.com/anim.gif" }),
    );
    let carousel_step = fc::send_carousel_step_default_fn();
    let view_catalog_step = fc::send_view_catalog_step_default_fn(&json!({ "text": "Browse" }));
    let multiple_images_step = fc::send_multiple_images_step_default_fn();

    assert!(fc::is_quick_reply_carrier_step("telegram", &text_step));
    assert!(fc::is_quick_reply_carrier_step("telegram", &image_step));
    assert!(fc::is_quick_reply_carrier_step("telegram", &video_step));
    assert!(fc::is_quick_reply_carrier_step("telegram", &audio_step));
    assert!(fc::is_quick_reply_carrier_step("telegram", &file_step));
    assert!(fc::is_quick_reply_carrier_step("telegram", &gif_step));
    // Telegram's sendMediaGroup has no reply_markup/buttons support, unlike
    // Instagram/Messenger's attachments[] — so it must NOT be picked as a
    // carrier here (choosing it would silently drop the quick replies).
    assert!(!fc::is_quick_reply_carrier_step(
        "telegram",
        &multiple_images_step
    ));

    assert!(fc::is_quick_reply_carrier_step("whatsapp", &image_step));
    assert!(fc::is_quick_reply_carrier_step("whatsapp", &carousel_step));
    // A catalog message carries no quick replies: its interactive action holds
    // only the catalog button, so it must never be picked as the carrier.
    assert!(!fc::is_quick_reply_carrier_step(
        "whatsapp",
        &view_catalog_step
    ));
    assert!(fc::is_quick_reply_carrier_step("zalo", &image_step));
    assert!(fc::is_quick_reply_carrier_step("messenger", &image_step));
    assert!(fc::is_quick_reply_carrier_step("instagram", &image_step));
    assert!(!fc::is_quick_reply_carrier_step("tiktok", &image_step));
    assert!(fc::is_quick_reply_carrier_step("messenger", &text_step));
    assert!(fc::is_quick_reply_carrier_step(
        "messenger",
        &text_with_buttons_step
    ));
    assert!(fc::is_quick_reply_carrier_step(
        "messenger",
        &multiple_images_step
    ));
    assert!(fc::is_quick_reply_carrier_step(
        "instagram",
        &multiple_images_step
    ));
}

#[test]
fn sendmessage_quick_replies_enforces_quick_reply_label_and_count_limits() {
    let label = |text: String| fc::button_step_default_fn(&json!({ "label": text }));
    assert!(
        fc::button_step_schema()
            .safe_parse(&label(rep("a", fc::BUTTON_LABEL_MAX)))
            .success
    );
    assert!(
        !fc::button_step_schema()
            .safe_parse(&label(rep("a", fc::BUTTON_LABEL_MAX + 1)))
            .success
    );
    assert!(
        fc::button_step_schema()
            .safe_parse(&label(rep("😀", fc::BUTTON_LABEL_MAX)))
            .success
    );

    let replies: Vec<Value> = (0..fc::MAX_QUICK_REPLIES + 1)
        .map(|i| fc::button_step_default_fn(&json!({ "label": format!("Reply {i}") })))
        .collect();
    let node = fc::send_message_node_default_fn(&json!({
        "nodeProps": {},
        "detailProps": {
            "steps": [fc::send_text_step_default_fn(&json!({ "text": "Choose one" }))],
            "quickReplies": replies,
        },
    }));

    let result = fc::send_message_node_schema().safe_parse(&node);

    assert!(!result.success);
}

#[test]
fn sendmessage_quick_replies_does_not_default_legacy_sendquickreply_steps_to_a_hardcoded_prompt() {
    let step = fc::send_quick_reply_step_default_fn(&json!({}));

    assert_ne!(step["message"], "Please select an option");
    assert_eq!(step["message"], "");
}
