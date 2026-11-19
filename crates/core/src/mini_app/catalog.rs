//! `components/catalog.ts`, `fields.ts`, `limits.ts`, `placement.ts`: the
//! component catalog with Meta's Flow JSON 7.3 props schemas.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::runtime::{MAX_UPLOAD_BYTES, MAX_UPLOADS_PER_INPUT};
use crate::flow_config::z::{self, Schema};

pub const MAX_IMAGE_BASE64_LENGTH: usize = 410_000;
pub const MAX_OPTION_IMAGE_BASE64_LENGTH: usize = 137_000;
pub const MAX_OPTIONS: usize = 20;
pub const MAX_DROPDOWN_OPTIONS: usize = 200;
pub const MAX_DROPDOWN_OPTIONS_WITH_IMAGES: usize = 100;
pub const MAX_NAVIGATION_ITEMS: usize = 20;
pub const MAX_FILE_SIZE_KB: usize = MAX_UPLOAD_BYTES / 1024;
pub const MAX_COMPONENTS_PER_SCREEN: usize = 50;
pub const MAX_IF_DEPTH: usize = 3;

/// `MiniAppComponentDefinition` (no default props: the editor makes nodes).
pub struct Component {
    pub kind: &'static str,
    pub is_input: bool,
    pub max_per_screen: Option<usize>,
    pub actions: &'static [&'static str],
    pub action_required: bool,
    pub props: Schema,
}

fn required_text(max: usize) -> Schema {
    z::string().trim().min(1).max(max)
}
fn optional_text(max: usize) -> Schema {
    required_text(max).optional()
}
fn base64_image(max: usize) -> Schema {
    z::string().max(max).regex("^[A-Za-z0-9+/]+={0,2}$", "")
}
fn iso_date() -> Schema {
    z::string().regex(r"^\d{4}-\d{2}-\d{2}$", "")
}
fn int(min: f64, max: Option<f64>) -> Schema {
    let schema = z::number().int().gte(min);
    match max {
        Some(max) => schema.lte(max),
        None => schema,
    }
}
fn boolean() -> Schema {
    z::boolean().optional()
}
fn media_size() -> Schema {
    z::enum_(&["regular", "large"]).optional()
}
fn scale_type() -> Schema {
    z::enum_(&["cover", "contain"]).optional()
}
fn aspect_ratio() -> Schema {
    z::number().positive().lte(10.0).optional()
}

fn option_schema() -> Schema {
    z::strict_object(vec![
        ("id", required_text(80)),
        ("title", required_text(30)),
        ("description", optional_text(300)),
        ("metadata", optional_text(20)),
        ("enabled", boolean()),
        (
            "image",
            base64_image(MAX_OPTION_IMAGE_BASE64_LENGTH).optional(),
        ),
        ("alt-text", optional_text(100)),
        (
            "color",
            z::string().regex("^#[0-9A-Fa-f]{6}$", "").optional(),
        ),
    ])
}

/// `dataSource`: options with unique ids.
fn data_source(max: usize, min: usize) -> Schema {
    z::array(option_schema()).min(min).max(max).refine(
        |items| {
            let ids: Vec<&Value> = items
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| item.get("id").unwrap_or(&Value::Null))
                .collect();
            let unique: std::collections::HashSet<String> =
                ids.iter().map(|v| v.to_string()).collect();
            unique.len() == ids.len()
        },
        Some("duplicate_option_id"),
        None,
    )
}

fn navigation_item_schema() -> Schema {
    z::strict_object(vec![
        ("id", required_text(80)),
        (
            "main-content",
            z::strict_object(vec![
                ("title", required_text(30)),
                ("description", optional_text(20)),
                ("metadata", optional_text(80)),
            ]),
        ),
        (
            "start",
            z::strict_object(vec![
                ("image", base64_image(MAX_OPTION_IMAGE_BASE64_LENGTH)),
                ("alt-text", optional_text(100)),
            ])
            .optional(),
        ),
        (
            "end",
            z::strict_object(vec![
                ("title", optional_text(10)),
                ("description", optional_text(10)),
                ("metadata", optional_text(10)),
            ])
            .optional(),
        ),
        ("badge", optional_text(15)),
        ("tags", z::array(required_text(15)).max(3).optional()),
        ("on-click-action", z::unknown()),
    ])
}

type Shape = Vec<(&'static str, Schema)>;

/// `inputBase` minus the keys in `without`.
fn input_base(without: &[&str]) -> Shape {
    [
        ("name", z::string()),
        ("required", boolean()),
        ("enabled", boolean()),
        ("visible", boolean()),
    ]
    .into_iter()
    .filter(|(key, _)| !without.contains(key))
    .collect()
}

fn with(mut base: Shape, rest: Shape) -> Schema {
    base.extend(rest);
    z::strict_object(base)
}

fn text_props(max: usize) -> Schema {
    z::strict_object(vec![
        ("text", required_text(max)),
        (
            "font-weight",
            z::enum_(&["bold", "italic", "bold_italic", "normal"]).optional(),
        ),
        ("strikethrough", boolean()),
        ("markdown", boolean()),
        ("visible", boolean()),
    ])
}

fn selected_items() -> Shape {
    vec![
        (
            "min-selected-items",
            int(0.0, Some(MAX_OPTIONS as f64)).optional(),
        ),
        (
            "max-selected-items",
            int(1.0, Some(MAX_OPTIONS as f64)).optional(),
        ),
    ]
}

fn uploaded(min: f64) -> Schema {
    int(min, Some(MAX_UPLOADS_PER_INPUT as f64)).optional()
}

fn max_file_size_kb() -> Schema {
    int(1.0, Some(MAX_FILE_SIZE_KB as f64)).optional()
}

fn plain(kind: &'static str, props: Schema) -> Component {
    Component {
        kind,
        is_input: false,
        max_per_screen: None,
        actions: &[],
        action_required: false,
        props,
    }
}
fn input(kind: &'static str, props: Schema) -> Component {
    Component {
        is_input: true,
        ..plain(kind, props)
    }
}

fn build() -> Vec<Component> {
    let visible = || ("visible", boolean());
    let date_extra = |label: usize, err: usize| -> Shape {
        vec![
            ("label", required_text(label)),
            ("helper-text", optional_text(80)),
            ("min-date", iso_date().optional()),
            ("max-date", iso_date().optional()),
            (
                "unavailable-dates",
                z::array(iso_date()).max(366).optional(),
            ),
            ("init-value", iso_date().optional()),
            ("error-message", optional_text(err)),
        ]
    };
    vec![
        plain(
            "TextHeading",
            z::strict_object(vec![("text", required_text(80)), visible()]),
        ),
        plain(
            "TextSubheading",
            z::strict_object(vec![("text", required_text(80)), visible()]),
        ),
        plain("TextBody", text_props(4096)),
        plain("TextCaption", text_props(409)),
        plain(
            "RichText",
            z::strict_object(vec![("text", required_text(10_000)), visible()]),
        ),
        input(
            "TextInput",
            with(
                input_base(&["enabled"]),
                vec![
                    ("label", required_text(20)),
                    ("label-variant", z::enum_(&["large"]).optional()),
                    (
                        "input-type",
                        z::enum_(&[
                            "text", "number", "email", "password", "passcode", "phone", "date",
                            "datetime",
                        ])
                        .optional(),
                    ),
                    ("pattern", optional_text(500)),
                    ("min-chars", int(0.0, Some(80.0)).optional()),
                    ("max-chars", int(1.0, Some(80.0)).optional()),
                    ("helper-text", optional_text(80)),
                    ("init-value", optional_text(80)),
                    ("error-message", optional_text(30)),
                ],
            ),
        ),
        input(
            "TextArea",
            with(
                input_base(&[]),
                vec![
                    ("label", required_text(20)),
                    ("label-variant", z::enum_(&["large"]).optional()),
                    ("max-length", int(1.0, Some(600.0)).optional()),
                    ("helper-text", optional_text(80)),
                    ("init-value", optional_text(600)),
                    ("error-message", optional_text(30)),
                ],
            ),
        ),
        input(
            "Dropdown",
            with(
                input_base(&[]),
                vec![
                    ("label", required_text(20)),
                    ("data-source", data_source(MAX_DROPDOWN_OPTIONS, 1)),
                    ("init-value", optional_text(80)),
                    ("error-message", optional_text(30)),
                ],
            ),
        ),
        input(
            "RadioButtonsGroup",
            with(
                input_base(&[]),
                vec![
                    ("label", required_text(30)),
                    ("description", optional_text(300)),
                    ("data-source", data_source(MAX_OPTIONS, 1)),
                    ("media-size", media_size()),
                    ("init-value", optional_text(80)),
                    ("error-message", optional_text(30)),
                ],
            ),
        ),
        input("CheckboxGroup", {
            let mut rest = vec![
                ("label", required_text(30)),
                ("description", optional_text(300)),
                ("data-source", data_source(MAX_OPTIONS, 1)),
            ];
            rest.extend(selected_items());
            rest.extend([
                ("media-size", media_size()),
                ("init-value", z::array(z::string()).optional()),
                ("error-message", optional_text(30)),
            ]);
            with(input_base(&[]), rest)
        }),
        input("ChipsSelector", {
            let mut rest = vec![
                ("label", required_text(80)),
                ("description", optional_text(300)),
                ("data-source", data_source(MAX_OPTIONS, 2)),
            ];
            rest.extend(selected_items());
            rest.extend([
                ("init-value", z::array(z::string()).optional()),
                ("error-message", optional_text(30)),
            ]);
            with(input_base(&[]), rest)
        }),
        input("DatePicker", with(input_base(&[]), date_extra(40, 80))),
        input(
            "CalendarPicker",
            with(
                input_base(&[]),
                vec![
                    ("label", required_text(40)),
                    ("title", optional_text(80)),
                    ("description", optional_text(300)),
                    ("helper-text", optional_text(80)),
                    ("mode", z::enum_(&["single", "range"]).optional()),
                    ("min-date", iso_date().optional()),
                    ("max-date", iso_date().optional()),
                    (
                        "unavailable-dates",
                        z::array(iso_date()).max(366).optional(),
                    ),
                    (
                        "include-days",
                        z::array(z::enum_(&["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]))
                            .optional(),
                    ),
                    ("min-days", int(1.0, None).optional()),
                    ("max-days", int(1.0, None).optional()),
                    ("error-message", optional_text(80)),
                ],
            ),
        ),
        Component {
            max_per_screen: Some(5),
            actions: &["navigate", "open_url"],
            ..input(
                "OptIn",
                with(
                    input_base(&["enabled"]),
                    vec![("label", required_text(120)), ("init-value", boolean())],
                ),
            )
        },
        Component {
            max_per_screen: Some(1),
            ..input(
                "PhotoPicker",
                with(
                    input_base(&["required"]),
                    vec![
                        ("label", required_text(80)),
                        ("description", optional_text(300)),
                        (
                            "photo-source",
                            z::enum_(&["camera_gallery", "camera", "gallery"]).optional(),
                        ),
                        ("max-file-size-kb", max_file_size_kb()),
                        ("min-uploaded-photos", uploaded(0.0)),
                        ("max-uploaded-photos", uploaded(1.0)),
                        ("error-message", optional_text(80)),
                    ],
                ),
            )
        },
        Component {
            max_per_screen: Some(1),
            ..input(
                "DocumentPicker",
                with(
                    input_base(&["required"]),
                    vec![
                        ("label", required_text(80)),
                        ("description", optional_text(300)),
                        ("max-file-size-kb", max_file_size_kb()),
                        ("min-uploaded-documents", uploaded(0.0)),
                        ("max-uploaded-documents", uploaded(1.0)),
                        (
                            "allowed-mime-types",
                            z::array(required_text(120)).max(30).optional(),
                        ),
                        ("error-message", optional_text(80)),
                    ],
                ),
            )
        },
        Component {
            max_per_screen: Some(3),
            ..plain(
                "Image",
                z::strict_object(vec![
                    ("src", base64_image(MAX_IMAGE_BASE64_LENGTH)),
                    ("width", int(1.0, Some(2000.0)).optional()),
                    ("height", int(1.0, Some(2000.0)).optional()),
                    ("scale-type", scale_type()),
                    ("aspect-ratio", aspect_ratio()),
                    ("alt-text", optional_text(100)),
                    visible(),
                ]),
            )
        },
        Component {
            max_per_screen: Some(2),
            ..plain(
                "ImageCarousel",
                z::strict_object(vec![
                    (
                        "images",
                        z::array(z::object(vec![
                            ("src", base64_image(MAX_IMAGE_BASE64_LENGTH)),
                            ("alt-text", optional_text(100)),
                        ]))
                        .min(1)
                        .max(3),
                    ),
                    ("scale-type", scale_type()),
                    ("aspect-ratio", aspect_ratio()),
                    visible(),
                ]),
            )
        },
        Component {
            max_per_screen: Some(2),
            actions: &["navigate", "open_url"],
            action_required: true,
            ..plain(
                "EmbeddedLink",
                z::strict_object(vec![("text", required_text(25)), visible()]),
            )
        },
        Component {
            max_per_screen: Some(2),
            ..plain(
                "NavigationList",
                z::strict_object(vec![
                    ("name", z::string()),
                    (
                        "list-items",
                        z::array(navigation_item_schema())
                            .min(1)
                            .max(MAX_NAVIGATION_ITEMS),
                    ),
                    ("media-size", media_size()),
                    visible(),
                ]),
            )
        },
        Component {
            max_per_screen: Some(1),
            actions: &["navigate", "complete"],
            action_required: true,
            ..plain(
                "Footer",
                z::strict_object(vec![
                    ("label", required_text(35)),
                    ("left-caption", optional_text(15)),
                    ("center-caption", optional_text(15)),
                    ("right-caption", optional_text(15)),
                    ("enabled", boolean()),
                ]),
            )
        },
        plain("Form", z::strict_object(vec![("name", z::string())])),
        plain(
            "If",
            z::strict_object(vec![("condition", required_text(500))]),
        ),
        plain(
            "Switch",
            z::strict_object(vec![("value", required_text(200))]),
        ),
    ]
}

fn catalog() -> &'static HashMap<&'static str, Component> {
    static CATALOG: OnceLock<HashMap<&'static str, Component>> = OnceLock::new();
    CATALOG.get_or_init(|| build().into_iter().map(|c| (c.kind, c)).collect())
}

/// `MINI_APP_COMPONENTS[type]`.
pub fn component(kind: &str) -> Option<&'static Component> {
    catalog().get(kind)
}

/// `needsName`: inputs, Forms and NavigationLists carry a `name`.
pub fn needs_name(kind: &str) -> bool {
    component(kind).is_some_and(|c| c.is_input) || kind == "Form" || kind == "NavigationList"
}

const IF_ALLOWED: &[&str] = &[
    "TextHeading",
    "TextSubheading",
    "TextBody",
    "TextCaption",
    "CheckboxGroup",
    "ChipsSelector",
    "DatePicker",
    "Dropdown",
    "EmbeddedLink",
    "Footer",
    "Image",
    "OptIn",
    "RadioButtonsGroup",
    "Switch",
    "TextArea",
    "TextInput",
    "If",
];
const SWITCH_ALLOWED: &[&str] = &[
    "TextHeading",
    "TextSubheading",
    "TextBody",
    "TextCaption",
    "CheckboxGroup",
    "ChipsSelector",
    "DatePicker",
    "Dropdown",
    "EmbeddedLink",
    "Footer",
    "Image",
    "OptIn",
    "RadioButtonsGroup",
    "TextArea",
    "TextInput",
];

/// `canPlaceInSlot`.
pub fn can_place_in_slot(parent: &str, child: &str) -> bool {
    match parent {
        "Form" => !matches!(child, "Form" | "NavigationList"),
        "If" => IF_ALLOWED.contains(&child),
        "Switch" => SWITCH_ALLOWED.contains(&child),
        _ => false,
    }
}

/// `TEXT_INPUT_DATE_FORMATS`: (pattern, helper text).
pub fn text_input_date_format(input_type: &str) -> Option<(&'static str, &'static str)> {
    match input_type {
        "date" => Some((
            r"^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])$",
            "YYYY-MM-DD",
        )),
        "datetime" => Some((
            r"^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])[ T]([01]\d|2[0-3]):[0-5]\d$",
            "YYYY-MM-DD HH:mm",
        )),
        _ => None,
    }
}
