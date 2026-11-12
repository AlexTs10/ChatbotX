//! Port of `packages/utils/__tests__/json-path.test.ts`. `undefined` is
//! `None`; object keys iterate in `serde_json` map order (sorted).

use chatbotx_core::utils::json_path::{
    JsonPathEntry, JsonPathSegment, canonicalize_json_path, format_json_path, is_jsonb_storable,
    is_jsonb_storable_json, list_json_path_entries, parse_json_path, read_json_path,
    stringify_json_path_value,
};
use serde_json::{Value, json};

fn k(key: &str) -> JsonPathSegment {
    JsonPathSegment::Key(key.to_owned())
}

fn i(index: u64) -> JsonPathSegment {
    JsonPathSegment::Index(index)
}

fn payload() -> Value {
    json!({
        "customer": { "email": "john@example.com", "first-name": "John", "a.b": "dot" },
        "order": { "total": 125.5, "items": [{ "name": "Headphones" }, { "name": "Case" }] },
        "nothing": null,
    })
}

fn nested(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses $.customer.email"
#[test]
fn parsejsonpath_parses_customer_email() {
    assert_eq!(
        parse_json_path("$.customer.email"),
        Some(vec![k("customer"), k("email")])
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses customer.email"
#[test]
fn parsejsonpath_parses_customer_email_2() {
    assert_eq!(
        parse_json_path("customer.email"),
        Some(vec![k("customer"), k("email")])
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses $.order.items[0].name"
#[test]
fn parsejsonpath_parses_order_items_0_name() {
    assert_eq!(
        parse_json_path("$.order.items[0].name"),
        Some(vec![k("order"), k("items"), i(0), k("name")])
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses $["customer"]["first-name"]"
#[test]
fn parsejsonpath_parses_customer_first_name() {
    assert_eq!(
        parse_json_path("$[\"customer\"][\"first-name\"]"),
        Some(vec![k("customer"), k("first-name")])
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses $['a.b']"
#[test]
fn parsejsonpath_parses_a_b() {
    assert_eq!(parse_json_path("$['a.b']"), Some(vec![k("a.b")]));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses $"
#[test]
fn parsejsonpath_parses() {
    assert_eq!(parse_json_path("$"), Some(vec![]));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses $type.x"
#[test]
fn parsejsonpath_parses_type_x() {
    assert_eq!(parse_json_path("$type.x"), Some(vec![k("$type"), k("x")]));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses $a"
#[test]
fn parsejsonpath_parses_a() {
    assert_eq!(parse_json_path("$a"), Some(vec![k("$a")]));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:19 "parseJsonPath > parses a\.b.c"
#[test]
fn parsejsonpath_parses_a_b_c() {
    assert_eq!(parse_json_path("a\\.b.c"), Some(vec![k("a.b"), k("c")]));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax """
#[test]
fn parsejsonpath_rejects_unsupported_syntax() {
    assert_eq!(parse_json_path(""), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "   ""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_2() {
    assert_eq!(parse_json_path("   "), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "$.""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_3() {
    assert_eq!(parse_json_path("$."), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "$..a""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_a() {
    assert_eq!(parse_json_path("$..a"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "$.a[""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_a_2() {
    assert_eq!(parse_json_path("$.a["), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "$.a[abc]""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_a_abc() {
    assert_eq!(parse_json_path("$.a[abc]"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "$[\"a\"""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_a_3() {
    assert_eq!(parse_json_path("$[\"a\""), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "$.a[*]""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_a_4() {
    assert_eq!(parse_json_path("$.a[*]"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:33 "parseJsonPath > rejects unsupported syntax "$.items[?(@.id==1)]""
#[test]
fn parsejsonpath_rejects_unsupported_syntax_items_id_1() {
    assert_eq!(parse_json_path("$.items[?(@.id==1)]"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:49 "parseJsonPath round trip > formatting $['a.b'].c keeps the same segments"
#[test]
fn parsejsonpath_round_trip_formatting_a_b_c_keeps_the_same_segments() {
    let segments = parse_json_path("$['a.b'].c");
    assert!(segments.is_some());
    assert_eq!(
        parse_json_path(&format_json_path(&segments.clone().unwrap_or_default())),
        segments
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:49 "parseJsonPath round trip > formatting $['x[0]'] keeps the same segments"
#[test]
fn parsejsonpath_round_trip_formatting_x_0_keeps_the_same_segments() {
    let segments = parse_json_path("$['x[0]']");
    assert!(segments.is_some());
    assert_eq!(
        parse_json_path(&format_json_path(&segments.clone().unwrap_or_default())),
        segments
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:49 "parseJsonPath round trip > formatting $['back\slash'].k keeps the same segments"
#[test]
fn parsejsonpath_round_trip_formatting_back_slash_k_keeps_the_same_segments() {
    let segments = parse_json_path("$['back\\slash'].k");
    assert!(segments.is_some());
    assert_eq!(
        parse_json_path(&format_json_path(&segments.clone().unwrap_or_default())),
        segments
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:49 "parseJsonPath round trip > formatting $.order.items[2].name keeps the same segments"
#[test]
fn parsejsonpath_round_trip_formatting_order_items_2_name_keeps_the_same_segments() {
    let segments = parse_json_path("$.order.items[2].name");
    assert!(segments.is_some());
    assert_eq!(
        parse_json_path(&format_json_path(&segments.clone().unwrap_or_default())),
        segments
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:49 "parseJsonPath round trip > formatting $['$'].id keeps the same segments"
#[test]
fn parsejsonpath_round_trip_formatting_id_keeps_the_same_segments() {
    let segments = parse_json_path("$['$'].id");
    assert!(segments.is_some());
    assert_eq!(
        parse_json_path(&format_json_path(&segments.clone().unwrap_or_default())),
        segments
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:49 "parseJsonPath round trip > formatting $['$'] keeps the same segments"
#[test]
fn parsejsonpath_round_trip_formatting_keeps_the_same_segments() {
    let segments = parse_json_path("$['$']");
    assert!(segments.is_some());
    assert_eq!(
        parse_json_path(&format_json_path(&segments.clone().unwrap_or_default())),
        segments
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:49 "parseJsonPath round trip > formatting a\.b.c keeps the same segments"
#[test]
fn parsejsonpath_round_trip_formatting_a_b_c_keeps_the_same_segments_2() {
    let segments = parse_json_path("a\\.b.c");
    assert!(segments.is_some());
    assert_eq!(
        parse_json_path(&format_json_path(&segments.clone().unwrap_or_default())),
        segments
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:63 "parseJsonPath round trip > reads a legacy dot-prop escaped path (a\.b) as the key a.b"
#[test]
fn parsejsonpath_round_trip_reads_a_legacy_dot_prop_escaped_path_a_b_as_the_key_a_b() {
    assert_eq!(parse_json_path("a\\.b.c"), Some(vec![k("a.b"), k("c")]));
    assert_eq!(
        read_json_path(&json!({ "a.b": { "c": 1 } }), "a\\.b.c"),
        Some(&json!(1))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:68 "parseJsonPath round trip > reads a top-level $ key written with brackets"
#[test]
fn parsejsonpath_round_trip_reads_a_top_level_key_written_with_brackets() {
    assert_eq!(
        read_json_path(&json!({ "$": { "id": 7 } }), "$['$'].id"),
        Some(&json!(7))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:72 "parseJsonPath round trip > keeps an empty quoted key, which has a canonical form"
#[test]
fn parsejsonpath_round_trip_keeps_an_empty_quoted_key_which_has_a_canonical_form() {
    assert_eq!(canonicalize_json_path("$['']").as_deref(), Some("$[\"\"]"));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:76 "parseJsonPath round trip > rejects an index that cannot be rewritten losslessly: $[1000000000000000000000]"
#[test]
fn parsejsonpath_round_trip_rejects_an_index_that_cannot_be_rewritten_losslessly_1000000000000000000000()
 {
    assert_eq!(parse_json_path("$[1000000000000000000000]"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:76 "parseJsonPath round trip > rejects an index that cannot be rewritten losslessly: $.a[12345678901234567890]"
#[test]
fn parsejsonpath_round_trip_rejects_an_index_that_cannot_be_rewritten_losslessly_a_12345678901234567890()
 {
    assert_eq!(parse_json_path("$.a[12345678901234567890]"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:76 "parseJsonPath round trip > rejects an index that cannot be rewritten losslessly: $[01]"
#[test]
fn parsejsonpath_round_trip_rejects_an_index_that_cannot_be_rewritten_losslessly_01() {
    assert_eq!(parse_json_path("$[01]"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:86 "readJsonPath > reads $.customer.email"
#[test]
fn readjsonpath_reads_customer_email() {
    assert_eq!(
        read_json_path(&payload(), "$.customer.email"),
        Some(&json!("john@example.com"))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:86 "readJsonPath > reads $.order.total"
#[test]
fn readjsonpath_reads_order_total() {
    assert_eq!(
        read_json_path(&payload(), "$.order.total"),
        Some(&json!(125.5))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:86 "readJsonPath > reads $.order.items[1].name"
#[test]
fn readjsonpath_reads_order_items_1_name() {
    assert_eq!(
        read_json_path(&payload(), "$.order.items[1].name"),
        Some(&json!("Case"))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:86 "readJsonPath > reads $.customer["first-name"]"
#[test]
fn readjsonpath_reads_customer_first_name() {
    assert_eq!(
        read_json_path(&payload(), "$.customer[\"first-name\"]"),
        Some(&json!("John"))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:86 "readJsonPath > reads $.customer['a.b']"
#[test]
fn readjsonpath_reads_customer_a_b() {
    assert_eq!(
        read_json_path(&payload(), "$.customer['a.b']"),
        Some(&json!("dot"))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:96 "readJsonPath > returns the root for $"
#[test]
fn readjsonpath_returns_the_root_for() {
    let root = payload();
    assert!(std::ptr::eq(read_json_path(&root, "$").unwrap(), &root));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:100 "readJsonPath > returns undefined for missing paths, invalid paths and non-object payloads"
#[test]
fn readjsonpath_returns_undefined_for_missing_paths_invalid_paths_and_non_object_payloads() {
    assert_eq!(read_json_path(&payload(), "$.customer.phone"), None);
    assert_eq!(read_json_path(&payload(), "$.order.items[9].name"), None);
    assert_eq!(read_json_path(&payload(), "$.a["), None);
    assert_eq!(read_json_path(&json!("text"), "$.a"), None);
    assert_eq!(read_json_path(&Value::Null, "$.a"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:108 "readJsonPath > does not walk the prototype chain or read built-in members"
#[test]
fn readjsonpath_does_not_walk_the_prototype_chain_or_read_built_in_members() {
    for path in [
        "$.__proto__.constructor",
        "$.constructor.name",
        "$.customer.toString",
        "$.customer.hasOwnProperty",
        "$.customer.email.length",
        "$.order.items.length",
    ] {
        assert_eq!(read_json_path(&payload(), path), None, "{path}");
    }
}

/// Port of: packages/utils/__tests__/json-path.test.ts:117 "readJsonPath > reads the escaped dot-prop form the builder JSON picker emits"
#[test]
fn readjsonpath_reads_the_escaped_dot_prop_form_the_builder_json_picker_emits() {
    let keyed = json!({ "a.b": { "x[0]": "ok", "back": { "c\\d": "slash" } } });
    assert_eq!(read_json_path(&keyed, "a\\.b.x\\[0]"), Some(&json!("ok")));
    assert_eq!(
        read_json_path(&keyed, "a\\.b.back.c\\\\d"),
        Some(&json!("slash"))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:123 "readJsonPath > reads keys that start with a dollar sign"
#[test]
fn readjsonpath_reads_keys_that_start_with_a_dollar_sign() {
    assert_eq!(
        read_json_path(&json!({ "$type": "t", "$": { "id": 1 } }), "$type"),
        Some(&json!("t"))
    );
    assert_eq!(
        read_json_path(&json!({ "$type": "t" }), "$.$type"),
        Some(&json!("t"))
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:130 "stringifyJsonPathValue > keeps strings, JSON-encodes the rest, skips null and undefined"
#[test]
fn stringifyjsonpathvalue_keeps_strings_json_encodes_the_rest_skips_null_and_undefined() {
    assert_eq!(
        stringify_json_path_value(Some(&json!("a"))).as_deref(),
        Some("a")
    );
    assert_eq!(
        stringify_json_path_value(Some(&json!(125.5))).as_deref(),
        Some("125.5")
    );
    assert_eq!(
        stringify_json_path_value(Some(&json!(false))).as_deref(),
        Some("false")
    );
    assert_eq!(
        stringify_json_path_value(Some(&json!([1, 2]))).as_deref(),
        Some("[1,2]")
    );
    assert_eq!(stringify_json_path_value(Some(&Value::Null)), None);
    assert_eq!(stringify_json_path_value(None), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:144 "isJsonbStorable > accepts ordinary documents, accented text and emoji"
#[test]
fn isjsonbstorable_accepts_ordinary_documents_accented_text_and_emoji() {
    assert!(is_jsonb_storable(
        &json!({ "a": [1, "Nguyễn 😀", null, { "b": true }] })
    ));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:150 "isJsonbStorable > refuses a NUL in a value, a key, an array item or a top-level string"
#[test]
fn isjsonbstorable_refuses_a_nul_in_a_value_a_key_an_array_item_or_a_top_level_string() {
    assert!(!is_jsonb_storable(&json!({ "a": "x\u{0}" })));
    assert!(!is_jsonb_storable(&json!({ "\u{0}": 1 })));
    assert!(!is_jsonb_storable(&json!(["ok", ["\u{0}"]])));
    assert!(!is_jsonb_storable(&json!("\u{0}")));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:157 "isJsonbStorable > refuses a lone surrogate"
#[test]
fn isjsonbstorable_refuses_a_lone_surrogate() {
    // A Rust string cannot hold "\ud800"; the JSON text carrying it is refused.
    assert!(!is_jsonb_storable_json(r#"{"a":"\ud800"}"#));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:161 "isJsonbStorable > allows nesting up to 64 levels and refuses 65"
#[test]
fn isjsonbstorable_allows_nesting_up_to_64_levels_and_refuses_65() {
    assert!(is_jsonb_storable(
        &serde_json::from_str(&nested(64)).unwrap()
    ));
    assert!(!is_jsonb_storable(
        &serde_json::from_str(&nested(65)).unwrap()
    ));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes [] as $"
#[test]
fn formatjsonpath_writes_as() {
    assert_eq!(format_json_path(&vec![]), "$");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["customer", "email"] as $.customer.email"
#[test]
fn formatjsonpath_writes_customer_email_as_customer_email() {
    assert_eq!(
        format_json_path(&vec![k("customer"), k("email")]),
        "$.customer.email"
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["items", 0, "name"] as $.items[0].name"
#[test]
fn formatjsonpath_writes_items_0_name_as_items_0_name() {
    assert_eq!(
        format_json_path(&vec![k("items"), i(0), k("name")]),
        "$.items[0].name"
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["customer-data", "email"] as $["customer-data"].email"
#[test]
fn formatjsonpath_writes_customer_data_email_as_customer_data_email() {
    assert_eq!(
        format_json_path(&vec![k("customer-data"), k("email")]),
        "$[\"customer-data\"].email"
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["a.b"] as $["a.b"]"
#[test]
fn formatjsonpath_writes_a_b_as_a_b() {
    assert_eq!(format_json_path(&vec![k("a.b")]), "$[\"a.b\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["has space"] as $["has space"]"
#[test]
fn formatjsonpath_writes_has_space_as_has_space() {
    assert_eq!(format_json_path(&vec![k("has space")]), "$[\"has space\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["$type"] as $["$type"]"
#[test]
fn formatjsonpath_writes_type_as_type() {
    assert_eq!(format_json_path(&vec![k("$type")]), "$[\"$type\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["9lives"] as $["9lives"]"
#[test]
fn formatjsonpath_writes_9lives_as_9lives() {
    assert_eq!(format_json_path(&vec![k("9lives")]), "$[\"9lives\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["007"] as $["007"]"
#[test]
fn formatjsonpath_writes_007_as_007() {
    assert_eq!(format_json_path(&vec![k("007")]), "$[\"007\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["12"] as $[12]"
#[test]
fn formatjsonpath_writes_12_as_12() {
    assert_eq!(format_json_path(&vec![k("12")]), "$[12]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes [""] as $[""]"
#[test]
fn formatjsonpath_writes_as_2() {
    assert_eq!(format_json_path(&vec![k("")]), "$[\"\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["say \"hi\""] as $["say \"hi\""]"
#[test]
fn formatjsonpath_writes_say_hi_as_say_hi() {
    assert_eq!(
        format_json_path(&vec![k("say \"hi\"")]),
        "$[\"say \\\"hi\\\"\"]"
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["back\\slash"] as $["back\\slash"]"
#[test]
fn formatjsonpath_writes_back_slash_as_back_slash() {
    assert_eq!(
        format_json_path(&vec![k("back\\slash")]),
        "$[\"back\\\\slash\"]"
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["a]b"] as $["a]b"]"
#[test]
fn formatjsonpath_writes_a_b_as_a_b_2() {
    assert_eq!(format_json_path(&vec![k("a]b")]), "$[\"a]b\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:168 "formatJsonPath > writes ["new\nline"] as $["new\nline"]"
#[test]
fn formatjsonpath_writes_new_nline_as_new_nline() {
    assert_eq!(format_json_path(&vec![k("new\nline")]), "$[\"new\\nline\"]");
}

/// Port of: packages/utils/__tests__/json-path.test.ts:188 "formatJsonPath > reads back to the same segments for awkward keys"
#[test]
fn formatjsonpath_reads_back_to_the_same_segments_for_awkward_keys() {
    for key in [
        "", "a.b", "a]b", "\"", "'", "\\", "\\\"", "\n", "$", "é", "007",
    ] {
        let segments = vec![k(key), i(1), k(key)];
        assert_eq!(
            parse_json_path(&format_json_path(&segments)),
            Some(segments),
            "{key:?}"
        );
    }
}

/// Port of: packages/utils/__tests__/json-path.test.ts:213 "canonicalizeJsonPath > rewrites every accepted form to the $ form"
#[test]
fn canonicalizejsonpath_rewrites_every_accepted_form_to_the_form() {
    assert_eq!(
        canonicalize_json_path("customer.phone").as_deref(),
        Some("$.customer.phone")
    );
    assert_eq!(
        canonicalize_json_path("items.0.name").as_deref(),
        Some("$.items[0].name")
    );
    assert_eq!(
        canonicalize_json_path("$['a-b'].c").as_deref(),
        Some("$[\"a-b\"].c")
    );
    assert_eq!(canonicalize_json_path("$.a.b").as_deref(), Some("$.a.b"));
    assert_eq!(canonicalize_json_path("$").as_deref(), Some("$"));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:221 "canonicalizeJsonPath > returns null for input that does not parse"
#[test]
fn canonicalizejsonpath_returns_null_for_input_that_does_not_parse() {
    assert_eq!(canonicalize_json_path(""), None);
    assert_eq!(canonicalize_json_path("$.a[*]"), None);
    assert_eq!(canonicalize_json_path("$[\"unterminated"), None);
    assert_eq!(canonicalize_json_path("$[\"bad\\q\"]"), None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:230 "parseJsonPath escapes > reads escaped quotes and brackets inside a double-quoted key"
#[test]
fn parsejsonpath_escapes_reads_escaped_quotes_and_brackets_inside_a_double_quoted_key() {
    assert_eq!(parse_json_path("$[\"a\\\"b\"]"), Some(vec![k("a\"b")]));
    assert_eq!(
        parse_json_path("$[\"a]b\"].c"),
        Some(vec![k("a]b"), k("c")])
    );
    assert_eq!(parse_json_path("$[\"\"]"), Some(vec![k("")]));
}

/// Port of: packages/utils/__tests__/json-path.test.ts:240 "listJsonPathEntries > lists every node in document order with a $ path and a preview"
#[test]
fn listjsonpathentries_lists_every_node_in_document_order_with_a_path_and_a_preview() {
    let sample = json!({ "customer": { "email": "a@b.co", "tags": ["x", "y"] }, "n": 3 });
    let listing = list_json_path_entries(&sample, None, None);
    assert!(!listing.truncated);
    let paths: Vec<&str> = listing.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "$",
            "$.customer",
            "$.customer.email",
            "$.customer.tags",
            "$.customer.tags[0]",
            "$.customer.tags[1]",
            "$.n",
        ]
    );
    assert_eq!(listing.entries[2].preview.as_deref(), Some("a@b.co"));
    assert_eq!(listing.entries[1].preview, None);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:256 "listJsonPathEntries > stops at the entry cap and says so"
#[test]
fn listjsonpathentries_stops_at_the_entry_cap_and_says_so() {
    let wide: serde_json::Map<String, Value> = (0..50)
        .map(|index| (format!("k{index}"), json!(index)))
        .collect();
    let listing = list_json_path_entries(&Value::Object(wide), Some(10), None);
    assert_eq!(listing.entries.len(), 10);
    assert!(listing.truncated);
}

/// Port of: packages/utils/__tests__/json-path.test.ts:265 "listJsonPathEntries > truncates long previews"
#[test]
fn listjsonpathentries_truncates_long_previews() {
    let listing = list_json_path_entries(&json!({ "text": "x".repeat(500) }), None, Some(20));
    assert_eq!(
        listing.entries[1].preview.as_deref(),
        Some(format!("{}…", "x".repeat(20)).as_str())
    );
}

/// Port of: packages/utils/__tests__/json-path.test.ts:273 "listJsonPathEntries > handles a primitive root"
#[test]
fn listjsonpathentries_handles_a_primitive_root() {
    assert_eq!(
        list_json_path_entries(&json!(5), None, None).entries,
        vec![JsonPathEntry {
            segments: vec![],
            path: "$".to_owned(),
            preview: Some("5".to_owned()),
        }]
    );
}
