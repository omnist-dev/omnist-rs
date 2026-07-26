use super::*;
use crate::document::{RawNode, Scalar};

fn edges(pairs: Vec<(&str, RawNode)>) -> RawNode {
    RawNode::Edges(pairs.into_iter().map(|(l, v)| (l.to_string(), v)).collect())
}

fn leaf_str(s: &str) -> RawNode {
    RawNode::Leaf(Scalar::Str(s.to_string()))
}

fn leaf_int(i: i64) -> RawNode {
    RawNode::Leaf(Scalar::Int(i))
}

// ------------------------------------------------------------ reader: basics

#[test]
fn reads_a_leaf_root_element() {
    let doc = read_xml("<root>hello</root>").unwrap();
    let raw = doc.to_raw();
    assert_eq!(raw, edges(vec![("root", leaf_str("hello"))]));
}

#[test]
fn reads_an_empty_element_as_empty_string_leaf() {
    let doc = read_xml("<root />").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("root", leaf_str(""))]));
    let doc2 = read_xml("<root></root>").unwrap();
    assert_eq!(doc2.to_raw(), edges(vec![("root", leaf_str(""))]));
}

#[test]
fn reads_nested_elements() {
    let doc = read_xml("<root><a>1</a></root>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("root", edges(vec![("a", leaf_int(1))]))])
    );
}

#[test]
fn invalid_xml_is_a_parse_error() {
    let err = read_xml("<root><a></root>").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(_)));
}

#[test]
fn empty_document_is_a_parse_error() {
    let err = read_xml("   ").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(ref e) if e.message.contains("no root element")));
}

#[test]
fn malformed_syntax_before_any_root_element_is_a_parse_error() {
    // A stray closing tag with no matching opening tag fails on the very
    // first `read_event_into` call in `read_xml`'s top-level loop, before
    // any `Start`/`Empty`/`Eof` event has been returned -- this exercises
    // that loop's own `map_err(..)` mapping, distinct from the identical
    // mapping inside `parse_content` (exercised by
    // `invalid_xml_is_a_parse_error` and friends, whose malformed syntax
    // occurs only after a valid root `Start` event has already opened).
    let err = read_xml("</root>").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(_)));
}

// -------------------------------------------------- interleaving / repetition

#[test]
fn preserves_repeated_sibling_elements_as_repeated_edges() {
    let doc = read_xml("<root><x>1</x><x>2</x></root>").unwrap();
    let RawNode::Edges(root_edges) = doc.to_raw() else {
        panic!("expected edges")
    };
    let (tag, content) = &root_edges[0];
    assert_eq!(tag, "root");
    assert_eq!(
        *content,
        edges(vec![("x", leaf_int(1)), ("x", leaf_int(2))])
    );
}

#[test]
fn preserves_interleaved_distinct_labels_exactly() {
    // <b/><c/><b/> -- interleaved, not a contiguous run of "b". This is
    // exactly the shape a `Value::Object`/`IndexMap` can't represent,
    // which is why this module goes through `RawNode` instead of
    // `to_grouped` (see this module's doc comment).
    let doc = read_xml("<root><b>1</b><c>2</c><b>3</b></root>").unwrap();
    let RawNode::Edges(root_edges) = doc.to_raw() else {
        panic!("expected edges")
    };
    let (_, content) = &root_edges[0];
    let RawNode::Edges(inner) = content else {
        panic!("expected edges")
    };
    let labels: Vec<&str> = inner.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, vec!["b", "c", "b"]);
    assert_eq!(inner[0].1, leaf_int(1));
    assert_eq!(inner[1].1, leaf_int(2));
    assert_eq!(inner[2].1, leaf_int(3));
}

#[test]
fn round_trips_interleaved_repeated_elements() {
    let text = "<root>\n  <b>1</b>\n  <c>2</c>\n  <b>3</b>\n</root>\n";
    let doc = read_xml(text).unwrap();
    let out = write_xml(&doc, false, None).unwrap();
    assert_eq!(out, text);
    let doc2 = read_xml(&out).unwrap();
    assert!(doc.eq_doc(&doc2));
}

// ---------------------------------------------------------------- mixed content

#[test]
fn mixed_content_text_alongside_children_is_a_parse_error() {
    let err = read_xml("<root>text<a>1</a></root>").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(ref e) if e.message.contains("mixed content")));
}

#[test]
fn whitespace_only_text_alongside_children_is_not_mixed_content() {
    let doc = read_xml("<root>\n  <a>1</a>\n</root>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("root", edges(vec![("a", leaf_int(1))]))])
    );
}

// --------------------------------------------------------------------- coercion

#[test]
fn coerces_booleans_case_insensitively() {
    for (text, expected) in [
        ("true", true),
        ("True", true),
        ("TRUE", true),
        ("false", false),
        ("FALSE", false),
    ] {
        let doc = read_xml(&format!("<r>{text}</r>")).unwrap();
        assert_eq!(
            doc.to_raw(),
            edges(vec![("r", RawNode::Leaf(Scalar::Bool(expected)))])
        );
    }
}

#[test]
fn bool_match_does_not_trim_surrounding_whitespace() {
    // Live-confirmed against Python: `_coerce(' true ')` stays the string
    // `' true '` -- the bool comparison uses the raw text, not a trimmed
    // copy (omnist-ts#53-style undocumented-narrowing lesson: verify, don't
    // assume).
    let doc = read_xml("<r> true </r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str(" true "))]));
}

#[test]
fn coerces_plain_integers() {
    let doc = read_xml("<r>42</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_int(42))]));
    let doc2 = read_xml("<r>-7</r>").unwrap();
    assert_eq!(doc2.to_raw(), edges(vec![("r", leaf_int(-7))]));
}

#[test]
fn coerces_underscore_grouped_integer_literal() {
    // Live-confirmed: Python's int("1_0") == 10.
    let doc = read_xml("<r>1_0</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_int(10))]));
}

#[test]
fn coerces_leading_zero_integer() {
    // Live-confirmed: Python's int("007") == 7 (int() has no octal
    // interpretation of a leading zero, unlike a Python integer literal).
    let doc = read_xml("<r>007</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_int(7))]));
}

#[test]
fn coerces_plain_floats() {
    let doc = read_xml("<r>1.5</r>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("r", RawNode::Leaf(Scalar::Float(1.5)))])
    );
}

#[test]
fn coerces_leading_and_trailing_dot_floats() {
    // Live-confirmed: float(".5") == 0.5, float("5.") == 5.0.
    let doc = read_xml("<a>.5</a>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("a", RawNode::Leaf(Scalar::Float(0.5)))])
    );
    let doc2 = read_xml("<a>5.</a>").unwrap();
    assert_eq!(
        doc2.to_raw(),
        edges(vec![("a", RawNode::Leaf(Scalar::Float(5.0)))])
    );
}

#[test]
fn parses_inf_and_nan_words() {
    // Live-confirmed: float("inf") == inf, float("Infinity") == inf,
    // float("nan") == nan (any ASCII case, optionally signed).
    for text in ["inf", "Infinity", "INFINITY", "-inf", "+inf"] {
        let doc = read_xml(&format!("<r>{text}</r>")).unwrap();
        let RawNode::Edges(e) = doc.to_raw() else {
            panic!()
        };
        let RawNode::Leaf(Scalar::Float(f)) = &e[0].1 else {
            panic!("expected float leaf for {text:?}")
        };
        assert!(f.is_infinite(), "{text} should be infinite, got {f}");
    }
    let doc = read_xml("<r>nan</r>").unwrap();
    let RawNode::Edges(e) = doc.to_raw() else {
        panic!()
    };
    let RawNode::Leaf(Scalar::Float(f)) = &e[0].1 else {
        panic!("expected float leaf")
    };
    assert!(f.is_nan());
}

#[test]
fn underscore_grouped_float_coerces() {
    // Live-confirmed: float("1_0.5") == 10.5.
    let doc = read_xml("<r>1_0.5</r>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("r", RawNode::Leaf(Scalar::Float(10.5)))])
    );
}

#[test]
fn malformed_underscore_grouping_stays_a_string() {
    // Live-confirmed: Python's int()/float() reject a leading, trailing,
    // or doubled underscore -- "_1", "1_", "1__0" all stay strings.
    for text in ["_1", "1_", "1__0"] {
        let doc = read_xml(&format!("<r>{text}</r>")).unwrap();
        assert_eq!(
            doc.to_raw(),
            edges(vec![("r", leaf_str(text))]),
            "input {text:?}"
        );
    }
}

#[test]
fn non_numeric_text_stays_a_string() {
    let doc = read_xml("<r>hello world</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("hello world"))]));
}

#[test]
fn hex_literal_text_stays_a_string() {
    // Live-confirmed: Python's int("0x1A")/float("0x1A") both raise
    // (no implicit base without base=0), so it stays a string.
    let doc = read_xml("<r>0x1A</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("0x1A"))]));
}

#[test]
fn whitespace_only_text_with_no_digits_stays_unchanged() {
    // Live-confirmed: Python's int("  ")/float("  ") both raise (no
    // digits at all), so _coerce falls through to the *original*,
    // unstripped text.
    let doc = read_xml("<r>  </r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("  "))]));
}

#[test]
fn oversized_integer_falls_back_to_float_not_a_parse_error() {
    // Live-confirmed divergence: Python's _coerce("9"*20) is still an
    // exact Python int (arbitrary precision) -- this port's Scalar::Int
    // is i64-backed (19-digit range), so it falls through to
    // try_parse_float instead of erroring, matching _coerce's actual
    // int-then-float control flow (see this module's doc comment).
    let text = "9".repeat(20);
    let doc = read_xml(&format!("<r>{text}</r>")).unwrap();
    let RawNode::Edges(e) = doc.to_raw() else {
        panic!()
    };
    assert!(matches!(&e[0].1, RawNode::Leaf(Scalar::Float(_))));
}

// ------------------------------------------------------------------ CR normalization

#[test]
fn carriage_returns_normalize_to_newline_on_read() {
    // Live-confirmed against defusedxml: "line\r\ncr\ronly" reads as
    // "line\ncr\nonly".
    let doc = read_xml("<a>line\r\ncr\ronly</a>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("a", leaf_str("line\ncr\nonly"))]));
}

// ------------------------------------------------------------------------ depth guard

#[test]
fn depth_guard_rejects_deeply_nested_xml() {
    let mut text = String::from("<a>");
    for _ in 0..MAX_DEPTH + 5 {
        text.push_str("<a>");
    }
    text.push('1');
    for _ in 0..MAX_DEPTH + 6 {
        text.push_str("</a>");
    }
    let err = read_xml(&text).unwrap_err();
    assert!(
        matches!(err, OmnistError::Document(ref e) if e.message.contains("maximum depth")),
        "got {err:?}"
    );
}

#[test]
fn depth_at_the_boundary_is_accepted() {
    // MAX_DEPTH levels of nesting is the accept boundary (root itself is
    // depth 0, its content is depth 1, ...).
    let mut text = String::from("<a>");
    for _ in 0..MAX_DEPTH - 1 {
        text.push_str("<a>");
    }
    text.push('1');
    for _ in 0..MAX_DEPTH {
        text.push_str("</a>");
    }
    assert!(read_xml(&text).is_ok());
}

// --------------------------------------------------------------------- write: shape

#[test]
fn write_requires_exactly_one_top_level_edge() {
    let doc = Doc::of(&crate::document::Value::Int(5)).unwrap();
    let err = write_xml(&doc, false, None).unwrap_err();
    assert!(err.to_string().contains("exactly one document element"));
    assert!(err.report().is_none());
}

#[test]
fn write_rejects_a_multi_edge_root() {
    let raw = edges(vec![("a", leaf_int(1)), ("b", leaf_int(2))]);
    let doc = Doc::from_raw(raw).unwrap();
    let err = write_xml(&doc, false, None).unwrap_err();
    assert!(err.to_string().contains("exactly one document element"));
}

// ---------------------------------------------------------------------- write: shapes

#[test]
fn writes_a_leaf_root_on_one_line_no_trailing_newline() {
    let doc = Doc::from_raw(edges(vec![("root", leaf_str("hello"))])).unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(text, "<root>hello</root>");
}

#[test]
fn writes_an_empty_root_self_closed_no_trailing_newline() {
    let doc = Doc::from_raw(edges(vec![("root", edges(vec![]))])).unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(text, "<root />");
}

#[test]
fn writes_nested_elements_indented_with_trailing_newline() {
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![
            ("a", leaf_int(1)),
            ("b", edges(vec![("x", leaf_str("1")), ("x", leaf_str("2"))])),
            ("c", edges(vec![])),
        ]),
    )]))
    .unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(
        text,
        "<root>\n  <a>1</a>\n  <b>\n    <x>1</x>\n    <x>2</x>\n  </b>\n  <c />\n</root>\n"
    );
}

#[test]
fn writes_deeply_nested_single_child_chain() {
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![(
            "a",
            edges(vec![("b", edges(vec![("c", leaf_int(1))]))]),
        )]),
    )]))
    .unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(
        text,
        "<root>\n  <a>\n    <b>\n      <c>1</c>\n    </b>\n  </a>\n</root>\n"
    );
}

// --------------------------------------------------------------- write: escaping

#[test]
fn escapes_ampersand_lt_gt_in_text_but_not_quotes() {
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![("a", leaf_str("x < y & z > w \"quote\" 'apos'"))]),
    )]))
    .unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert!(text.contains("x &lt; y &amp; z &gt; w \"quote\" 'apos'"));
}

// ------------------------------------------------------- all-occurrences sanitization

#[test]
fn sanitizes_every_illegal_character_not_just_the_first() {
    // Regression test for omnist-ts#36: writeXml's xmlSanitize used a
    // non-global regex, replacing only the *first* XML-illegal character.
    // Three illegal C0 controls (\x01, \x02, \x03) in one string -- all
    // three must become U+FFFD, not just the first.
    let illegal = "a\u{01}b\u{02}c\u{03}d";
    let doc = Doc::from_raw(edges(vec![("root", leaf_str(illegal))])).unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(text, "<root>a\u{FFFD}b\u{FFFD}c\u{FFFD}d</root>");
    assert_eq!(
        text.matches('\u{FFFD}').count(),
        3,
        "all three must be replaced"
    );
}

#[test]
fn check_xml_reports_illegal_char_as_error_and_cr_as_warning() {
    let doc = Doc::from_raw(edges(vec![("root", leaf_str("bad\u{01}\rtext"))])).unwrap();
    let rep = check_xml(&doc);
    assert!(
        rep.adjustments()
            .iter()
            .any(|a| a.code == "string.illegal_xml_char" && a.severity == Severity::Error)
    );
    assert!(
        rep.adjustments()
            .iter()
            .any(|a| a.code == "string.cr_normalized" && a.severity == Severity::Warning)
    );
}

#[test]
fn strict_write_raises_on_illegal_char_error() {
    let doc = Doc::from_raw(edges(vec![("root", leaf_str("bad\u{01}"))])).unwrap();
    let err = write_xml(&doc, true, None).unwrap_err();
    assert!(err.report().is_some());
}

// ----------------------------------------------------------------- key sanitization

#[test]
fn sanitizes_an_invalid_label_and_records_adjustment() {
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![("1bad label", leaf_int(1))]),
    )]))
    .unwrap();
    let mut rep = WriteReport::new();
    let text = write_xml(&doc, false, Some(&mut rep)).unwrap();
    assert!(text.contains("<_1bad_label>1</_1bad_label>"));
    assert!(rep.adjustments().iter().any(|a| a.code == "key.sanitized"));
}

#[test]
fn xml_name_prefixes_underscore_when_sanitized_result_still_invalid() {
    // A label made of only illegal characters sanitizes to an
    // all-underscore string, which IS a valid XML name on its own
    // (starts with `_`) -- but a label that sanitizes to something
    // starting with a digit still needs the extra leading underscore.
    assert_eq!(xml_name("123"), "_123");
    assert_eq!(xml_name("a b"), "a_b");
    assert_eq!(xml_name("valid_name"), "valid_name");
}

// --------------------------------------------------------------------- empty shape

#[test]
fn empty_internal_node_reports_shape_empty_ambiguous() {
    let doc = Doc::from_raw(edges(vec![("root", edges(vec![("empty", edges(vec![]))]))])).unwrap();
    let rep = check_xml(&doc);
    assert!(
        rep.adjustments()
            .iter()
            .any(|a| a.code == "shape.empty_ambiguous")
    );
}

// --------------------------------------------------------------- ambiguous string

#[test]
fn string_that_looks_like_another_type_is_flagged_ambiguous() {
    let doc = Doc::from_raw(edges(vec![("root", leaf_str("42"))])).unwrap();
    let rep = check_xml(&doc);
    assert!(
        rep.adjustments()
            .iter()
            .any(|a| a.code == "string.ambiguous"),
        "{rep:?}"
    );
}

#[test]
fn ordinary_string_is_not_flagged_ambiguous() {
    let doc = Doc::from_raw(edges(vec![("root", leaf_str("hello"))])).unwrap();
    let rep = check_xml(&doc);
    assert!(
        !rep.adjustments()
            .iter()
            .any(|a| a.code == "string.ambiguous")
    );
}

// --------------------------------------------------------------------- null leaf

#[test]
fn null_leaf_writes_as_empty_element_and_is_reported() {
    let doc = Doc::from_raw(edges(vec![("root", RawNode::Leaf(Scalar::Null))])).unwrap();
    let mut rep = WriteReport::new();
    let text = write_xml(&doc, false, Some(&mut rep)).unwrap();
    assert_eq!(text, "<root />");
    assert!(rep.adjustments().iter().any(|a| a.code == "null.omitted"));
}

// ------------------------------------------------------------------------ round trip

#[test]
fn round_trips_every_scalar_kind() {
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![
            ("i", leaf_int(42)),
            ("s", leaf_str("hello")),
            ("b", RawNode::Leaf(Scalar::Bool(true))),
            ("f", RawNode::Leaf(Scalar::Float(1.5))),
        ]),
    )]))
    .unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    let doc2 = read_xml(&text).unwrap();
    assert!(doc.eq_doc(&doc2));
}

#[test]
fn float_integral_value_gets_a_decimal_point() {
    let doc = Doc::from_raw(edges(vec![("root", RawNode::Leaf(Scalar::Float(1.0)))])).unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(text, "<root>1.0</root>");
}

#[test]
fn nan_and_infinity_round_trip() {
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![
            ("a", RawNode::Leaf(Scalar::Float(f64::NAN))),
            ("b", RawNode::Leaf(Scalar::Float(f64::INFINITY))),
            ("c", RawNode::Leaf(Scalar::Float(f64::NEG_INFINITY))),
        ]),
    )]))
    .unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert!(text.contains("<a>nan</a>"));
    assert!(text.contains("<b>inf</b>"));
    assert!(text.contains("<c>-inf</c>"));
    let doc2 = read_xml(&text).unwrap();
    let RawNode::Edges(e) = doc2.to_raw() else {
        panic!()
    };
    let RawNode::Edges(inner) = &e[0].1 else {
        panic!()
    };
    assert!(matches!(&inner[0].1, RawNode::Leaf(Scalar::Float(f)) if f.is_nan()));
    assert!(matches!(&inner[1].1, RawNode::Leaf(Scalar::Float(f)) if f.is_infinite() && *f > 0.0));
    assert!(matches!(&inner[2].1, RawNode::Leaf(Scalar::Float(f)) if f.is_infinite() && *f < 0.0));
}

// ------------------------------------------------------------------ namespaces

#[test]
fn a_prefixed_tag_reads_as_its_local_name() {
    // See this module's doc comment on the disclosed namespace
    // simplification -- the prefix is stripped lexically, not resolved
    // through an xmlns declaration.
    let doc = read_xml("<ns:root xmlns:ns=\"urn:example\">1</ns:root>").unwrap();
    let RawNode::Edges(e) = doc.to_raw() else {
        panic!()
    };
    assert_eq!(e[0].0, "root");
}

// ---------------------------------------------------------------- white-box coverage

#[test]
fn xml_sanitize_leaves_legal_characters_untouched() {
    assert_eq!(xml_sanitize("hello\tworld\n"), "hello\tworld\n");
}

#[test]
fn is_xml_illegal_char_boundary_values() {
    assert!(is_xml_illegal_char('\u{00}'));
    assert!(!is_xml_illegal_char('\t'));
    assert!(!is_xml_illegal_char('\n'));
    assert!(!is_xml_illegal_char('\r'));
    assert!(is_xml_illegal_char('\u{0B}'));
    assert!(is_xml_illegal_char('\u{1F}'));
    assert!(!is_xml_illegal_char(' '));
    assert!(is_xml_illegal_char('\u{FFFE}'));
    assert!(is_xml_illegal_char('\u{FFFF}'));
    assert!(!is_xml_illegal_char('\u{FFFD}'));
}

#[test]
fn xml_text_covers_bool_false() {
    assert_eq!(xml_text(&Scalar::Bool(false)), "false");
}

// -------------------------------------------------------------- reader: extra event kinds

#[test]
fn nested_self_closed_child_is_an_empty_string_leaf() {
    // Exercises the `Event::Empty` arm inside `parse_content` (the
    // top-level `read_xml` loop has its own separate `Empty` arm for a
    // self-closed *root*; this covers the nested case).
    let doc = read_xml("<root><a/></root>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("root", edges(vec![("a", leaf_str(""))]))])
    );
}

#[test]
fn comments_and_pis_inside_an_element_body_are_skipped() {
    // Exercises `parse_content`'s catch-all arm (comments/PIs interleaved
    // with real content are ignored, not treated as text or an error).
    let doc = read_xml("<root><!-- a comment --><a>1</a><?pi data?></root>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("root", edges(vec![("a", leaf_int(1))]))])
    );
}

#[test]
fn cdata_section_contributes_to_leaf_text() {
    let doc = read_xml("<root><![CDATA[hello <world>]]></root>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("root", leaf_str("hello <world>"))])
    );
}

#[test]
fn truncated_document_inside_a_nested_element_is_a_parse_error() {
    // Unclosed at EOF while *inside* `parse_content`'s own loop (not the
    // top-level `read_xml` loop) -- exercises `parse_content`'s own
    // `Event::Eof` arm.
    let err = read_xml("<root><a>").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(ref e) if e.message.contains("unexpected end")));
}

#[test]
fn malformed_numeric_character_reference_is_a_parse_error() {
    // "&#zzz;" is not a valid numeric character reference -- `quick_xml`
    // tokenizes it as its own `GeneralRef` event (distinct from `Text`),
    // and `resolve_general_ref`'s `resolve_char_ref()` call surfaces the
    // invalid-digit failure as a `ParseError`.
    let err = read_xml("<root>&#zzz;</root>").unwrap_err();
    assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
}

#[test]
fn parse_error_on_line_two_reports_line_two() {
    // Exercises `line_col`'s newline-counting branch for an XML parse
    // error (the same branch `toml.rs`'s identical helper already covers
    // for TOML), via a truncated document whose failure is only detected
    // after a line break.
    let err = read_xml("<root>\n<a>").unwrap_err();
    assert!(
        matches!(err, OmnistError::Parse(ref e) if e.line == 2),
        "got {err:?}"
    );
}

#[test]
fn numeric_and_named_character_references_resolve_correctly() {
    // "&#65;" (decimal) and "&#x41;" (hex) are both "A"; the five
    // predefined named entities resolve by name via `resolve_general_ref`.
    let doc = read_xml("<r>&#65;&#x41;&lt;&gt;&amp;&apos;&quot;</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("AA<>&'\""))]));
}

#[test]
fn unrecognized_named_entity_is_a_parse_error() {
    // `quick_xml` has no DTD support, so only the five predefined
    // entities can ever resolve -- anything else is a parse error
    // (see this module's doc comment on why that's the crate's XXE-safe
    // behavior, not a gap).
    let err = read_xml("<root>&undefinedentity;</root>").unwrap_err();
    assert!(
        matches!(err, OmnistError::Parse(ref e) if e.message.contains("unrecognized entity")),
        "got {err:?}"
    );
}
