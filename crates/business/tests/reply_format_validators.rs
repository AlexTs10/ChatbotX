//! Port of `packages/business/__tests__/reply-format-validators.test.ts`.

use chatbotx_business::get_user_data::{
    REPLY_FORMATS, ReplyInputAttachment, ReplyInputKind, ReplyInputMessage, ReplyValidation,
    ReplyValidator, accepted, first_accepted, from_attachment, from_location, rejected,
    reply_format_validator, validate_reply_input,
};
use serde_json::json;

const TEXT: ReplyInputKind = ReplyInputKind::Text;

fn text(t: &str) -> ReplyInputMessage {
    ReplyInputMessage {
        text: Some(t.into()),
        ..ReplyInputMessage::default()
    }
}

fn file(file_type: &str, path: &str) -> ReplyInputAttachment {
    ReplyInputAttachment {
        file_type: file_type.into(),
        origin_path: path.into(),
        ..ReplyInputAttachment::default()
    }
}

fn location(attrs: serde_json::Value) -> ReplyInputMessage {
    ReplyInputMessage {
        content_type: "location".into(),
        content_attributes: Some(attrs),
        ..ReplyInputMessage::default()
    }
}

mod reply_format_validators {
    use super::*;

    #[test]
    fn defines_a_validator_for_every_reply_format() {
        for format in REPLY_FORMATS {
            let _ = reply_format_validator(format)(&ReplyInputMessage::default());
        }
        assert_eq!(REPLY_FORMATS.len(), 11);
    }

    #[test]
    fn accepts_and_rejects_number_text() {
        assert_eq!(
            validate_reply_input("RF01", &text("42.5")),
            accepted("42.5", TEXT)
        );
        assert_eq!(
            validate_reply_input("RF01", &text("abc")),
            rejected("getUserData: invalid number")
        );
    }

    #[test]
    fn accepts_and_rejects_email_text() {
        assert_eq!(
            validate_reply_input("RF03", &text("user@example.com")),
            accepted("user@example.com", TEXT)
        );
        assert_eq!(
            validate_reply_input("RF03", &text("invalid")),
            rejected("getUserData: invalid email address")
        );
    }

    #[test]
    fn accepts_and_rejects_phone_text() {
        assert_eq!(
            validate_reply_input("RF04", &text("+1-555-123-4567")),
            accepted("+1-555-123-4567", TEXT)
        );
        assert_eq!(
            validate_reply_input("RF04", &text("hello")),
            rejected("getUserData: invalid phone number")
        );
    }

    #[test]
    fn accepts_and_rejects_link_text() {
        assert_eq!(
            validate_reply_input("RF07", &text("https://example.com/a")),
            accepted("https://example.com/a", TEXT)
        );
        assert_eq!(
            validate_reply_input("RF07", &text("not-a-url")),
            rejected("getUserData: invalid link")
        );
    }

    #[test]
    fn accepts_date_and_datetime_text_as_iso_strings() {
        assert_eq!(
            validate_reply_input("RF09", &text("2026-07-20")),
            accepted("2026-07-20T00:00:00.000Z", TEXT)
        );
        assert_eq!(
            validate_reply_input("RF10", &text("2026-07-20T10:00:00.000Z")),
            accepted("2026-07-20T10:00:00.000Z", TEXT)
        );
    }

    #[test]
    fn rejects_invalid_date_and_datetime_text() {
        assert_eq!(
            validate_reply_input("RF09", &text("never")),
            rejected("getUserData: invalid date")
        );
        assert_eq!(
            validate_reply_input("RF10", &text("never")),
            rejected("getUserData: invalid date")
        );
    }

    #[test]
    fn accepts_text_format_as_text_passthrough() {
        assert_eq!(
            validate_reply_input("RF02", &text("hello")),
            accepted("hello", TEXT)
        );
    }

    #[test]
    fn accepts_a_location_pin_as_latitude_and_longitude_text() {
        assert_eq!(
            validate_reply_input(
                "RF08",
                &location(json!({ "latitude": 10.5, "longitude": 106.75 }))
            ),
            accepted("10.5,106.75", ReplyInputKind::Location)
        );
    }

    #[test]
    fn accepts_typed_lat_lng_coordinates_for_location_format() {
        assert_eq!(
            validate_reply_input("RF08", &text("10.5, 106.75")),
            accepted("10.5,106.75", ReplyInputKind::Location)
        );
    }

    #[test]
    fn rejects_free_text_that_is_not_a_location_pin_or_coordinate_pair() {
        assert_eq!(
            validate_reply_input("RF08", &text("Hanoi")),
            rejected("getUserData: expected a location")
        );
        assert_eq!(
            validate_reply_input("RF08", &text("Received location")),
            rejected("getUserData: expected a location")
        );
    }

    #[test]
    fn rejects_out_of_range_coordinate_pairs() {
        assert_eq!(
            validate_reply_input("RF08", &text("91,0")),
            rejected("getUserData: invalid location")
        );
    }

    #[test]
    fn rejects_attachment_messages_for_text_based_formats() {
        for format in [
            "RF01", "RF02", "RF03", "RF04", "RF07", "RF08", "RF09", "RF10",
        ] {
            let message = ReplyInputMessage {
                text: Some("user@example.com".into()),
                attachments: vec![file("image", "/image.jpg")],
                ..ReplyInputMessage::default()
            };
            assert_eq!(
                validate_reply_input(format, &message),
                rejected("getUserData: unsupported file type"),
                "{format}"
            );
        }
    }

    #[test]
    fn rejects_missing_text_for_text_and_location_formats() {
        let empty = ReplyInputMessage::default();
        assert_eq!(
            validate_reply_input("RF02", &empty),
            rejected("getUserData: expected text input")
        );
        assert_eq!(
            validate_reply_input("RF08", &empty),
            rejected("getUserData: expected text input")
        );
    }

    #[test]
    fn carries_the_attachment_identity_so_a_lazily_ingested_file_can_be_stored_before_it_is_used() {
        let message = ReplyInputMessage {
            attachments: vec![ReplyInputAttachment {
                id: Some(1),
                message_created_at: Some(1_760_403_600_000),
                ..file("image", "https://cdn.example/photo.jpg")
            }],
            ..ReplyInputMessage::default()
        };
        assert_eq!(
            validate_reply_input("RF05", &message),
            ReplyValidation::Ok {
                user_input: "https://cdn.example/photo.jpg".into(),
                kind: ReplyInputKind::Attachment,
                attachment: Some((1, 1_760_403_600_000)),
            }
        );
    }

    #[test]
    fn omits_the_attachment_identity_when_the_row_does_not_provide_one() {
        let message = ReplyInputMessage {
            attachments: vec![file("image", "/image.jpg")],
            ..ReplyInputMessage::default()
        };
        assert_eq!(
            validate_reply_input("RF05", &message),
            accepted("/image.jpg", ReplyInputKind::Attachment)
        );
    }

    #[test]
    fn accepts_image_attachment_only_for_image_format_and_keeps_text_fallback() {
        let image = ReplyInputMessage {
            attachments: vec![file("image", "/image.jpg")],
            ..ReplyInputMessage::default()
        };
        assert_eq!(
            validate_reply_input("RF05", &image),
            accepted("/image.jpg", ReplyInputKind::Attachment)
        );
        let pdf = ReplyInputMessage {
            text: Some("caption should not override unsupported attachment".into()),
            attachments: vec![file("file", "/doc.pdf")],
            ..ReplyInputMessage::default()
        };
        assert_eq!(
            validate_reply_input("RF05", &pdf),
            rejected("getUserData: unsupported file type")
        );
        assert_eq!(
            validate_reply_input("RF05", &text("legacy image fallback")),
            accepted("legacy image fallback", TEXT)
        );
    }

    #[test]
    fn accepts_any_attachment_for_file_format_and_keeps_text_fallback() {
        let voice = ReplyInputMessage {
            attachments: vec![file("audio", "/voice.mp3")],
            ..ReplyInputMessage::default()
        };
        assert_eq!(
            validate_reply_input("RF06", &voice),
            accepted("/voice.mp3", ReplyInputKind::Attachment)
        );
        assert_eq!(
            validate_reply_input("RF06", &text("legacy file fallback")),
            accepted("legacy file fallback", TEXT)
        );
        assert_eq!(
            validate_reply_input("RF06", &ReplyInputMessage::default()),
            rejected("getUserData: expected text input")
        );
    }

    #[test]
    fn accepts_any_input_text() {
        assert_eq!(
            validate_reply_input("RF11", &text("free-form answer")),
            accepted("free-form answer", TEXT)
        );
    }

    #[test]
    fn accepts_any_input_attachments() {
        for (kind, path) in [
            ("image", "/image.jpg"),
            ("video", "/video.mp4"),
            ("audio", "/audio.mp3"),
            ("gif", "/animation.gif"),
            ("file", "/document.pdf"),
        ] {
            let message = ReplyInputMessage {
                attachments: vec![file(kind, path)],
                ..ReplyInputMessage::default()
            };
            assert_eq!(
                validate_reply_input("RF11", &message),
                accepted(path, ReplyInputKind::Attachment)
            );
        }
    }

    #[test]
    fn accepts_any_input_location_as_latitude_and_longitude_text() {
        assert_eq!(
            validate_reply_input(
                "RF11",
                &location(json!({ "latitude": 10.5, "longitude": 106.75 }))
            ),
            accepted("10.5,106.75", ReplyInputKind::Location)
        );
    }

    #[test]
    fn rejects_empty_any_input_messages() {
        assert_eq!(
            validate_reply_input("RF11", &ReplyInputMessage::default()),
            rejected("getUserData: expected text input")
        );
    }
}

mod reply_input_combinators {
    use super::*;

    fn fixed(result: ReplyValidation) -> ReplyValidator {
        Box::new(move |_| result.clone())
    }

    #[test]
    fn first_accepted_returns_the_first_accepted_result() {
        let validate = first_accepted(vec![
            fixed(rejected("first rejected")),
            fixed(accepted("second accepted", TEXT)),
            fixed(accepted("third accepted", TEXT)),
        ]);
        assert_eq!(
            validate(&ReplyInputMessage::default()),
            accepted("second accepted", TEXT)
        );
    }

    #[test]
    fn first_accepted_returns_the_last_rejection_when_all_validators_reject() {
        let validate = first_accepted(vec![
            fixed(rejected("first rejected")),
            fixed(rejected("second rejected")),
        ]);
        assert_eq!(
            validate(&ReplyInputMessage::default()),
            rejected("second rejected")
        );
    }

    #[test]
    fn from_attachment_rejects_unsupported_file_types() {
        let message = ReplyInputMessage {
            attachments: vec![file("video", "/video.mp4")],
            ..ReplyInputMessage::default()
        };
        assert_eq!(
            from_attachment(|t| t == "image")(&message),
            rejected("getUserData: unsupported file type")
        );
    }

    #[test]
    fn from_location_accepts_alternate_latitude_and_longitude_attribute_names() {
        assert_eq!(
            from_location(&location(json!({ "lat": "21.0278", "long": "105.8342" }))),
            accepted("21.0278,105.8342", ReplyInputKind::Location)
        );
    }

    #[test]
    fn from_location_rejects_non_finite_coordinates() {
        assert_eq!(
            from_location(&location(json!({ "latitude": "NaN", "longitude": 106.75 }))),
            rejected("getUserData: invalid location")
        );
    }
}
