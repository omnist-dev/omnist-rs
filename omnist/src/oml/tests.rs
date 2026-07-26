use super::*;
use crate::document::{Doc, Value};
use indexmap::IndexMap;

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut m = IndexMap::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

fn roundtrip_value(v: &Value) {
    let doc = Doc::of(v).unwrap();
    let raw = doc.to_raw();
    let text = write_oml(&raw, 2).unwrap();
    let parsed = read_oml(&text).unwrap();
    let doc2 = Doc::from_raw(parsed).unwrap();
    assert!(
        doc.eq_doc(&doc2),
        "round trip mismatch for {v:?}\n--- OML ---\n{text}"
    );
}

// -- scalar round trips (every kind) ----------------------------------------

#[test]
fn string_round_trips() {
    roundtrip_value(&obj(&[("a", Value::Str("hello world".into()))]));
}

#[test]
fn string_with_escapes_round_trips() {
    roundtrip_value(&obj(&[(
        "a",
        Value::Str("quote:\" backslash:\\ nl:\n cr:\r tab:\t ctrl:\u{1}".into()),
    )]));
}

#[test]
fn integer_round_trips_positive_and_negative() {
    roundtrip_value(&obj(&[
        ("a", Value::Int(42)),
        ("b", Value::Int(-42)),
        ("c", Value::Int(0)),
    ]));
}

#[test]
fn number_round_trips_decimal_and_whole_valued_float() {
    roundtrip_value(&obj(&[
        ("a", Value::Float(3.15)),
        ("b", Value::Float(1.0)),
        ("c", Value::Float(-2.0)),
        ("d", Value::Float(1e10)),
    ]));
}

#[test]
fn number_round_trips_nan_and_infinities() {
    let doc = Doc::of(&obj(&[
        ("a", Value::Float(f64::NAN)),
        ("b", Value::Float(f64::INFINITY)),
        ("c", Value::Float(f64::NEG_INFINITY)),
    ]))
    .unwrap();
    let text = write_oml(&doc.to_raw(), 2).unwrap();
    assert!(text.contains("a: nan"));
    assert!(text.contains("b: inf"));
    assert!(text.contains("c: -inf"));
    let parsed = Doc::from_raw(read_oml(&text).unwrap()).unwrap();
    let root = parsed.root();
    assert!(matches!(
        root.child("a").unwrap().value().unwrap(),
        crate::document::Scalar::Float(f) if f.is_nan()
    ));
    assert!(matches!(
        root.child("b").unwrap().value().unwrap(),
        crate::document::Scalar::Float(f) if f.is_infinite() && *f > 0.0
    ));
    assert!(matches!(
        root.child("c").unwrap().value().unwrap(),
        crate::document::Scalar::Float(f) if f.is_infinite() && *f < 0.0
    ));
}

#[test]
fn boolean_round_trips() {
    roundtrip_value(&obj(&[("a", Value::Bool(true)), ("b", Value::Bool(false))]));
}

#[test]
fn null_round_trips() {
    roundtrip_value(&obj(&[("a", Value::Null)]));
}

#[test]
fn date_round_trips() {
    roundtrip_value(&obj(&[("a", Value::Str("2024-01-01".into()))]));
}

#[test]
fn time_round_trips_with_and_without_seconds_and_fraction() {
    roundtrip_value(&obj(&[
        ("a", Value::Str("12:00:00".into())),
        ("b", Value::Str("12:00".into())),
        ("c", Value::Str("12:00:00.123456".into())),
    ]));
}

#[test]
fn datetime_round_trips_with_and_without_utc_offset() {
    roundtrip_value(&obj(&[
        ("a", Value::Str("2024-01-01T12:00:00".into())),
        ("b", Value::Str("2024-01-01T12:00:00+02:00".into())),
        ("c", Value::Str("2024-01-01T12:00:00-05:30".into())),
    ]));
}

#[test]
fn nested_object_round_trips() {
    roundtrip_value(&obj(&[(
        "a",
        obj(&[("b", Value::Int(1)), ("c", Value::Str("x".into()))]),
    )]));
}

#[test]
fn repeated_label_contiguous_round_trips() {
    roundtrip_value(&obj(&[(
        "tag",
        Value::Array(vec![Value::Str("x".into()), Value::Str("y".into())]),
    )]));
}

#[test]
fn empty_document_round_trips() {
    let doc = Doc::from_raw(RawNode::Edges(vec![])).unwrap();
    let text = write_oml(&doc.to_raw(), 2).unwrap();
    assert_eq!(text, "");
    let parsed = Doc::from_raw(read_oml(&text).unwrap()).unwrap();
    assert!(doc.eq_doc(&parsed));
}

#[test]
fn bare_top_level_scalar_round_trips() {
    let doc = Doc::of(&Value::Int(7)).unwrap();
    let text = write_oml(&doc.to_raw(), 2).unwrap();
    assert_eq!(text, "7");
    let parsed = Doc::from_raw(read_oml(&text).unwrap()).unwrap();
    assert!(doc.eq_doc(&parsed));
}

// -- interleaved repeated labels: the case Value/IndexMap cannot represent --

#[test]
fn interleaved_repeated_labels_round_trip_exactly_via_raw_node() {
    let raw = RawNode::Edges(vec![
        (
            "b".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(1)),
        ),
        (
            "c".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(2)),
        ),
        (
            "b".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(3)),
        ),
    ]);
    let text = write_oml(&raw, 2).unwrap();
    assert_eq!(text, "b: 1\nc: 2\nb: 3");
    let parsed = read_oml(&text).unwrap();
    assert_eq!(parsed, raw);
}

// -- the four TS-bug-derived regressions ------------------------------------

// omnist-ts#37 / #70: write_oml must call the shared depth guard itself.
#[test]
fn write_oml_calls_the_shared_depth_guard() {
    fn nest_raw(levels: usize) -> RawNode {
        let mut n = RawNode::Leaf(crate::document::Scalar::Int(0));
        for _ in 0..levels {
            n = RawNode::Edges(vec![("a".to_string(), n)]);
        }
        n
    }
    // At exactly MAX_DEPTH it's accepted (boundary, matches document.rs's
    // own "reject only depth > MAX_DEPTH" convention).
    assert!(write_oml(&nest_raw(document::MAX_DEPTH), 2).is_ok());
    // One past it, write_oml must reject on its own -- this RawNode was
    // never passed through Doc::from_raw's guarded construction, so if
    // write_oml didn't call check_write_depth itself, this would write
    // an over-deep tree with no error, byte-identical to the omnist-ts
    // bug (a writer trusting an upstream check that never ran).
    let err = write_oml(&nest_raw(document::MAX_DEPTH + 1), 2).unwrap_err();
    assert!(err.to_string().contains("maximum depth"));
    // Same guard, compact writer entry point.
    let err2 = write_oml_compact(&nest_raw(document::MAX_DEPTH + 1)).unwrap_err();
    assert!(err2.to_string().contains("maximum depth"));
}

// omnist-ts#51: writeOml erased a datetime UTC offset.
#[test]
fn write_oml_preserves_datetime_utc_offset_exactly() {
    let raw = RawNode::Edges(vec![(
        "meeting".to_string(),
        RawNode::Leaf(crate::document::Scalar::Str(
            "2024-06-01T09:30:00+02:00".to_string(),
        )),
    )]);
    let text = write_oml(&raw, 2).unwrap();
    assert!(
        text.contains("+02:00"),
        "offset must survive the write unchanged: {text}"
    );
    let parsed = read_oml(&text).unwrap();
    assert_eq!(parsed, raw);
}

// omnist-ts#52: a bare TIME literal did not round-trip (became a quoted
// string on write-then-read).
#[test]
fn bare_time_literal_round_trips_as_a_time_not_a_quoted_string() {
    let raw = RawNode::Edges(vec![(
        "a".to_string(),
        RawNode::Leaf(crate::document::Scalar::Str("12:00".to_string())),
    )]);
    let text = write_oml(&raw, 2).unwrap();
    // Must write as a bare, unquoted time literal, not `"12:00"`.
    assert_eq!(text, "a: 12:00");
    let parsed = read_oml(&text).unwrap();
    assert_eq!(parsed, raw);
}

// omnist-ts#36-equivalent: writer escaping must be all-occurrences, not
// first-match-only (a non-global regex-style replace would under-sanitize).
#[test]
fn string_escaping_is_all_occurrences_not_first_match_only() {
    let s = "a\"b\"c\"d\\e\\f";
    let written = write_scalar(&crate::document::Scalar::Str(s.to_string()));
    assert_eq!(written, r#""a\"b\"c\"d\\e\\f""#);
    // Every quote and every backslash escaped -- not just the first of each.
    assert_eq!(written.matches("\\\"").count(), 3);
    assert_eq!(written.matches("\\\\").count(), 2);
}

// -- malformed input / error positions --------------------------------------

#[test]
fn stray_character_reports_position() {
    let err = read_oml("a: 1\n@").unwrap_err();
    assert!(err.message.contains("stray character"));
    assert_eq!(err.line, 2);
}

#[test]
fn unterminated_string_is_an_error() {
    let err = read_oml("a: \"unterminated").unwrap_err();
    assert!(err.message.contains("unterminated string"));
}

#[test]
fn unterminated_raw_string_is_an_error() {
    let err = read_oml("a: 'unterminated").unwrap_err();
    assert!(err.message.contains("unterminated raw string"));
}

#[test]
fn unterminated_multiline_string_is_an_error() {
    let err = read_oml("a: \"\"\"unterminated").unwrap_err();
    assert!(err.message.contains("unterminated multiline string"));
}

#[test]
fn control_character_in_string_is_rejected() {
    let err = read_oml("a: \"bad\u{1}char\"").unwrap_err();
    assert!(err.message.contains("control character"));
}

#[test]
fn control_character_in_multiline_string_is_rejected() {
    let err = read_oml("a: \"\"\"bad\u{1}char\"\"\"").unwrap_err();
    assert!(err.message.contains("control character"));
}

#[test]
fn bare_word_is_rejected_as_a_value() {
    let err = read_oml("a: bareword").unwrap_err();
    assert!(err.message.contains("bare word"));
}

#[test]
fn reserved_word_cannot_be_a_bare_label() {
    // At the very top of a document, `null` parses as the null scalar
    // (looks_like_edge deliberately excludes reserved words -- matching
    // Python's `_looks_like_edge`), so the reserved-word-as-label check
    // only fires once the parser is unambiguously in edge/label position,
    // e.g. inside a `{ }` block.
    let err = read_oml("a: { null: 1 }").unwrap_err();
    assert!(err.message.contains("reserved word"));
}

#[test]
fn invalid_date_reports_a_clear_error() {
    let err = read_oml("a: 2024-02-30").unwrap_err();
    assert!(err.message.contains("invalid date"));
}

#[test]
fn invalid_time_reports_a_clear_error() {
    let err = read_oml("a: 25:00:00").unwrap_err();
    assert!(err.message.contains("invalid time"));
}

#[test]
fn invalid_datetime_reports_a_clear_error() {
    let err = read_oml("a: 2024-13-01T12:00:00").unwrap_err();
    assert!(err.message.contains("invalid datetime"));
}

#[test]
fn empty_array_is_rejected() {
    let err = read_oml("a: []").unwrap_err();
    assert!(err.message.contains("empty array"));
}

#[test]
fn nested_array_is_rejected() {
    let err = read_oml("a: [[1]]").unwrap_err();
    assert!(err.message.contains("nested array"));
}

#[test]
fn missing_colon_after_label_reports_position() {
    // At the document's very top level, "a 1" doesn't look like an edge
    // attempt at all (looks_like_edge needs a colon right after the first
    // token, which isn't there), so "a" alone is read as a bare-word-value
    // error instead. The "expected ':'" error is unambiguous once already
    // inside a `{ }` block, where a label is mandatory.
    let err = read_oml("r: { a 1 }").unwrap_err();
    assert!(err.message.contains("expected ':'"));
}

#[test]
fn missing_closing_brace_is_an_error() {
    let err = read_oml("a: { b: 1").unwrap_err();
    assert!(err.message.contains("expected '}'"));
}

#[test]
fn missing_separator_between_edges_is_an_error() {
    let err = read_oml("a: 1 b: 2").unwrap_err();
    assert!(err.message.contains("expected a separator"));
}

#[test]
fn trailing_content_after_document_body_is_an_error() {
    let err = read_oml("1 2").unwrap_err();
    assert!(err.message.contains("unexpected trailing content"));
}

#[test]
fn unquoted_field_label_must_be_a_valid_ident_or_string() {
    let err = read_oml("a: 1\n5: 2").unwrap_err();
    assert!(
        err.message.contains("unexpected trailing content") || err.message.contains("expected")
    );
}

#[test]
fn integer_digit_cap_is_enforced() {
    let digits = "1".repeat(MAX_INT_DIGITS + 1);
    let src = format!("a: {digits}");
    let err = read_oml(&src).unwrap_err();
    assert!(err.message.contains("digit"));
    assert!(err.message.contains("4300"));
}

#[test]
fn integer_at_the_digit_cap_boundary_is_accepted_by_the_scanner_even_if_i64_overflows() {
    // Exactly MAX_INT_DIGITS digits passes the cap check; i64 can't hold a
    // 4300-digit number, so this is a distinct, later error -- proving the
    // cap and the i64-range check are two separate failure modes.
    let digits = "1".repeat(MAX_INT_DIGITS);
    let src = format!("a: {digits}");
    let err = read_oml(&src).unwrap_err();
    assert!(err.message.contains("out of range"));
}

#[test]
fn integer_cap_does_not_false_positive_on_a_long_identifier() {
    // A guard-against-false-positives check (issue #10): an identifier
    // that merely *contains* a long digit run is not an INTEGER token at
    // all (IDENT must start with a letter/underscore), so the cap must
    // never fire on it.
    let ident = format!("x{}", "1".repeat(MAX_INT_DIGITS + 5));
    let err = read_oml(&format!("a: {ident}")).unwrap_err();
    assert!(err.message.contains("bare word"));
    assert!(!err.message.contains("digit"));
}

#[test]
fn out_of_range_integer_reports_a_clear_error() {
    let err = read_oml("a: 99999999999999999999").unwrap_err();
    assert!(err.message.contains("out of range"));
}

#[test]
fn invalid_unicode_escape_is_rejected() {
    let err = read_oml(r#"a: "\uZZZZ""#).unwrap_err();
    assert!(err.message.contains(r"invalid \u escape"));
}

#[test]
fn unpaired_high_surrogate_is_rejected() {
    let err = read_oml(r#"a: "\ud800""#).unwrap_err();
    assert!(err.message.contains("unpaired high surrogate"));
}

#[test]
fn unpaired_low_surrogate_is_rejected() {
    let err = read_oml(r#"a: "\udc00""#).unwrap_err();
    assert!(err.message.contains("unpaired low surrogate"));
}

#[test]
fn valid_surrogate_pair_decodes_to_the_combined_codepoint() {
    // U+1F600 (grinning face) as a UTF-16 surrogate pair.
    let node = read_oml(r#"a: "😀""#).unwrap();
    match node {
        RawNode::Edges(edges) => {
            assert_eq!(edges.len(), 1);
            match &edges[0].1 {
                RawNode::Leaf(crate::document::Scalar::Str(s)) => {
                    assert_eq!(s, "\u{1F600}");
                }
                other => panic!("expected a string leaf, got {other:?}"),
            }
        }
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn invalid_escape_character_is_rejected() {
    let err = read_oml(r#"a: "\q""#).unwrap_err();
    assert!(err.message.contains("invalid escape"));
}

#[test]
fn unterminated_escape_sequence_is_rejected() {
    let err = read_oml("a: \"\\").unwrap_err();
    assert!(err.message.contains("unterminated escape"));
}

// -- OML-Extended on read: raw strings (E2) and triple-quoted (E3) ---------

#[test]
fn raw_string_e2_reads_with_no_escape_processing() {
    let node = read_oml(r"a: 'no \n escapes here \\'").unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => {
                assert_eq!(s, r"no \n escapes here \\");
            }
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn triple_quoted_multiline_e3_strips_opening_newline() {
    let node = read_oml("a: \"\"\"\nline one\nline two\"\"\"").unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => {
                assert_eq!(s, "line one\nline two");
            }
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn triple_quoted_multiline_e3_allows_embedded_quotes_up_to_two() {
    let node = read_oml("a: \"\"\"has \"\" two quotes\"\"\"").unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => {
                assert_eq!(s, "has \"\" two quotes");
            }
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

// -- writer form details ----------------------------------------------------

#[test]
fn write_oml_compact_joins_edges_with_semicolons() {
    let raw = RawNode::Edges(vec![
        (
            "a".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(1)),
        ),
        (
            "b".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(2)),
        ),
    ]);
    assert_eq!(write_oml_compact(&raw).unwrap(), "a: 1; b: 2");
}

#[test]
fn write_oml_pretty_indents_nested_objects() {
    let raw = RawNode::Edges(vec![(
        "a".to_string(),
        RawNode::Edges(vec![(
            "b".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(1)),
        )]),
    )]);
    assert_eq!(write_oml(&raw, 2).unwrap(), "a: {\n  b: 1\n}");
}

#[test]
fn write_oml_writes_empty_object_compactly_in_both_modes() {
    let raw = RawNode::Edges(vec![("a".to_string(), RawNode::Edges(vec![]))]);
    assert_eq!(write_oml(&raw, 2).unwrap(), "a: {}");
    assert_eq!(write_oml_compact(&raw).unwrap(), "a: {}");
}

#[test]
fn labels_needing_quotes_are_quoted_on_write() {
    let raw = RawNode::Edges(vec![
        (
            "has space".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(1)),
        ),
        (
            "null".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(2)),
        ),
        (
            "1leading".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(3)),
        ),
    ]);
    let text = write_oml(&raw, 2).unwrap();
    assert!(text.contains("\"has space\": 1"));
    assert!(text.contains("\"null\": 2"));
    assert!(text.contains("\"1leading\": 3"));
}

#[test]
fn bare_identifier_label_is_written_unquoted() {
    let raw = RawNode::Edges(vec![(
        "under_score-and-dash".to_string(),
        RawNode::Leaf(crate::document::Scalar::Int(1)),
    )]);
    assert_eq!(write_oml(&raw, 2).unwrap(), "under_score-and-dash: 1");
}

// -- comments / separators ---------------------------------------------------

#[test]
fn comments_and_semicolons_are_accepted_as_separators() {
    let node = read_oml("a: 1 # a comment\n; b: 2").unwrap();
    match node {
        RawNode::Edges(edges) => assert_eq!(edges.len(), 2),
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn crlf_line_endings_are_accepted_as_separators() {
    let node = read_oml("a: 1\r\nb: 2").unwrap();
    match node {
        RawNode::Edges(edges) => assert_eq!(edges.len(), 2),
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn lone_carriage_return_is_a_stray_character() {
    let err = read_oml("a: 1\rb: 2").unwrap_err();
    assert!(err.message.contains("stray character"));
}

#[test]
fn utf8_bom_is_stripped() {
    let node = read_oml("\u{feff}a: 1").unwrap();
    match node {
        RawNode::Edges(edges) => assert_eq!(edges.len(), 1),
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn top_level_brace_document_is_equivalent_to_the_bare_edge_list() {
    let a = read_oml("a: 1").unwrap();
    let b = read_oml("{ a: 1 }").unwrap();
    assert_eq!(a, b);
}

#[test]
fn array_syntax_expands_to_repeated_edges() {
    let node = read_oml("tag: [\"x\", \"y\", \"z\"]").unwrap();
    match node {
        RawNode::Edges(edges) => {
            assert_eq!(edges.len(), 3);
            assert!(edges.iter().all(|(l, _)| l == "tag"));
        }
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn array_syntax_allows_a_trailing_comma() {
    let node = read_oml("tag: [1, 2,]").unwrap();
    match node {
        RawNode::Edges(edges) => assert_eq!(edges.len(), 2),
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn array_of_brace_subtrees_round_trips() {
    let raw = RawNode::Edges(vec![
        (
            "item".to_string(),
            RawNode::Edges(vec![(
                "v".to_string(),
                RawNode::Leaf(crate::document::Scalar::Int(1)),
            )]),
        ),
        (
            "item".to_string(),
            RawNode::Edges(vec![(
                "v".to_string(),
                RawNode::Leaf(crate::document::Scalar::Int(2)),
            )]),
        ),
    ]);
    let text = write_oml_compact(&raw).unwrap();
    let parsed = read_oml(&text).unwrap();
    assert_eq!(parsed, raw);
}

// -- Doc-level depth guard, ported from #4's pattern ------------------------

#[test]
fn read_oml_document_depth_guard_via_doc_from_raw() {
    // The reader itself also enforces MAX_DEPTH while parsing nested `{ }`
    // (parse_brace_value checks before descending), independent of
    // write_oml's own guard.
    let mut src = String::new();
    for _ in 0..(document::MAX_DEPTH + 1) {
        src.push_str("a: {");
    }
    src.push('1');
    for _ in 0..(document::MAX_DEPTH + 1) {
        src.push('}');
    }
    let err = read_oml(&src).unwrap_err();
    assert!(err.message.contains("maximum depth"));
}

// -- coverage: multiline string edge cases ----------------------------------

#[test]
fn multiline_string_strips_crlf_opening_newline() {
    let node = read_oml("a: \"\"\"\r\nline one\"\"\"").unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => assert_eq!(s, "line one"),
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn multiline_string_decodes_escapes_inside_the_body() {
    let node = read_oml("a: \"\"\"tab:\\there\"\"\"").unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => assert_eq!(s, "tab:\there"),
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn multiline_string_allows_a_run_of_more_than_three_quotes_as_content() {
    // A run of 4 quotes: the first 3 close the string, the 4th is literal
    // content of the *next* token attempt -- mirrors the Python reference's
    // `run >= 3` closing rule (only the first 3 are ever "the delimiter").
    // 4 trailing quotes: the first 3 close the string, the 4th becomes a
    // literal `"` appended to the content (mirrors the Python reference).
    let node = read_oml("a: \"\"\"x\"\"\"\"").unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => assert_eq!(s, "x\""),
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

// -- coverage: surrogate-pair escape decoding -------------------------------

#[test]
fn escaped_surrogate_pair_decodes_to_the_combined_codepoint() {
    let node = read_oml(r#"a: "😀""#).unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => assert_eq!(s, "\u{1F600}"),
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

// -- coverage: number/date/time near-miss shapes falling back correctly ----

#[test]
fn four_digits_not_followed_by_dash_is_a_plain_integer() {
    let node = read_oml("a: 1234").unwrap();
    match node {
        RawNode::Edges(edges) => {
            assert_eq!(
                edges[0].1,
                RawNode::Leaf(crate::document::Scalar::Int(1234))
            );
        }
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn date_shape_missing_second_dash_falls_back_and_errors_on_trailing_content() {
    // "2024-01x": the second dash never appears, so try_date gives up on
    // the date/datetime/time interpretation and falls through to a plain
    // integer read of "2024"; the remaining "-01x" then fails to form a
    // valid document -- an error either way, but *not* a date-shape error.
    let err = read_oml("a: 2024-01x").unwrap_err();
    assert!(!err.message.contains("invalid date"));
}

#[test]
fn two_digit_integer_is_not_mistaken_for_a_time() {
    let node = read_oml("a: 12").unwrap();
    match node {
        RawNode::Edges(edges) => {
            assert_eq!(edges[0].1, RawNode::Leaf(crate::document::Scalar::Int(12)));
        }
        other => panic!("expected edges, got {other:?}"),
    }
}

// -- coverage: writer entry points on a bare top-level node -----------------

#[test]
fn write_oml_compact_on_a_bare_leaf_node() {
    let raw = RawNode::Leaf(crate::document::Scalar::Int(5));
    assert_eq!(write_oml(&raw, 2).unwrap(), "5");
    assert_eq!(write_oml_compact(&raw).unwrap(), "5");
}

#[test]
fn write_oml_pretty_on_multiple_top_level_edges() {
    let raw = RawNode::Edges(vec![
        (
            "a".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(1)),
        ),
        (
            "b".to_string(),
            RawNode::Leaf(crate::document::Scalar::Int(2)),
        ),
    ]);
    assert_eq!(write_oml(&raw, 2).unwrap(), "a: 1\nb: 2");
}

// -- coverage: parse-error display / field paths ----------------------------

#[test]
fn parse_array_with_multiple_scalar_elements_and_no_trailing_comma() {
    let node = read_oml("tag: [1, 2, 3]").unwrap();
    match node {
        RawNode::Edges(edges) => assert_eq!(edges.len(), 3),
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn parse_label_rejects_a_non_label_token() {
    let err = read_oml("a: { 1: 2 }").unwrap_err();
    assert!(err.message.contains("expected a label"));
}

#[test]
fn parse_value_rejects_a_bad_token_where_a_scalar_is_expected() {
    let err = read_oml("a: :").unwrap_err();
    assert!(err.message.contains("expected a value"));
}

#[test]
fn array_close_error_when_neither_comma_nor_bracket() {
    let err = read_oml("a: [1 2]").unwrap_err();
    assert!(err.message.contains("expected ',' or ']'"));
}

// -- coverage: remaining escape / scanner / parser branches -----------------

#[test]
fn string_decodes_solidus_backspace_and_formfeed_escapes() {
    let node = read_oml(r#"a: "\/ \b \f""#).unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => {
                assert_eq!(s, "/ \u{8} \u{c}");
            }
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn escaped_surrogate_pair_via_explicit_u_escapes_decodes_correctly() {
    // 😀 is the UTF-16 surrogate pair for U+1F600 (grinning
    // face) -- unlike `escaped_surrogate_pair_decodes_to_the_combined_
    // codepoint` (which embeds the literal character), this exercises the
    // scanner's own `\uXXXX\uYYYY` pairing/combination code path directly:
    // the source text below contains the literal six-character escape
    // sequences, not the pre-combined UTF-8 character.
    let node = read_oml("a: \"\\uD83D\\uDE00\"").unwrap();
    match node {
        RawNode::Edges(edges) => match &edges[0].1 {
            RawNode::Leaf(crate::document::Scalar::Str(s)) => assert_eq!(s, "\u{1F600}"),
            other => panic!("expected a string leaf, got {other:?}"),
        },
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn lone_minus_not_followed_by_digit_or_inf_is_a_stray_character() {
    let err = read_oml("a: -x").unwrap_err();
    assert!(err.message.contains("stray character '-'"));
}

#[test]
fn decimal_number_with_an_exponent_round_trips() {
    // Not a write-then-read round trip on purpose: Rust's `f64::Display`
    // never emits scientific notation (unlike Python's `repr`), so
    // `write_oml` never actually *produces* a decimal-with-exponent
    // literal (`1.5e10`) -- it always writes the equivalent plain decimal.
    // This is a direct-read test of the NUMDEC-with-exponent scanner
    // branch, which OML-Extended source *can* still contain.
    let node = read_oml("a: 1.5e10").unwrap();
    match node {
        RawNode::Edges(edges) => {
            assert_eq!(
                edges[0].1,
                RawNode::Leaf(crate::document::Scalar::Float(1.5e10))
            );
        }
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn exponent_only_number_forms_are_recognized() {
    let cases = ["1e10", "1e+10", "1e-10", "-1e5"];
    for src in cases {
        let node = read_oml(&format!("a: {src}")).unwrap();
        match node {
            RawNode::Edges(edges) => {
                assert!(
                    matches!(edges[0].1, RawNode::Leaf(crate::document::Scalar::Float(_))),
                    "{src} should scan as a float"
                );
            }
            other => panic!("expected edges, got {other:?}"),
        }
    }
}

#[test]
fn quoted_string_label_at_the_top_level_is_recognized_as_an_edge() {
    // Exercises looks_like_edge's and parse_label's `Str` arms directly --
    // every other test in this suite uses a bare identifier label.
    let node = read_oml(r#""a b": 1"#).unwrap();
    match node {
        RawNode::Edges(edges) => {
            assert_eq!(edges[0].0, "a b");
            assert_eq!(edges[0].1, RawNode::Leaf(crate::document::Scalar::Int(1)));
        }
        other => panic!("expected edges, got {other:?}"),
    }
}

#[test]
fn reserved_word_at_the_document_top_level_is_read_as_the_scalar_not_a_label() {
    // Exercises `looks_like_edge`'s reserved-word-returns-false arm
    // directly: "null" at the very top of a document is the null scalar
    // (looks_like_edge deliberately excludes reserved words), so parsing
    // stops there and the trailing ": 1" is reported as unexpected
    // trailing content, not a reserved-label error (that error only fires
    // once genuinely in label position -- see
    // `reserved_word_cannot_be_a_bare_label` above).
    let err = read_oml("null: 1").unwrap_err();
    assert!(err.message.contains("unexpected trailing content"));
}

#[test]
fn missing_separator_error_names_a_quoted_string_token() {
    // Exercises `tok_display`'s `Str` arm: the *offending* token in the
    // error message is itself a quoted string, not an identifier/number.
    let err = read_oml(r#"a: 1 "b""#).unwrap_err();
    assert!(err.message.contains("expected a separator"));
    assert!(err.message.contains("\"b\""));
}
