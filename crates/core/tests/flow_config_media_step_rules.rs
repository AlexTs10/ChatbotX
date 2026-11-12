//! Port of `packages/flow-config/__tests__/media-step-rules.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn buttons() -> Vec<Value> {
    vec![fc::button_step_default_fn(&json!({ "label": "Yes" }))]
}

fn step(step_type: &str, extra: Value) -> Value {
    spread(
        &json!({
            "id": "1",
            "stepType": step_type,
            "mode": "file",
            "url": "https://example.com/a.bin",
            "buttons": buttons(),
        }),
        extra,
    )
}

// These expectations mirror the per-channel policies derived from each
// integration's outgoing message handler.
#[test]
fn resolvemediastepsupport_s_s_s() {
    let cases = [
        ("telegram", "sendVideo", "full"),
        ("telegram", "sendFile", "full"),
        ("messenger", "sendImage", "full"),
        ("messenger", "sendVideo", "full"),
        ("messenger", "sendAudio", "noButtons"),
        ("messenger", "sendFile", "noButtons"),
        ("instagram", "sendImage", "noButtons"),
        ("instagram", "sendVideo", "noButtons"),
        ("whatsapp", "sendImage", "full"),
        ("whatsapp", "sendVideo", "unsupported"),
        ("whatsapp", "sendAudio", "unsupported"),
        ("whatsapp", "sendFile", "unsupported"),
        ("whatsapp", "sendGif", "unsupported"),
        ("zalo", "sendImage", "full"),
        ("zalo", "sendVideo", "unsupported"),
        ("zalo", "sendAudio", "unsupported"),
        ("zalo", "sendFile", "noButtons"),
        ("zalo", "sendGif", "full"),
        ("tiktok", "sendImage", "noButtons"),
        ("tiktok", "sendVideo", "unsupported"),
        ("tiktok", "sendGif", "unsupported"),
        ("api", "sendImage", "noButtons"),
        ("webchat", "sendVideo", "full"),
        ("omnichannel", "sendVideo", "full"),
    ];
    for (channel, step_type, expected) in cases {
        assert_eq!(
            fc::resolve_media_step_support(channel, step_type),
            expected,
            "{channel} / {step_type}"
        );
    }
}

#[test]
fn resolvemediastepsupport_an_unknown_or_empty_channel_falls_back_to_full_like_resolvestepvalidator()
 {
    assert_eq!(fc::resolve_media_step_support("", "sendVideo"), "full");
    assert_eq!(fc::resolve_media_step_support("nope", "sendVideo"), "full");
}

#[test]
fn resolvemediastepsupport_a_non_media_step_is_never_constrained_here() {
    assert_eq!(
        fc::resolve_media_step_support("whatsapp", "sendText"),
        "full"
    );
}

#[test]
fn ismediabuttonsdropped_true_only_once_buttons_are_actually_attached() {
    assert!(fc::is_media_buttons_dropped(
        "instagram",
        "sendImage",
        &buttons()
    ));
    assert!(!fc::is_media_buttons_dropped("instagram", "sendImage", &[]));
}

// The step is not sent at all there, so the buttons are not the story.
#[test]
fn ismediabuttonsdropped_false_on_a_channel_where_the_whole_step_is_unsupported() {
    assert!(!fc::is_media_buttons_dropped(
        "whatsapp",
        "sendVideo",
        &buttons()
    ));
}

#[test]
fn ismediastepunsupported_true_where_the_channel_has_no_branch_for_the_step() {
    assert!(fc::is_media_step_unsupported("tiktok", "sendFile"));
}

#[test]
fn ismediastepunsupported_false_where_the_media_is_sent_buttons_or_not() {
    assert!(!fc::is_media_step_unsupported("tiktok", "sendImage"));
}

#[test]
fn media_step_validators_block_publish_per_channel_omnichannel_the_node_default_is_never_blocked() {
    for (validator, step_type) in [
        (fc::send_image_validator(), "sendImage"),
        (fc::send_video_validator(), "sendVideo"),
        (fc::send_audio_validator(), "sendAudio"),
        (fc::send_file_validator(), "sendFile"),
    ] {
        assert!(
            validator
                .base()
                .safe_parse(&step(step_type, json!({})))
                .success,
            "{step_type}"
        );
    }
    assert!(
        fc::send_gif_validator()
            .base()
            .safe_parse(
                &json!({ "id": "1", "stepType": "sendGif", "url": "https://example.com/a.gif" })
            )
            .success
    );
}

#[test]
fn media_step_validators_block_publish_per_channel_rejects_buttons_on_a_channel_that_drops_them_naming_the_buttons_field()
 {
    let result = fc::send_image_validator()
        .channel("instagram")
        .expect("instagram override")
        .safe_parse(&step("sendImage", json!({})));

    assert!(!result.success);
    assert_eq!(result.issues()[0].message, "mediaButtonsUnsupported");
    assert_eq!(result.issues()[0].path_json(), json!(["buttons"]));
}

#[test]
fn media_step_validators_block_publish_per_channel_a_channel_with_nothing_to_say_gets_no_override_at_all()
 {
    // Telegram sends every media step in full, buttons included.
    for validator in [
        fc::send_image_validator(),
        fc::send_video_validator(),
        fc::send_audio_validator(),
        fc::send_file_validator(),
        fc::send_gif_validator(),
    ] {
        assert!(validator.channel("telegram").is_none());
    }
}

#[test]
fn media_step_validators_block_publish_per_channel_every_media_step_type_is_registered_in_channelawarestepvalidators()
 {
    for step_type in ["sendImage", "sendVideo", "sendAudio", "sendFile", "sendGif"] {
        assert!(
            fc::channel_aware_step_validators().contains_key(step_type),
            "{step_type}"
        );
    }
}
