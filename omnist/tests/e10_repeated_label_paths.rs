//! E-10 (omnist-spec v0.30.0-beta): a label that occurs more than once in a
//! node is indexed on EVERY occurrence, the first included (`$.a[0]`,
//! `$.a[1]`); a label that occurs once carries no index. The count is of the
//! node's edges and is taken per node. Every path producer is covered here:
//! validate, materialize, `Cursor::edges`, the XML reader's report, and the
//! check/write scans of the codecs.

use omnist::document::{Doc, RawNode, Scalar};
use omnist::formats::json::check_json;
use omnist::formats::toml::check_toml;
use omnist::formats::xml::{check_xml, read_xml_report};
use omnist::osd::parse_schema;
use omnist::report::WriteReport;

fn leaf(s: &str) -> RawNode {
    RawNode::Leaf(Scalar::Str(s.to_string()))
}

fn edges(pairs: Vec<(&str, RawNode)>) -> RawNode {
    RawNode::Edges(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn paths(report: &WriteReport) -> Vec<String> {
    report.iter().map(|a| a.path.clone()).collect()
}

/// (name, edge labels of the root, the paths a fully repeated `x` takes).
/// Interleaved labels are counted per node, not per run.
const SHAPES: &[(&str, &[&str], &[&str])] = &[
    ("single", &["x"], &["$.x"]),
    ("pair", &["x", "x"], &["$.x[0]", "$.x[1]"]),
    ("triple", &["x", "x", "x"], &["$.x[0]", "$.x[1]", "$.x[2]"]),
    (
        "interleaved",
        &["x", "y", "x"],
        &["$.x[0]", "$.x[1]"], // y is not in the list below: see per-test use
    ),
];

#[test]
fn validate_indexes_every_occurrence_of_a_repeated_label() {
    let schema = parse_schema("record R {\n    \"y\" [0,]: string,\n}\nroot R\n").unwrap();
    for (name, labels, expect) in SHAPES {
        let root = edges(labels.iter().map(|l| (*l, leaf("v"))).collect());
        let doc = Doc::from_raw(root).unwrap();
        let res = schema.validate(&doc.root());
        let got: Vec<String> = res
            .errors()
            .iter()
            .filter(|e| e.message == "unexpected field")
            .map(|e| e.path.clone())
            .collect();
        assert_eq!(&got, expect, "{name}");
    }
}

#[test]
fn materialize_indexes_every_occurrence_of_a_repeated_label() {
    let schema = parse_schema("record R {\n    \"y\" [0,]: string,\n}\nroot R\n").unwrap();
    for (name, labels, expect) in SHAPES {
        let root = edges(labels.iter().map(|l| (*l, leaf("v"))).collect());
        let err = omnist::materialize(&root, Some(&schema));
        // An unexpected field is a materialize error; its report carries
        // every path.
        let msg = err.unwrap_err().to_string();
        for p in *expect {
            assert!(msg.contains(p), "{name}: {p} missing from {msg}");
        }
        if labels.len() == 1 {
            assert!(!msg.contains("$.x["), "{name}: {msg}");
        }
    }
}

#[test]
fn cursor_edges_index_every_occurrence_of_a_repeated_label() {
    for (name, labels, expect) in SHAPES {
        // Each x holds an internal node, so `value()` fails with its path.
        let root = edges(
            labels
                .iter()
                .map(|l| (*l, edges(vec![("k", leaf("v"))])))
                .collect(),
        );
        let doc = Doc::from_raw(root).unwrap();
        let got: Vec<String> = doc
            .root()
            .edges()
            .unwrap()
            .into_iter()
            .filter(|(l, _)| l == "x")
            .map(|(_, c)| c.value().unwrap_err().path)
            .collect();
        assert_eq!(&got, expect, "{name}");
    }
}

#[test]
fn toml_check_indexes_the_first_occurrence_of_a_repeated_null() {
    let doc = Doc::from_raw(edges(vec![
        ("a", RawNode::Leaf(Scalar::Null)),
        ("a", RawNode::Leaf(Scalar::Null)),
    ]))
    .unwrap();
    assert_eq!(paths(&check_toml(&doc)), vec!["$.a[0]", "$.a[1]"]);
    let one = Doc::from_raw(edges(vec![("a", RawNode::Leaf(Scalar::Null))])).unwrap();
    assert_eq!(paths(&check_toml(&one)), vec!["$.a"]);
}

#[test]
fn json_check_indexes_the_first_occurrence_of_a_repeated_nan() {
    let nan = || RawNode::Leaf(Scalar::Float(f64::NAN));
    let doc = Doc::from_raw(edges(vec![("a", nan()), ("a", nan())])).unwrap();
    assert_eq!(paths(&check_json(&doc)), vec!["$.a[0]", "$.a[1]"]);
}

#[test]
fn xml_check_indexes_the_first_occurrence_of_a_repeated_bad_label() {
    // A leaf string with an XML-illegal character, repeated.
    let bad = || leaf("a\u{1}b");
    let doc = Doc::from_raw(edges(vec![("a", bad()), ("a", bad())])).unwrap();
    assert_eq!(paths(&check_xml(&doc)), vec!["$.a[0]", "$.a[1]"]);
}

#[test]
fn xml_reader_report_indexes_the_first_occurrence_of_a_repeated_element() {
    let cases: &[(&str, &[&str])] = &[
        ("<r><a x=\"1\"/></r>", &["$.r.a"]),
        ("<r><a x=\"1\"/><a/></r>", &["$.r.a[0]"]),
        ("<r><a/><a x=\"1\"/></r>", &["$.r.a[1]"]),
        // The nested diagnostic follows its ancestor's index.
        ("<r><a><b x=\"1\"/></a><a/></r>", &["$.r.a[0].b"]),
        // Interleaved: `a` is counted per node, `b` occurs once.
        (
            "<r><a x=\"1\"/><b x=\"1\"/><a x=\"1\"/></r>",
            &["$.r.a[0]", "$.r.b", "$.r.a[1]"],
        ),
        // The root element is a single edge of `$`.
        ("<r x=\"1\"/>", &["$.r"]),
    ];
    for (text, expect) in cases {
        let mut rep = WriteReport::new();
        read_xml_report(text, Some(&mut rep)).unwrap();
        assert_eq!(&paths(&rep), expect, "{text}");
    }
}
