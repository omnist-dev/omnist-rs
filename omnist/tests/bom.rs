//! D-15 / D-21 (omnist-spec `docs/02-document-model.md` section 2.5) across
//! all six read surfaces and every writer, through the public API.
//!
//! The mark is always the escape `\u{FEFF}` here, never a raw character; the
//! unit test `bom::tests::no_tracked_source_file_contains_a_raw_bom` fails if
//! any tracked file, this one included, ever contains one.

use omnist::document::{Doc, RawNode, Scalar};
use omnist::error::OmnistError;
use omnist::formats::json::{read_json, write_json};
use omnist::formats::toml::{read_toml, write_toml};
use omnist::formats::xml::{read_xml, write_xml};
use omnist::formats::yaml::{read_yaml, write_yaml};
use omnist::oml::{read_oml, write_oml, write_oml_compact};
use omnist::osd::{parse_schema, to_osd};

const BOM: char = '\u{FEFF}';

/// What every reader reports for a doubled mark: `(position, code)`.
type Rejection = (String, String);

fn oml(text: &str) -> Result<String, Rejection> {
    read_oml(text)
        .map(|r| format!("{r:?}"))
        .map_err(|e| (e.position(), e.code))
}

fn osd(text: &str) -> Result<String, Rejection> {
    parse_schema(text)
        .map(|s| to_osd(&s, None).unwrap())
        .map_err(|e| (e.path, e.code))
}

fn codec(r: Result<Doc, OmnistError>) -> Result<String, Rejection> {
    match r {
        Ok(d) => Ok(format!("{:?}", d.to_raw())),
        Err(OmnistError::Parse(e)) => Err((e.position(), e.code)),
        Err(other) => panic!("expected a parse error, got {other:?}"),
    }
}

/// `(surface name, reader, well-formed text, expected code for a doubled mark)`
type Case = (
    &'static str,
    fn(&str) -> Result<String, Rejection>,
    &'static str,
    &'static str,
);

fn cases() -> Vec<Case> {
    vec![
        ("oml", oml, "a: 1\n", "parse.unexpected-token"),
        (
            "osd",
            osd,
            "record R {\n    \"a\": string,\n}\nroot R\n",
            "parse.unexpected-token",
        ),
        (
            "json",
            |t| codec(read_json(t)),
            "{\"a\":1}",
            "parse.codec-syntax",
        ),
        (
            "yaml",
            |t| codec(read_yaml(t)),
            "a: 1\n",
            "parse.codec-syntax",
        ),
        (
            "toml",
            |t| codec(read_toml(t)),
            "a = 1\n",
            "parse.codec-syntax",
        ),
        (
            "xml",
            |t| codec(read_xml(t)),
            "<root><a>1</a></root>",
            "parse.codec-syntax",
        ),
    ]
}

#[test]
fn one_leading_mark_is_stripped_on_every_read_surface() {
    for (name, read, text, _) in cases() {
        let plain = read(text);
        assert!(plain.is_ok(), "{name}: baseline must parse: {plain:?}");
        assert_eq!(
            read(&format!("{BOM}{text}")),
            plain,
            "{name}: a BOM-prefixed input must read exactly like the same input without it"
        );
    }
}

#[test]
fn a_second_leading_mark_is_rejected_at_1_1_on_every_read_surface() {
    for (name, read, text, code) in cases() {
        for marks in [2, 3] {
            let input = format!("{}{text}", BOM.to_string().repeat(marks));
            assert_eq!(
                read(&input),
                Err(("1:1".to_string(), code.to_string())),
                "{name}: {marks} leading marks"
            );
        }
    }
}

/// `[(label, "value")]`, the one-edge Document every case below expects.
fn one_edge(label: &str, value: Scalar) -> RawNode {
    RawNode::Edges(vec![(label.to_string(), RawNode::Leaf(value))])
}

fn s(text: &str) -> Scalar {
    Scalar::Str(text.to_string())
}

#[test]
fn a_mark_that_is_not_at_offset_zero_is_never_stripped_or_rejected() {
    let label = format!("a{BOM}b");
    let value = format!("x{BOM}y");
    // OML: a mark inside a quoted label and a quoted value survives.
    let raw = read_oml(&format!("\"{label}\": \"{value}\"\n")).unwrap();
    assert_eq!(raw, one_edge(&label, s(&value)));
    // JSON: the suite's `interior-bom-is-ordinary-content` shape.
    let doc = read_json(&format!("{{\"{label}\": \"{value}\"}}")).unwrap();
    assert_eq!(doc.to_raw(), one_edge(&label, s(&value)));
    // A single leading mark is stripped and an interior one survives.
    let doc = read_json(&format!("{BOM}{{\"{label}\": 1}}")).unwrap();
    assert_eq!(doc.to_raw(), one_edge(&label, Scalar::Int(1.into())));
    // YAML: a quoted key that begins with the mark is readable (the
    // documented workaround for the one shape D-21 costs YAML)...
    let led = format!("{BOM}abc");
    let doc = read_yaml(&format!("\"{led}\": 1\n")).unwrap();
    assert_eq!(doc.to_raw(), one_edge(&led, Scalar::Int(1.into())));
    // ...and so is an unquoted key with the mark anywhere but the start.
    let mid = format!("a{BOM}bc");
    let doc = read_yaml(&format!("{mid}: 1\n")).unwrap();
    assert_eq!(doc.to_raw(), one_edge(&mid, Scalar::Int(1.into())));
    // TOML and XML text content.
    let doc = read_toml(&format!("\"{label}\" = \"{value}\"\n")).unwrap();
    assert_eq!(doc.to_raw(), one_edge(&label, s(&value)));
    let doc = read_xml(&format!("<r>{value}</r>")).unwrap();
    assert_eq!(doc.to_raw(), one_edge("r", s(&value)));
    // OSD: inside a quoted field label.
    let schema = parse_schema(&format!(
        "record R {{\n    \"{label}\": string,\n}}\nroot R\n"
    ))
    .unwrap();
    assert!(to_osd(&schema, None).unwrap().contains(&label));
}

#[test]
fn the_mark_is_not_treated_as_whitespace_or_a_stray_character_after_the_strip() {
    // Whitespace, a comment, then the mark again is NOT a leading mark:
    // it is at offset > 0, hence ordinary content -- and for OML/OSD a
    // stray character (their grammars give it no position but the first).
    assert!(oml(&format!(" {BOM}a: 1")).is_err());
    assert!(osd(&format!(" {BOM}root R")).is_err());
}

#[test]
fn no_writer_ever_emits_a_leading_mark() {
    let doc = read_oml("r: { a: \"x\"; b: 2 }\n")
        .map(|raw| Doc::from_raw(raw).unwrap())
        .unwrap();
    let mut outputs: Vec<(&str, String)> = vec![
        ("json", write_json(&doc, None, false, None).unwrap()),
        (
            "json-pretty",
            write_json(&doc, Some(2), false, None).unwrap(),
        ),
        ("yaml", write_yaml(&doc, false, None).unwrap()),
        ("toml", write_toml(&doc, false, None).unwrap()),
        ("xml", write_xml(&doc, false, None).unwrap()),
        ("oml", write_oml(&doc.to_raw(), 2).unwrap()),
        ("oml-compact", write_oml_compact(&doc.to_raw()).unwrap()),
    ];
    let schema = parse_schema("record R {\n    \"a\": string,\n}\nroot R\n").unwrap();
    outputs.push(("osd", to_osd(&schema, None).unwrap()));
    outputs.push(("osd-compact", to_osd(&schema, Some(2)).unwrap()));
    for (name, text) in outputs {
        assert!(!text.starts_with(BOM), "{name} wrote a leading BOM");
        assert!(!text.contains(BOM), "{name} wrote a BOM");
    }
}

#[test]
fn a_document_holding_the_mark_writes_it_only_as_content_never_as_a_leading_mark() {
    let raw = read_oml(&format!("\"{BOM}k\": \"{BOM}v\"\n")).unwrap();
    let doc = Doc::from_raw(raw).unwrap();
    for (name, text) in [
        ("json", write_json(&doc, None, false, None).unwrap()),
        ("yaml", write_yaml(&doc, false, None).unwrap()),
        ("toml", write_toml(&doc, false, None).unwrap()),
        ("oml", write_oml(&doc.to_raw(), 2).unwrap()),
        ("oml-compact", write_oml_compact(&doc.to_raw()).unwrap()),
    ] {
        assert!(
            !text.starts_with(BOM),
            "{name}: content BOM leaked to the front"
        );
        assert!(text.contains(BOM), "{name}: the content mark must survive");
    }
    // Round-trips: the mark stays content.
    let yaml = write_yaml(&doc, false, None).unwrap();
    assert_eq!(read_yaml(&yaml).unwrap().to_raw(), doc.to_raw());
}
