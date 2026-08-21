use super::*;
use crate::document::{RawNode, Scalar};

fn edges(pairs: Vec<(&str, RawNode)>) -> RawNode {
    RawNode::Edges(pairs.into_iter().map(|(l, v)| (l.to_string(), v)).collect())
}

fn leaf_str(s: &str) -> RawNode {
    RawNode::Leaf(Scalar::Str(s.to_string()))
}

fn leaf(s: Scalar) -> RawNode {
    RawNode::Leaf(s)
}

fn leaf_bool(b: bool) -> RawNode {
    RawNode::Leaf(Scalar::Bool(b))
}

/// Only meaningful on the *write* side now (omnist-rs#86: `read_xml` never
/// produces `Scalar::Int` -- every leaf it builds is a `Scalar::Str`). Kept
/// for constructing `Doc`s to feed into `write_xml`/`check_xml`.
fn leaf_int(i: i64) -> RawNode {
    RawNode::Leaf(Scalar::Int(i.into()))
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
    // omnist-rs#86: numeric-looking text stays a string -- typing is
    // materialize's job, not the reader's.
    let doc = read_xml("<root><a>1</a></root>").unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![("root", edges(vec![("a", leaf_str("1"))]))])
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
        edges(vec![("x", leaf_str("1")), ("x", leaf_str("2"))])
    );
}

#[test]
fn preserves_interleaved_distinct_labels_exactly() {
    // <b/><c/><b/> -- interleaved, not a contiguous run of "b". This is
    // exactly the shape a `Value::Object`/`IndexMap` can't represent,
    // which is why this module goes through `RawNode` instead of
    // `to_grouped` (see this module's doc comment). This is also the
    // exact shape of omnist-rs#86's regression vector
    // (`formats-xml/basic/interleaved-elements-preserve-order`): order
    // preservation *and* untyped (string) leaves, both checked here.
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
    assert_eq!(inner[0].1, leaf_str("1"));
    assert_eq!(inner[1].1, leaf_str("2"));
    assert_eq!(inner[2].1, leaf_str("3"));
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
        edges(vec![("root", edges(vec![("a", leaf_str("1"))]))])
    );
}

// ------------------------------------------------- reader: text stays untyped

// omnist-rs#86: `read_xml` used to type-infer leaf text (int/float/bool) at
// parse time via a `coerce()` helper, contradicting `docs/formats/xml.md`
// ("Text is untyped ... every leaf arrives as a string. Typing requires a
// schema in stage 2.") and diverging from Python's reference `read_xml`.
// These tests used to assert the coerced (buggy) typed result; they're kept
// and updated in place -- same inputs, now asserting the untyped string
// result -- rather than deleted, to preserve the coverage of exactly which
// text shapes were previously (wrongly) coerced.

#[test]
fn boolean_looking_text_stays_a_string() {
    for text in ["true", "True", "TRUE", "false", "FALSE"] {
        let doc = read_xml(&format!("<r>{text}</r>")).unwrap();
        assert_eq!(
            doc.to_raw(),
            edges(vec![("r", leaf_str(text))]),
            "input {text:?}"
        );
    }
}

#[test]
fn bool_looking_text_with_surrounding_whitespace_stays_a_string() {
    // Previously this was already a string under the old coercion (the bool
    // match required an exact, untrimmed "true"/"false"), but it's included
    // here alongside the other untyped-text tests for completeness now that
    // *no* leaf text is ever coerced.
    let doc = read_xml("<r> true </r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str(" true "))]));
}

#[test]
fn plain_integer_text_stays_a_string() {
    let doc = read_xml("<r>42</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("42"))]));
    let doc2 = read_xml("<r>-7</r>").unwrap();
    assert_eq!(doc2.to_raw(), edges(vec![("r", leaf_str("-7"))]));
}

#[test]
fn underscore_grouped_integer_literal_stays_a_string() {
    // Previously coerced (Python's int("1_0") == 10); now the raw text is
    // kept verbatim, underscore and all.
    let doc = read_xml("<r>1_0</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("1_0"))]));
}

#[test]
fn leading_zero_integer_text_stays_a_string() {
    let doc = read_xml("<r>007</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("007"))]));
}

#[test]
fn plain_float_text_stays_a_string() {
    let doc = read_xml("<r>1.5</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("1.5"))]));
}

#[test]
fn leading_and_trailing_dot_float_text_stays_a_string() {
    let doc = read_xml("<a>.5</a>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("a", leaf_str(".5"))]));
    let doc2 = read_xml("<a>5.</a>").unwrap();
    assert_eq!(doc2.to_raw(), edges(vec![("a", leaf_str("5."))]));
}

#[test]
fn inf_and_nan_words_stay_strings() {
    // Previously coerced via `float()`-style parsing (inf/infinity/nan,
    // any ASCII case, optionally signed); now kept verbatim.
    for text in ["inf", "Infinity", "INFINITY", "-inf", "+inf", "nan"] {
        let doc = read_xml(&format!("<r>{text}</r>")).unwrap();
        assert_eq!(
            doc.to_raw(),
            edges(vec![("r", leaf_str(text))]),
            "input {text:?}"
        );
    }
}

#[test]
fn underscore_grouped_float_text_stays_a_string() {
    let doc = read_xml("<r>1_0.5</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("1_0.5"))]));
}

#[test]
fn malformed_underscore_grouping_stays_a_string() {
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
    let doc = read_xml("<r>0x1A</r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("0x1A"))]));
}

#[test]
fn whitespace_only_text_stays_unchanged() {
    let doc = read_xml("<r>  </r>").unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str("  "))]));
}

#[test]
fn oversized_integer_text_stays_a_string_not_a_parse_error() {
    // Previously this was where `Scalar::Int`'s i64-only range forced a
    // fallback to `Scalar::Float` (issue-worthy divergence from Python's
    // arbitrary-precision `int`, documented at length in the old coercion
    // doc comment). That whole representational question is moot now: a
    // 20-digit run of `9`s just stays the literal text, no numeric parsing
    // attempted at all.
    let text = "9".repeat(20);
    let doc = read_xml(&format!("<r>{text}</r>")).unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("r", leaf_str(&text))]));
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
    let doc = Doc::of(&crate::document::Value::Int((5).into())).unwrap();
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

#[test]
fn write_requires_exactly_one_top_level_edge_even_with_a_report_supplied() {
    let doc = Doc::of(&crate::document::Value::Int((5).into())).unwrap();
    let mut rep = crate::report::WriteReport::new();
    let err = write_xml(&doc, false, Some(&mut rep)).unwrap_err();
    assert!(err.to_string().contains("exactly one document element"));
    assert!(rep.is_empty());
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

// ------------------------------------------------------- write: non-string scalars

// omnist-rs#86: `read_xml` no longer infers scalar kind from element-text
// shape, so a non-string scalar (`bool`/`int`/`float`) written to XML now
// reads back as a plain string, not its original type -- XML has no native
// typed literals, everything is text. This used to be silent (the old
// shape-based read-side coercion happened to undo it); it's now reported,
// matching Python's identical fix (`omnist#288`). These tests replace the
// old `string.ambiguous`-based ones (that code checked whether a *string*
// value merely looked like another type, which no longer means anything
// once the reader never re-types on looks alone).

#[test]
fn non_string_scalar_is_flagged_value_stringified() {
    let doc = Doc::from_raw(edges(vec![("root", leaf_int(42))])).unwrap();
    let rep = check_xml(&doc);
    assert!(
        rep.adjustments()
            .iter()
            .any(|a| a.code == "value.stringified"),
        "{rep:?}"
    );
}

#[test]
fn bool_and_float_scalars_are_also_flagged_value_stringified() {
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![
            ("b", RawNode::Leaf(Scalar::Bool(true))),
            ("f", RawNode::Leaf(Scalar::Float(1.5))),
        ]),
    )]))
    .unwrap();
    let rep = check_xml(&doc);
    let count = rep
        .adjustments()
        .iter()
        .filter(|a| a.code == "value.stringified")
        .count();
    assert_eq!(count, 2, "{rep:?}");
}

#[test]
fn a_string_that_looks_like_a_number_is_not_flagged() {
    // A `Scalar::Str` is never flagged `value.stringified` -- it's already
    // a string, nothing is being stringified. This is true regardless of
    // what the string's text looks like, since read_xml never re-types on
    // looks (omnist-rs#86).
    let doc = Doc::from_raw(edges(vec![("root", leaf_str("42"))])).unwrap();
    let rep = check_xml(&doc);
    assert!(
        !rep.adjustments()
            .iter()
            .any(|a| a.code == "value.stringified" || a.code == "string.ambiguous"),
        "{rep:?}"
    );
}

#[test]
fn ordinary_string_is_not_flagged_value_stringified() {
    let doc = Doc::from_raw(edges(vec![("root", leaf_str("hello"))])).unwrap();
    let rep = check_xml(&doc);
    assert!(
        !rep.adjustments()
            .iter()
            .any(|a| a.code == "value.stringified")
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
fn round_trips_string_leaves() {
    // omnist-rs#86: a non-string scalar (int/bool/float) no longer
    // round-trips through XML at all -- it reads back as a string (see the
    // `value.stringified` tests above) -- so the meaningful round-trip
    // guarantee left for `read_xml`/`write_xml` alone (no schema) is over
    // string leaves only.
    let doc = Doc::from_raw(edges(vec![(
        "root",
        edges(vec![
            ("i", leaf_str("42")),
            ("s", leaf_str("hello")),
            ("b", leaf_str("true")),
            ("f", leaf_str("1.5")),
        ]),
    )]))
    .unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    let doc2 = read_xml(&text).unwrap();
    assert!(doc.eq_doc(&doc2));
}

#[test]
fn non_string_scalar_reads_back_as_a_string_not_its_original_type() {
    // The write-side counterpart of `non_string_scalar_is_flagged_value_stringified`:
    // confirms what actually happens on a full write->read round trip, not
    // just that it's reported.
    let doc = Doc::from_raw(edges(vec![("root", leaf_int(42))])).unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    let doc2 = read_xml(&text).unwrap();
    assert_eq!(doc2.to_raw(), edges(vec![("root", leaf_str("42"))]));
}

#[test]
fn bool_scalar_writes_as_the_word_true_and_reads_back_as_that_string() {
    // Same shape as the int case above, for `Scalar::Bool(true)`
    // specifically (`xml_text`'s `Scalar::Bool` arm has a `true`/`false`
    // branch; `xml_text_covers_bool_false` already exercises the `false`
    // side, this exercises `true`).
    let doc = Doc::from_raw(edges(vec![("root", RawNode::Leaf(Scalar::Bool(true)))])).unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(text, "<root>true</root>");
    let doc2 = read_xml(&text).unwrap();
    assert_eq!(doc2.to_raw(), edges(vec![("root", leaf_str("true"))]));
}

#[test]
fn float_integral_value_gets_a_decimal_point() {
    let doc = Doc::from_raw(edges(vec![("root", RawNode::Leaf(Scalar::Float(1.0)))])).unwrap();
    let text = write_xml(&doc, false, None).unwrap();
    assert_eq!(text, "<root>1.0</root>");
}

#[test]
fn integral_float_at_and_above_1e17_boundary_issue_46_writes_without_a_decimal_point_regression() {
    // Regression test for issue #46: an integral-valued float >= 1e17 used
    // to render as a bare digit run, which the old `coerce()` then
    // re-read as `Scalar::Int` instead of `Scalar::Float` (a round-trip
    // failure at the time). omnist-rs#86 removed `coerce()` entirely, so
    // that specific failure mode is structurally impossible now -- every
    // leaf reads back as a string regardless of its digit shape -- but the
    // float-formatting behavior issue #46 was actually about (always
    // including a `.`/`e` marker) is still worth pinning down here.
    for x in [1.0e17, 1.0e18, -1.23e17, 9.9e16_f64] {
        let doc = Doc::from_raw(edges(vec![("root", RawNode::Leaf(Scalar::Float(x)))])).unwrap();
        let text = write_xml(&doc, false, None).unwrap();
        assert!(
            text.contains('.') || text.contains('e') || text.contains('E'),
            "x={x} text={text}"
        );
        let back = read_xml(&text).unwrap();
        // The float no longer round-trips as a Float (it's a Str now,
        // omnist-rs#86) -- confirm it at least reads back as the same text
        // that was written, i.e. no silent reinterpretation happened.
        let RawNode::Edges(e) = back.to_raw() else {
            panic!()
        };
        assert!(
            matches!(&e[0].1, RawNode::Leaf(Scalar::Str(_))),
            "x={x} text={text}"
        );
    }
}

#[test]
fn nan_and_infinity_write_as_words_and_read_back_as_strings() {
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
    assert_eq!(
        doc2.to_raw(),
        edges(vec![(
            "root",
            edges(vec![
                ("a", leaf_str("nan")),
                ("b", leaf_str("inf")),
                ("c", leaf_str("-inf")),
            ])
        )])
    );
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
        edges(vec![("root", edges(vec![("a", leaf_str("1"))]))])
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

#[test]
fn test_xml_epilog_second_root_start_event_rejected() {
    let err = read_xml("<root></root><second></second>").unwrap_err();
    assert!(err.to_string().contains("multiple root elements"));
}

#[test]
fn test_xml_epilog_second_root_empty_event_rejected() {
    let err = read_xml("<root></root><second/>").unwrap_err();
    assert!(err.to_string().contains("multiple root elements"));
}

#[test]
fn test_xml_epilog_non_whitespace_text_rejected() {
    let err = read_xml("<root></root>trailing").unwrap_err();
    assert!(
        err.to_string()
            .contains("unexpected text after root element")
    );
}

#[test]
fn test_xml_epilog_non_whitespace_cdata_rejected() {
    let err = read_xml("<root></root><![CDATA[trailing]]>").unwrap_err();
    assert!(
        err.to_string()
            .contains("unexpected text after root element")
    );
}

#[test]
fn test_xml_epilog_legal_comments_pi_doctype_skipped() {
    let src = "<root></root><!-- trailing comment --><?pi target?><!DOCTYPE note>";
    let doc = read_xml(src).unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("root", leaf_str(""))]));
}

#[test]
fn test_xml_epilog_whitespace_text_and_cdata_skipped() {
    let src = "<root></root>   \n\t  <![CDATA[   \n ]]>  ";
    let doc = read_xml(src).unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("root", leaf_str(""))]));
}

#[test]
fn test_xml_prolog_non_whitespace_cdata_rejected() {
    let err = read_xml("<![CDATA[leading]]><root></root>").unwrap_err();
    assert!(
        err.to_string()
            .contains("unexpected text outside root element")
    );
}

#[test]
fn test_xml_prolog_whitespace_cdata_skipped() {
    let src = "<![CDATA[   ]]><root></root>";
    let doc = read_xml(src).unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("root", leaf_str(""))]));
}

#[test]
fn test_xml_repro_garbage_multi_root_trailing_rejected() {
    let err = read_xml("garbage<a/><b/>trailing").unwrap_err();
    assert!(err.to_string().contains("invalid XML"));
}
#[test]
fn test_xml_prolog_decl_comment_pi_doctype_skipped() {
    let src = "<?xml version=\"1.0\"?>\n<!-- c -->\n<?pi target?>\n<!DOCTYPE root>\n<root/>";
    let doc = read_xml(src).unwrap();
    assert_eq!(doc.to_raw(), edges(vec![("root", leaf_str(""))]));
}

#[test]
fn test_xml_prolog_unexpected_end_event_rejected() {
    let err = read_xml("</closing><root/>").unwrap_err();
    assert!(err.to_string().contains("invalid XML"));
}

#[test]
fn test_xml_epilog_unexpected_end_event_rejected() {
    let err = read_xml("<root/></closing>").unwrap_err();
    assert!(err.to_string().contains("invalid XML"));
}
#[test]
fn test_xml_prolog_general_ref_rejected() {
    let err = read_xml("&amp;<root/>").unwrap_err();
    assert!(
        err.to_string()
            .contains("unexpected text outside root element")
    );
}

#[test]
fn test_xml_epilog_general_ref_rejected() {
    let err = read_xml("<root/>&amp;").unwrap_err();
    assert!(
        err.to_string()
            .contains("unexpected text after root element")
    );
}
// ---------------------------------------------------------------------------
// Issue #114: XML schema-guided pretyping
// ---------------------------------------------------------------------------

#[test]
fn test_xml_pretype_boolean() {
    let schema_text = "record Data { \"b_true\": boolean, \"b_false\": boolean, \"b_invalid1\": boolean, \"b_invalid2\": boolean } record Root { \"data\": Data } root Root";
    let schema = crate::osd::parse_schema(schema_text).unwrap();
    let xml = "<data><b_true>true</b_true><b_false>false</b_false><b_invalid1>True</b_invalid1><b_invalid2>1</b_invalid2></data>";
    let doc = read_xml_with_schema(xml, &schema).unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![(
            "data",
            edges(vec![
                ("b_true", leaf_bool(true)),
                ("b_false", leaf_bool(false)),
                ("b_invalid1", leaf_str("True")),
                ("b_invalid2", leaf_str("1")),
            ])
        )])
    );
}

#[test]
fn test_xml_pretype_integer() {
    let schema_text = "record Data { \"zero\": integer, \"neg_zero\": integer, \"pos\": integer, \"neg\": integer, \"lead_zero\": integer, \"non_digit\": integer } record Root { \"data\": Data } root Root";
    let schema = crate::osd::parse_schema(schema_text).unwrap();
    let xml = "<data><zero>0</zero><neg_zero>-0</neg_zero><pos>42</pos><neg>-123</neg><lead_zero>007</lead_zero><non_digit>abc</non_digit></data>";
    let doc = read_xml_with_schema(xml, &schema).unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![(
            "data",
            edges(vec![
                ("zero", leaf(Scalar::Int((0).into()))),
                ("neg_zero", leaf(Scalar::Int((0).into()))),
                ("pos", leaf(Scalar::Int((42).into()))),
                ("neg", leaf(Scalar::Int((-123).into()))),
                ("lead_zero", leaf_str("007")),
                ("non_digit", leaf_str("abc")),
            ])
        )])
    );
}

#[test]
fn test_xml_pretype_number() {
    let schema_text = "record Data { \"num1\": number, \"num2\": number, \"exp1\": number, \"exp2\": number, \"invalid\": number } record Root { \"data\": Data } root Root";
    let schema = crate::osd::parse_schema(schema_text).unwrap();
    let xml = "<data><num1>3.14</num1><num2>-0.5</num2><exp1>1e6</exp1><exp2>-2.5E-3</exp2><invalid>1.2.3</invalid></data>";
    let doc = read_xml_with_schema(xml, &schema).unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![(
            "data",
            edges(vec![
                ("num1", leaf(Scalar::Float(3.14))),
                ("num2", leaf(Scalar::Float(-0.5))),
                ("exp1", leaf(Scalar::Float(1000000.0))),
                ("exp2", leaf(Scalar::Float(-0.0025))),
                ("invalid", leaf_str("1.2.3")),
            ])
        )])
    );
}

#[test]
fn test_xml_pretype_digit_cap_oversized_literal_stays_string() {
    use crate::formats::int_cap::MAX_INT_DIGITS;
    let schema_text = "record Data { \"big_int\": integer, \"big_num\": number } record Root { \"data\": Data } root Root";
    let schema = crate::osd::parse_schema(schema_text).unwrap();
    let oversized = "9".repeat(MAX_INT_DIGITS + 1);
    let xml =
        format!("<data><big_int>{oversized}</big_int><big_num>{oversized}.5</big_num></data>");
    let doc = read_xml_with_schema(&xml, &schema).unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![(
            "data",
            edges(vec![
                ("big_int", leaf_str(&oversized)),
                ("big_num", leaf_str(&format!("{oversized}.5"))),
            ])
        )])
    );
}

#[test]
fn test_xml_pretype_any_and_unknown_fields_untouched() {
    let schema_text = "record Data { \"any_field\": any, \"known\": integer } record Root { \"data\": Data } root Root";
    let schema = crate::osd::parse_schema(schema_text).unwrap();
    let xml = "<data><any_field><nested>42</nested><flag>true</flag></any_field><known>10</known><unknown_field>99</unknown_field></data>";
    let doc = read_xml_with_schema(xml, &schema).unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![(
            "data",
            edges(vec![
                (
                    "any_field",
                    edges(vec![("nested", leaf_str("42")), ("flag", leaf_str("true")),])
                ),
                ("known", leaf(Scalar::Int((10).into()))),
                ("unknown_field", leaf_str("99")),
            ])
        )])
    );
}

#[test]
fn test_xml_pretype_spec_order_worked_example() {
    let schema_osd = r#"
record Address  { "street": string, "city": string }
record LineItem { "sku": string, "qty": integer, "price": number }

record Order {
    "id":           string,
    "status":       string,
    "total":        number,
    "address":      Address,
    "items" [1,]:   LineItem,
    "coupon" [0,1]: string,
}

record Root { "order": Order }
root Root
"#;
    let schema = crate::osd::parse_schema(schema_osd).unwrap();
    let xml = r#"<order>
  <id>A1</id>
  <status>shipped</status>
  <total>29.97</total>
  <address><street>1 Main</street><city>London</city></address>
  <items><sku>W</sku><qty>3</qty><price>9.99</price></items>
  <items><sku>G</sku><qty>1</qty><price>9.99</price></items>
</order>"#;

    let doc = read_xml_with_schema(xml, &schema).unwrap();
    assert_eq!(
        doc.to_raw(),
        edges(vec![(
            "order",
            edges(vec![
                ("id", leaf_str("A1")),
                ("status", leaf_str("shipped")),
                ("total", leaf(Scalar::Float(29.97))),
                (
                    "address",
                    edges(vec![
                        ("street", leaf_str("1 Main")),
                        ("city", leaf_str("London")),
                    ])
                ),
                (
                    "items",
                    edges(vec![
                        ("sku", leaf_str("W")),
                        ("qty", leaf(Scalar::Int((3).into()))),
                        ("price", leaf(Scalar::Float(9.99))),
                    ])
                ),
                (
                    "items",
                    edges(vec![
                        ("sku", leaf_str("G")),
                        ("qty", leaf(Scalar::Int((1).into()))),
                        ("price", leaf(Scalar::Float(9.99))),
                    ])
                ),
            ])
        )])
    );

    // Validate passes cleanly
    assert!(schema.validate(&doc.root()).ok());
}

#[test]
fn test_plain_read_xml_unaffected_by_schema_pretyping() {
    let xml = "<order><qty>3</qty><total>29.97</total><flag>true</flag></order>";
    let doc = read_xml(xml).unwrap();
    // Plain read_xml always yields strings
    assert_eq!(
        doc.to_raw(),
        edges(vec![(
            "order",
            edges(vec![
                ("qty", leaf_str("3")),
                ("total", leaf_str("29.97")),
                ("flag", leaf_str("true")),
            ])
        )])
    );
}

#[test]
fn test_xml_pretype_whitebox_non_string_and_non_edges() {
    let schema = crate::osd::parse_schema("record Root { \"x\": integer } root Root").unwrap();
    // Already non-str leaf stays untouched
    let leaf_node = leaf(Scalar::Int((5).into()));
    let pretyped_leaf = super::xml_pretype(
        leaf_node.clone(),
        &schema,
        &crate::schema::FieldType::Scalar(crate::schema::INTEGER),
    );
    assert_eq!(pretyped_leaf, leaf_node);

    // Leaf passed to Record type resolution returns node
    let pretyped_rec_on_leaf = super::xml_pretype(
        leaf_node.clone(),
        &schema,
        &crate::schema::FieldType::Ref(schema.root().clone()),
    );
    assert_eq!(pretyped_rec_on_leaf, leaf_node);
}
#[test]
fn test_xml_pretype_read_xml_with_schema_parse_error() {
    let schema = crate::osd::parse_schema("record Root { \"x\": integer } root Root").unwrap();
    let err = read_xml_with_schema("<unclosed>", &schema).unwrap_err();
    assert!(err.to_string().contains("invalid XML"));
}
