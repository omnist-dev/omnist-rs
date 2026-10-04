//! Runtime-configurable safety limits (omnist-rs#181) and the Document-path
//! form of limit diagnostics (omnist-rs#182, item 1; spec E-11): every format
//! at the boundary and one past, with the exact code and path.

use omnist::document::{Doc, RawNode, Scalar, Value};
use omnist::error::OmnistError;
use omnist::formats::json::{read_json, read_json_with};
use omnist::formats::toml::{read_toml, read_toml_with};
use omnist::formats::xml::{read_xml, read_xml_with};
use omnist::formats::yaml::{YamlReadOptions, read_yaml, read_yaml_with};
use omnist::limits::{
    DEFAULT_MAX_DEPTH, DEFAULT_MAX_INT_DIGITS, DEFAULT_MAX_NODES, Limits, MAX_DEPTH_CEILING,
    MAX_INT_DIGITS_CEILING, MAX_NODES_CEILING,
};
use omnist::oml::{read_oml, read_oml_with};

const DEPTH: &str = "document.limit.depth";
const NODES: &str = "document.limit.nodes";
const DIGITS: &str = "document.limit.int-digits";

fn depth3() -> Limits {
    Limits::default().with_max_depth(3)
}
fn nodes1() -> Limits {
    Limits::default().with_max_nodes(1)
}
fn digits3() -> Limits {
    Limits::default().with_max_int_digits(3)
}

/// `(code, path)` of a Document error; panics on any other error kind.
fn diag(e: OmnistError) -> (String, String) {
    match e {
        OmnistError::Document(d) => (d.code.clone().unwrap_or_default(), d.path.clone()),
        other => panic!("expected a DocumentError, got {other:?}"),
    }
}

fn want(code: &str, path: &str) -> (String, String) {
    (code.to_string(), path.to_string())
}

fn oml_err(text: &str, l: &Limits) -> (String, String) {
    diag(read_oml_with(text, l).unwrap_err())
}
fn json_err(text: &str, l: &Limits) -> (String, String) {
    diag(read_json_with(text, l).unwrap_err())
}
fn toml_err(text: &str, l: &Limits) -> (String, String) {
    diag(read_toml_with(text, l).unwrap_err())
}
fn yaml_err(text: &str, l: &Limits) -> (String, String) {
    diag(read_yaml_with(text, &YamlReadOptions::default().with_limits(*l)).unwrap_err())
}
fn xml_err(text: &str, l: &Limits) -> (String, String) {
    diag(read_xml_with(text, l).unwrap_err())
}
fn yaml_ok(text: &str, l: &Limits) -> Doc {
    read_yaml_with(text, &YamlReadOptions::default().with_limits(*l)).unwrap()
}

// ---------------------------------------------------------------- Limits

#[test]
fn defaults_are_the_spec_reference_values() {
    let l = Limits::default();
    assert_eq!(
        (l.max_depth, l.max_nodes, l.max_int_digits),
        (200, 1_000_000, 4_300)
    );
    assert_eq!(
        (DEFAULT_MAX_DEPTH, DEFAULT_MAX_NODES, DEFAULT_MAX_INT_DIGITS),
        (200, 1_000_000, 4_300)
    );
    assert_eq!(omnist::document::MAX_DEPTH, 200);
    assert_eq!(omnist::document::MAX_NODES, 1_000_000);
}

#[test]
fn zero_selects_the_default_for_each_limit() {
    let zero = Limits::default()
        .with_max_depth(0)
        .with_max_nodes(0)
        .with_max_int_digits(0);
    assert_eq!(zero.effective_max_depth(), 200);
    assert_eq!(zero.effective_max_nodes(), 1_000_000);
    assert_eq!(zero.effective_max_int_digits(), 4_300);
    assert!(zero.validate().is_ok());
    let set = Limits::default()
        .with_max_depth(7)
        .with_max_nodes(8)
        .with_max_int_digits(9);
    assert_eq!(
        (
            set.effective_max_depth(),
            set.effective_max_nodes(),
            set.effective_max_int_digits()
        ),
        (7, 8, 9)
    );
}

#[test]
fn a_zero_limit_reads_with_the_default_never_unbounded() {
    // 4301 digits is over the default cap, so `0` did not widen it.
    let text = format!("n: {}\n", "9".repeat(4301));
    let zero = Limits::default().with_max_int_digits(0);
    assert_eq!(oml_err(&text, &zero), want(DIGITS, "$.n"));
    // And a 200-deep document is accepted, 201 refused, under `0` depth.
    let zero = Limits::default().with_max_depth(0);
    assert!(read_oml_with(&nested_oml(200), &zero).is_ok());
    assert_eq!(oml_err(&nested_oml(201), &zero), want(DEPTH, "$"));
}

/// `a: { a: { ... n: 1 } }` with `levels` braces: the scalar sits at depth
/// `levels + 1`... counted from the root, `levels` nested nodes below it.
fn nested_oml(levels: usize) -> String {
    let mut s = String::new();
    for _ in 0..levels - 1 {
        s.push_str("a: { ");
    }
    s.push_str("n: 1");
    for _ in 0..levels - 1 {
        s.push_str(" }");
    }
    s
}

#[test]
fn ceilings_are_accepted_and_one_past_is_refused_not_clamped() {
    let at = Limits::default()
        .with_max_depth(MAX_DEPTH_CEILING)
        .with_max_nodes(MAX_NODES_CEILING)
        .with_max_int_digits(MAX_INT_DIGITS_CEILING);
    assert!(at.validate().is_ok());
    for (bad, name) in [
        (
            Limits::default().with_max_depth(MAX_DEPTH_CEILING + 1),
            "max_depth 1001 exceeds the ceiling 1000",
        ),
        (
            Limits::default().with_max_nodes(MAX_NODES_CEILING + 1),
            "max_nodes 10000001 exceeds the ceiling 10000000",
        ),
        (
            Limits::default().with_max_int_digits(MAX_INT_DIGITS_CEILING + 1),
            "max_int_digits 43001 exceeds the ceiling 43000",
        ),
    ] {
        let e = bad.validate().unwrap_err();
        assert_eq!((e.path.as_str(), e.code.as_deref()), ("$", None));
        assert!(e.message.contains(name), "{}", e.message);
    }
}

#[test]
fn every_entry_point_returns_the_validate_error_unchanged() {
    let bad = Limits::default().with_max_depth(MAX_DEPTH_CEILING + 1);
    let expected = bad.validate().unwrap_err();
    let raw = RawNode::Edges(vec![]);
    let results: Vec<OmnistError> = vec![
        read_oml_with("a: 1", &bad).unwrap_err(),
        read_json_with("{}", &bad).unwrap_err(),
        read_toml_with("", &bad).unwrap_err(),
        read_xml_with("<a/>", &bad).unwrap_err(),
        read_yaml_with("a: 1", &YamlReadOptions::default().with_limits(bad)).unwrap_err(),
        Doc::from_raw_with(raw, &bad).unwrap_err().into(),
        Doc::of_with(&Value::Null, &bad).unwrap_err().into(),
    ];
    for r in results {
        assert_eq!(diag_full(r), expected);
    }
    let opts = YamlReadOptions::default().with_limits(bad);
    assert_eq!(opts.validate().unwrap_err(), expected);
}

fn diag_full(e: OmnistError) -> omnist::error::DocumentError {
    match e {
        OmnistError::Document(d) => d,
        other => panic!("expected a DocumentError, got {other:?}"),
    }
}

// ------------------------------------------------------------------- OML

#[test]
fn oml_depth_at_limit_accepted_one_past_refused_at_the_root() {
    assert!(read_oml_with("a: { b: { c: 1 } }", &depth3()).is_ok());
    assert_eq!(
        oml_err("a: { b: { c: { d: 1 } } }", &depth3()),
        want(DEPTH, "$")
    );
    // An array element is one level below its label, like a plain value.
    assert!(read_oml_with("a: { b: [1, { c: 1 }] }", &depth3()).is_ok());
    assert_eq!(
        oml_err("a: { b: [1, { c: { d: 1 } }] }", &depth3()),
        want(DEPTH, "$")
    );
}

#[test]
fn oml_node_count_counts_containers_not_scalars() {
    assert!(read_oml_with("a: 1\nb: 2\n", &nodes1()).is_ok());
    assert!(read_oml_with("1", &nodes1()).is_ok());
    assert!(read_oml_with("", &nodes1()).is_ok());
    assert_eq!(oml_err("a: { b: 1 }", &nodes1()), want(NODES, "$"));
    let two = Limits::default().with_max_nodes(2);
    assert!(read_oml_with("a: { b: 1 }", &two).is_ok());
    assert!(read_oml_with("a: [{ x: 1 }]", &two).is_ok());
    assert_eq!(oml_err("a: [{ x: 1 }, { y: 2 }]", &two), want(NODES, "$"));
}

#[test]
fn oml_integer_digits_at_limit_accepted_one_past_refused_at_the_integer() {
    assert!(read_oml_with("n: 999\n", &digits3()).is_ok());
    assert!(read_oml_with("n: -999\n", &digits3()).is_ok());
    assert_eq!(oml_err("n: 1000\n", &digits3()), want(DIGITS, "$.n"));
    assert_eq!(oml_err("n: -1000\n", &digits3()), want(DIGITS, "$.n"));
    // Not the first token of the input, nested, quoted label.
    assert_eq!(
        oml_err("a: 1\nb: { n: 1000 }\n", &digits3()),
        want(DIGITS, "$.b.n")
    );
    assert_eq!(
        oml_err("\"a b\": 1000\n", &digits3()),
        want(DIGITS, "$[\"a b\"]")
    );
    // A bare integer document.
    assert_eq!(oml_err("1000", &digits3()), want(DIGITS, "$"));
}

#[test]
fn oml_integer_path_indexes_a_repeated_label_and_only_then() {
    // E-10: the index is present when the label occurs more than once in the
    // node, absent when it occurs once.
    assert_eq!(oml_err("n: [1000]", &digits3()), want(DIGITS, "$.n"));
    assert_eq!(oml_err("n: [1, 1000]", &digits3()), want(DIGITS, "$.n[1]"));
    assert_eq!(oml_err("n: [1000, 1]", &digits3()), want(DIGITS, "$.n[0]"));
    assert_eq!(oml_err("n: 1\nn: 1000", &digits3()), want(DIGITS, "$.n[1]"));
    assert_eq!(oml_err("n: 1000\nn: 1", &digits3()), want(DIGITS, "$.n[0]"));
    // Another label between the two occurrences does not renumber them.
    assert_eq!(
        oml_err("n: 1\nm: 1\nn: 1000", &digits3()),
        want(DIGITS, "$.n[1]")
    );
    assert_eq!(
        oml_err("t: [{ n: 1 }, { n: 1000 }]", &digits3()),
        want(DIGITS, "$.t[1].n")
    );
}

#[test]
fn oml_first_over_cap_integer_in_document_order_is_reported() {
    assert_eq!(
        oml_err("a: 1000\nb: 10000\n", &digits3()),
        want(DIGITS, "$.a")
    );
}

#[test]
fn oml_syntax_error_after_an_over_cap_integer_still_wins() {
    let e = read_oml_with("n: 1000\nm: }", &digits3()).unwrap_err();
    assert!(matches!(e, OmnistError::Parse(_)), "{e:?}");
}

#[test]
fn oml_default_limits_over_the_default_cap() {
    let at = format!("n: {}\n", "9".repeat(4300));
    assert!(read_oml_with(&at, &Limits::default()).is_ok());
    let past = format!("n: {}\n", "9".repeat(4301));
    assert_eq!(oml_err(&past, &Limits::default()), want(DIGITS, "$.n"));
    // A raised limit accepts what the default refuses, up to the ceiling.
    let raised = Limits::default().with_max_int_digits(5000);
    assert!(read_oml_with(&past, &raised).is_ok());
}

#[test]
fn read_oml_keeps_its_signature_and_reports_limits_as_a_positioned_parse_error() {
    // `read_oml` is unchanged: a `ParseError` with the text position.
    let past = format!("n: {}\n", "9".repeat(4301));
    let e = read_oml(&past).unwrap_err();
    assert_eq!((e.code.as_str(), e.line, e.col), (DIGITS, 1, 4));
    let mut deep = String::new();
    for _ in 0..201 {
        deep.push_str("a: { ");
    }
    deep.push('n');
    let e = read_oml(&deep).unwrap_err();
    assert_eq!(e.code, DEPTH);
    assert_eq!(e.line, 1);
    let mut many = String::new();
    for _ in 0..1_000_000 {
        many.push_str("a: {}\n");
    }
    let e = read_oml(&many).unwrap_err();
    assert_eq!(e.code, NODES);
}

// ------------------------------------------------------------------ JSON

#[test]
fn json_depth_at_limit_accepted_one_past_refused_at_the_root() {
    assert!(read_json_with(r#"{"a":{"b":{"c":1}}}"#, &depth3()).is_ok());
    assert_eq!(
        json_err(r#"{"a":{"b":{"c":{"d":1}}}}"#, &depth3()),
        want(DEPTH, "$")
    );
    // An array is a level of nesting.
    assert!(read_json_with(r#"{"a":{"b":[1]}}"#, &depth3()).is_ok());
    assert_eq!(
        json_err(r#"{"a":{"b":[[1]]}}"#, &depth3()),
        want(DEPTH, "$")
    );
    assert_eq!(
        json_err(r#"{"a":[{"b":{"c":1}}]}"#, &depth3()),
        want(DEPTH, "$")
    );
}

#[test]
fn json_node_count_counts_containers_and_reports_at_the_root() {
    assert!(read_json_with(r#"{"a":1,"b":[1,2,3],"c":"x"}"#, &nodes1()).is_ok());
    assert_eq!(json_err(r#"{"a":{"b":1}}"#, &nodes1()), want(NODES, "$"));
    let two = Limits::default().with_max_nodes(2);
    assert!(read_json_with(r#"{"a":[{"x":1}]}"#, &two).is_ok());
    assert_eq!(
        json_err(r#"{"a":[{"x":1},{"y":2}]}"#, &two),
        want(NODES, "$")
    );
}

#[test]
fn json_integer_digits_at_limit_accepted_one_past_refused_at_the_integer() {
    assert!(read_json_with(r#"{"n":999,"m":-999}"#, &digits3()).is_ok());
    assert_eq!(json_err(r#"{"n":1000}"#, &digits3()), want(DIGITS, "$.n"));
    assert_eq!(json_err(r#"{"n":-1000}"#, &digits3()), want(DIGITS, "$.n"));
    assert_eq!(
        json_err(r#"{"a":{"n":1000}}"#, &digits3()),
        want(DIGITS, "$.a.n")
    );
    assert_eq!(json_err("1000", &digits3()), want(DIGITS, "$"));
    // E-10 indexes.
    assert_eq!(json_err(r#"{"n":[1000]}"#, &digits3()), want(DIGITS, "$.n"));
    assert_eq!(
        json_err(r#"{"n":[1,1000]}"#, &digits3()),
        want(DIGITS, "$.n[1]")
    );
    assert_eq!(
        json_err(r#"{"n":[1000,1]}"#, &digits3()),
        want(DIGITS, "$.n[0]")
    );
}

#[test]
fn json_over_cap_integer_dropped_by_a_duplicate_key_is_still_refused() {
    // Duplicate keys keep the last value, which would otherwise erase the
    // over-cap literal from the tree and accept the input.
    assert_eq!(
        json_err(r#"{"n":1000,"n":1}"#, &digits3()),
        want(DIGITS, "$")
    );
    assert_eq!(
        json_err(r#"{"n":[1000],"n":1}"#, &digits3()),
        want(DIGITS, "$")
    );
}

#[test]
fn json_default_limits_over_the_default_cap() {
    let at = format!(r#"{{"n":{}}}"#, "9".repeat(4300));
    assert!(read_json(&at).is_ok());
    let past = format!(r#"{{"n":{}}}"#, "9".repeat(4301));
    assert_eq!(diag(read_json(&past).unwrap_err()), want(DIGITS, "$.n"));
    assert!(read_json_with(&past, &Limits::default().with_max_int_digits(5000)).is_ok());
}

// ------------------------------------------------------------------ TOML

#[test]
fn toml_depth_at_limit_accepted_one_past_refused_at_the_root() {
    assert!(read_toml_with("a = { b = { c = 1 } }", &depth3()).is_ok());
    assert!(read_toml_with("[a.b]\nc = 1\n", &depth3()).is_ok());
    assert_eq!(
        toml_err("a = { b = { c = { d = 1 } } }", &depth3()),
        want(DEPTH, "$")
    );
    assert_eq!(toml_err("[a.b.c]\nd = 1\n", &depth3()), want(DEPTH, "$"));
}

#[test]
fn toml_node_count_counts_containers_and_reports_at_the_root() {
    assert!(read_toml_with("a = 1\nb = [1, 2]\n", &nodes1()).is_ok());
    assert_eq!(toml_err("[a]\nx = 1\n", &nodes1()), want(NODES, "$"));
}

#[test]
fn toml_integer_digits_at_limit_accepted_one_past_refused_at_the_integer() {
    assert!(read_toml_with("n = 999\nm = -999\n", &digits3()).is_ok());
    assert_eq!(toml_err("n = 1000\n", &digits3()), want(DIGITS, "$.n"));
    assert_eq!(
        toml_err("[t]\nn = 1000\n", &digits3()),
        want(DIGITS, "$.t.n")
    );
    assert_eq!(
        toml_err("n = [1, 1000]\n", &digits3()),
        want(DIGITS, "$.n[1]")
    );
    assert_eq!(toml_err("n = [1000]\n", &digits3()), want(DIGITS, "$.n"));
    assert_eq!(
        toml_err("[[t]]\nn = 1\n[[t]]\nn = 1000\n", &digits3()),
        want(DIGITS, "$.t[1].n")
    );
    assert_eq!(
        toml_err("[[t]]\nn = 1000\n", &digits3()),
        want(DIGITS, "$.t.n")
    );
}

#[test]
fn toml_literal_past_i64_and_past_the_cap_is_refused_at_its_path() {
    // toml_edit itself refuses these (no tree), so the path is recovered.
    let big = "99999999999999999999"; // 20 digits
    let l = Limits::default().with_max_int_digits(10);
    assert_eq!(toml_err(&format!("n = {big}\n"), &l), want(DIGITS, "$.n"));
    assert_eq!(
        toml_err(&format!("[t]\nn = {big}\n"), &l),
        want(DIGITS, "$.t.n")
    );
    assert_eq!(
        toml_err(&format!("n = [1, {big}]\n"), &l),
        want(DIGITS, "$.n[1]")
    );
    assert_eq!(toml_err(&format!("n = [{big}]\n"), &l), want(DIGITS, "$.n"));
    assert_eq!(
        toml_err(&format!("a = {{ b = {{ n = {big} }} }}\n"), &l),
        want(DIGITS, "$.a.b.n")
    );
    assert_eq!(
        toml_err(&format!("[[t]]\nn = 1\n[[t]]\nn = {big}\n"), &l),
        want(DIGITS, "$.t[1].n")
    );
    assert_eq!(
        toml_err(&format!("[[t]]\nn = {big}\n"), &l),
        want(DIGITS, "$.t.n")
    );
    // A hex literal counts its digits too.
    assert_eq!(
        toml_err("n = 0xFFFFFFFFFFFFFFFFFF\n", &l),
        want(DIGITS, "$.n")
    );
    // The first in document order.
    assert_eq!(
        toml_err(&format!("a = {big}\nb = {big}\n"), &l),
        want(DIGITS, "$.a")
    );
    // A label that needs quoting.
    assert_eq!(
        toml_err(&format!("\"a b\" = {big}\n"), &l),
        want(DIGITS, "$[\"a b\"]")
    );
}

#[test]
fn toml_syntax_or_range_error_after_an_over_cap_literal_still_wins() {
    let big = "99999999999999999999";
    let l = Limits::default().with_max_int_digits(10);
    let e = read_toml_with(&format!("a = {big}\nb = \n"), &l).unwrap_err();
    assert!(matches!(e, OmnistError::Parse(ref p) if p.code == "parse.codec-syntax"));
    // A later literal that is out of range for i64 but within the cap is the
    // existing out-of-range error, not the limit.
    let wide = Limits::default().with_max_int_digits(25);
    let thirty = "9".repeat(30);
    let e = read_toml_with(&format!("a = {thirty}\nb = {big}\n"), &wide).unwrap_err();
    assert!(
        matches!(&e, OmnistError::Parse(p) if p.message.contains("out of range")),
        "{e:?}"
    );
    // Several over-cap literals followed by a clean end: the first is named.
    assert_eq!(
        toml_err(
            &format!("a = {thirty}\nb = {thirty}\nc = {thirty}\n"),
            &wide
        ),
        want(DIGITS, "$.a")
    );
}

#[test]
fn toml_literal_exactly_at_the_cap_but_past_i64_is_out_of_range_not_a_limit() {
    // 20 digits: at a cap of 20 it is within the limit but still too big for
    // toml_edit's 64-bit integers; at a cap of 19 it is over the limit.
    let text = "n = 99999999999999999999\n";
    let at = Limits::default().with_max_int_digits(20);
    let e = read_toml_with(text, &at).unwrap_err();
    assert!(
        matches!(&e, OmnistError::Parse(p) if p.message.contains("out of range")),
        "{e:?}"
    );
    let past = Limits::default().with_max_int_digits(19);
    assert_eq!(toml_err(text, &past), want(DIGITS, "$.n"));
}

#[test]
fn toml_default_limits_over_the_default_cap() {
    let past = format!("n = {}\n", "9".repeat(4301));
    assert_eq!(diag(read_toml(&past).unwrap_err()), want(DIGITS, "$.n"));
}

// ------------------------------------------------------------------ YAML

#[test]
fn yaml_depth_at_limit_accepted_one_past_refused_at_the_root() {
    assert!(
        yaml_ok("a:\n  b:\n    c: 1\n", &depth3())
            .root()
            .child("a")
            .is_ok()
    );
    assert_eq!(
        yaml_err("a:\n  b:\n    c:\n      d: 1\n", &depth3()),
        want(DEPTH, "$")
    );
    assert!(
        read_yaml_with(
            "a: { b: { c: 1 } }",
            &YamlReadOptions::default().with_limits(depth3())
        )
        .is_ok()
    );
    assert_eq!(
        yaml_err("a: { b: { c: { d: 1 } } }", &depth3()),
        want(DEPTH, "$")
    );
    // Nesting far past the limit is caught while the tree is still shallow.
    let mut deep = String::new();
    for i in 0..60 {
        deep.push_str(&"  ".repeat(i));
        deep.push_str("a:\n");
    }
    assert_eq!(yaml_err(&deep, &depth3()), want(DEPTH, "$"));
    // Anchors do not smuggle in depth.
    assert_eq!(
        yaml_err("x: &x { b: { c: 1 } }\na: { y: *x }\n", &depth3()),
        want(DEPTH, "$")
    );
}

#[test]
fn yaml_node_count_counts_containers_and_reports_at_the_root() {
    assert!(
        read_yaml_with(
            "a: 1\nb: [1, 2]\n",
            &YamlReadOptions::default().with_limits(Limits::default().with_max_nodes(2))
        )
        .is_ok()
    );
    assert_eq!(yaml_err("a:\n  b: 1\n", &nodes1()), want(NODES, "$"));
}

#[test]
fn yaml_integer_digits_at_limit_accepted_one_past_refused_at_the_integer() {
    assert!(
        read_yaml_with(
            "n: 999\nm: -999\n",
            &YamlReadOptions::default().with_limits(digits3())
        )
        .is_ok()
    );
    assert_eq!(yaml_err("n: 1000\n", &digits3()), want(DIGITS, "$.n"));
    assert_eq!(yaml_err("n: -1000\n", &digits3()), want(DIGITS, "$.n"));
    assert_eq!(
        yaml_err("a:\n  n: 1000\n", &digits3()),
        want(DIGITS, "$.a.n")
    );
    assert_eq!(
        yaml_err("n: [1, 1000]\n", &digits3()),
        want(DIGITS, "$.n[1]")
    );
    assert_eq!(yaml_err("n: [1000]\n", &digits3()), want(DIGITS, "$.n"));
    assert_eq!(yaml_err("n: !!int 1000\n", &digits3()), want(DIGITS, "$.n"));
    // A sexagesimal literal's value is what is measured.
    assert_eq!(yaml_err("n: 1:59:59\n", &digits3()), want(DIGITS, "$.n"));
    // An over-long integer used as a key is refused by the digit limit.
    assert_eq!(yaml_err("1000: x\n", &digits3()), want(DIGITS, "$"));
}

#[test]
fn yaml_over_cap_integer_dropped_by_a_duplicate_key_is_still_refused() {
    assert_eq!(yaml_err("n: 1000\nn: 1\n", &digits3()), want(DIGITS, "$"));
}

#[test]
fn yaml_default_limits_over_the_default_cap() {
    let past = format!("n: {}\n", "9".repeat(4301));
    assert_eq!(diag(read_yaml(&past).unwrap_err()), want(DIGITS, "$.n"));
}

// ------------------------------------------------------------------- XML

#[test]
fn xml_depth_at_limit_accepted_one_past_refused_at_the_root() {
    assert!(read_xml_with("<a><b><c>x</c></b></a>", &depth3()).is_ok());
    assert_eq!(
        xml_err("<a><b><c><d>x</d></c></b></a>", &depth3()),
        want(DEPTH, "$")
    );
}

#[test]
fn xml_node_count_counts_containers_and_reports_at_the_root() {
    assert!(
        read_xml_with(
            "<r><a>1</a><b/><c>2</c></r>",
            &Limits::default().with_max_nodes(2)
        )
        .is_ok()
    );
    assert_eq!(
        xml_err("<r><a><b/></a></r>", &Limits::default().with_max_nodes(2)),
        want(NODES, "$")
    );
    assert!(read_xml("<r><a><b/></a></r>").is_ok());
}

// ------------------------------------------------- Doc::of_with / from_raw

fn nest_raw(levels: usize) -> RawNode {
    let mut node = RawNode::Leaf(Scalar::Int(1.into()));
    for _ in 0..levels {
        node = RawNode::Edges(vec![("a".to_string(), node)]);
    }
    node
}

#[test]
fn from_raw_with_enforces_each_limit_at_the_boundary() {
    // The leaf of nest_raw(n) is at depth n.
    assert!(Doc::from_raw_with(nest_raw(3), &depth3()).is_ok());
    let e = Doc::from_raw_with(nest_raw(4), &depth3()).unwrap_err();
    assert_eq!((e.code.as_deref(), e.path.as_str()), (Some(DEPTH), "$"));

    // nest_raw(1) is two containers? No: one (the root); nest_raw(2) is two.
    assert!(Doc::from_raw_with(nest_raw(1), &nodes1()).is_ok());
    let e = Doc::from_raw_with(nest_raw(2), &nodes1()).unwrap_err();
    assert_eq!((e.code.as_deref(), e.path.as_str()), (Some(NODES), "$"));

    let leaf = |i: i64| RawNode::Leaf(Scalar::Int(i.into()));
    let raw = |i: i64| RawNode::Edges(vec![("n".to_string(), leaf(i))]);
    assert!(Doc::from_raw_with(raw(999), &digits3()).is_ok());
    let e = Doc::from_raw_with(raw(1000), &digits3()).unwrap_err();
    assert_eq!((e.code.as_deref(), e.path.as_str()), (Some(DIGITS), "$.n"));
    let e = Doc::from_raw_with(leaf(1000), &digits3()).unwrap_err();
    assert_eq!((e.code.as_deref(), e.path.as_str()), (Some(DIGITS), "$"));
}

#[test]
fn from_raw_with_names_a_repeated_label_and_quotes_an_odd_one() {
    let leaf = |i: i64| RawNode::Leaf(Scalar::Int(i.into()));
    let edges = |pairs: Vec<(&str, i64)>| {
        RawNode::Edges(
            pairs
                .into_iter()
                .map(|(l, i)| (l.to_string(), leaf(i)))
                .collect(),
        )
    };
    let path = |raw| Doc::from_raw_with(raw, &digits3()).unwrap_err().path;
    assert_eq!(path(edges(vec![("n", 1), ("n", 1000)])), "$.n[1]");
    assert_eq!(path(edges(vec![("n", 1000), ("m", 1), ("n", 1)])), "$.n[0]");
    assert_eq!(path(edges(vec![("a b", 1000)])), "$[\"a b\"]");
}

#[test]
fn of_with_enforces_the_limits() {
    let obj = |v: Value| Value::Object([("a".to_string(), v)].into_iter().collect());
    assert!(Doc::of_with(&obj(obj(obj(Value::Null))), &depth3()).is_ok());
    let e = Doc::of_with(&obj(obj(obj(obj(Value::Null)))), &depth3()).unwrap_err();
    assert_eq!(e.code.as_deref(), Some(DEPTH));
    assert!(Doc::of_with(&obj(Value::Int(999.into())), &digits3()).is_ok());
    let e = Doc::of_with(&obj(Value::Int(1000.into())), &digits3()).unwrap_err();
    assert_eq!((e.code.as_deref(), e.path.as_str()), (Some(DIGITS), "$.a"));
    let e = Doc::of_with(&obj(obj(Value::Null)), &nodes1()).unwrap_err();
    assert_eq!(e.code.as_deref(), Some(NODES));
    // Programmatic construction is bounded too: every route into the model.
    let huge = Value::Int(num_bigint::BigInt::from(10u8).pow(4300));
    assert_eq!(
        Doc::of(&obj(huge)).unwrap_err().code.as_deref(),
        Some(DIGITS)
    );
}

#[test]
fn a_document_remembers_the_limits_it_was_built_under() {
    // Built with a depth of 300, a later add at depth 250 is within its own
    // limit; a default-limit document refuses the same add.
    let wide = Limits::default().with_max_depth(300);
    let mut doc = Doc::from_raw_with(nest_raw(250), &wide).unwrap();
    let mut default_doc = Doc::from_raw(nest_raw(200)).unwrap();
    let deepest = |d: &Doc, levels: usize| {
        let mut c = d.root();
        for _ in 0..levels - 1 {
            c = c.child("a").unwrap();
        }
        (c.id(), c.path.clone())
    };
    let (id, path) = deepest(&doc, 250);
    assert!(
        doc.add(id, &path, "b", &Value::Object(Default::default()))
            .is_ok()
    );
    let (id, path) = deepest(&default_doc, 200);
    let e = default_doc
        .add(id, &path, "b", &Value::Object(Default::default()))
        .and_then(|cid| {
            default_doc.add(
                cid,
                &format!("{path}.b"),
                "c",
                &Value::Object(Default::default()),
            )
        })
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some(DEPTH));
}

#[test]
fn node_cap_counts_a_mutation_added_container() {
    let mut doc = Doc::of_with(
        &Value::Object(Default::default()),
        &Limits::default().with_max_nodes(2),
    )
    .unwrap();
    let root = doc.root();
    let (id, path) = (root.id(), root.path.clone());
    assert!(
        doc.add(id, &path, "a", &Value::Object(Default::default()))
            .is_ok()
    );
    let e = doc
        .add(id, &path, "b", &Value::Object(Default::default()))
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some(NODES));
    // A scalar edge is not a node.
    assert!(doc.add(id, &path, "c", &Value::Int(1.into())).is_ok());
}
