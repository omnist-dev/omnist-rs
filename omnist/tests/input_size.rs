//! D-23 (omnist-spec v0.30.0-beta): a finite maximum input size, in bytes,
//! checked before decoding or parsing, on every read of a Document from
//! text. An input of exactly the maximum is accepted, one byte over is
//! refused with `document.limit.input-size` at `$`; a leading BOM is counted
//! (the length is taken before it is stripped); the refusal comes ahead of
//! every other diagnostic.

use omnist::document::{Doc, RawNode};
use omnist::error::OmnistError;
use omnist::formats::json::{read_json, read_json_with};
use omnist::formats::toml::{read_toml, read_toml_with};
use omnist::formats::xml::{read_xml, read_xml_report, read_xml_with, read_xml_with_schema};
use omnist::formats::yaml::{YamlReadOptions, read_yaml, read_yaml_with};
use omnist::limits::{DEFAULT_MAX_INPUT_BYTES, Limits, MAX_INPUT_BYTES_CEILING, input_size_error};
use omnist::oml::{read_oml, read_oml_with};
use omnist::osd::parse_schema;
use omnist::registry::{Format, register_format};

const CODE: &str = "document.limit.input-size";

/// A reader of one format under explicit limits, reduced to the error.
type Reader = fn(&str, &Limits) -> Result<(), OmnistError>;

fn oml(t: &str, l: &Limits) -> Result<(), OmnistError> {
    read_oml_with(t, l).map(|_| ())
}
fn json(t: &str, l: &Limits) -> Result<(), OmnistError> {
    read_json_with(t, l).map(|_| ())
}
fn toml(t: &str, l: &Limits) -> Result<(), OmnistError> {
    read_toml_with(t, l).map(|_| ())
}
fn xml(t: &str, l: &Limits) -> Result<(), OmnistError> {
    read_xml_with(t, l).map(|_| ())
}
fn yaml(t: &str, l: &Limits) -> Result<(), OmnistError> {
    read_yaml_with(t, &YamlReadOptions::default().with_limits(*l)).map(|_| ())
}

/// (format, a valid document that survives trailing spaces, its reader).
fn formats() -> Vec<(&'static str, &'static str, Reader)> {
    vec![
        ("oml", "a: \"x\"", oml as Reader),
        ("json", "{\"a\": \"x\"}", json as Reader),
        ("toml", "a = \"x\"", toml as Reader),
        ("xml", "<a>x</a>", xml as Reader),
        ("yaml", "a: x", yaml as Reader),
    ]
}

/// `doc` padded with trailing spaces to exactly `bytes` bytes.
fn padded(doc: &str, bytes: usize) -> String {
    assert!(doc.len() <= bytes);
    format!("{doc}{}", " ".repeat(bytes - doc.len()))
}

fn assert_refused(r: Result<(), OmnistError>, ctx: &str) {
    match r {
        Err(OmnistError::Document(e)) => {
            assert_eq!(e.code.as_deref(), Some(CODE), "{ctx}");
            assert_eq!(e.path, "$", "{ctx}");
        }
        other => panic!("{ctx}: expected {CODE}, got {other:?}"),
    }
}

#[test]
fn at_the_maximum_is_accepted_and_one_over_is_refused_in_every_format() {
    for max in [20u64, 64, 1000] {
        let limits = Limits::default().with_max_input_bytes(max);
        for (name, doc, read) in formats() {
            let at = padded(doc, max as usize);
            read(&at, &limits).unwrap_or_else(|e| panic!("{name} at {max}: {e}"));
            let over = padded(doc, max as usize + 1);
            assert_refused(read(&over, &limits), &format!("{name} over {max}"));
        }
    }
}

#[test]
fn the_unit_is_bytes_not_characters() {
    // Seven two-byte characters: 14 bytes of text, 7 characters.
    let doc = "a: \"\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\"";
    assert_eq!(doc.chars().count(), 3 + 1 + 7 + 1);
    assert_eq!(doc.len(), 3 + 1 + 14 + 1);
    let at = Limits::default().with_max_input_bytes(doc.len() as u64);
    assert!(oml(doc, &at).is_ok());
    let below = Limits::default().with_max_input_bytes(doc.len() as u64 - 1);
    // A character count (12) would admit it; the byte count (19) must not.
    assert_refused(oml(doc, &below), "multi-byte");
}

#[test]
fn a_leading_bom_is_counted_before_it_is_stripped() {
    for (name, doc, read) in formats() {
        let with_bom = format!("\u{feff}{doc}");
        let n = with_bom.len() as u64; // the BOM is three bytes
        assert!(read(&with_bom, &Limits::default().with_max_input_bytes(n)).is_ok());
        // One byte under: only a count taken before the strip refuses it.
        assert_refused(
            read(&with_bom, &Limits::default().with_max_input_bytes(n - 1)),
            &format!("{name} bom"),
        );
        // The same document without the BOM fits in n - 3.
        assert!(read(doc, &Limits::default().with_max_input_bytes(n - 3)).is_ok());
    }
}

#[test]
fn the_size_refusal_precedes_every_other_diagnostic() {
    let limits = Limits::default().with_max_input_bytes(8);
    // Malformed, doubled BOM, nesting past the depth limit: all over 8
    // bytes, all refused as an oversized input first.
    for text in [
        "{{{{{{{{{{{{{{{{{{{{",
        "\u{feff}\u{feff}a: 1",
        "a: {b: {c: {d: 1}}}}",
        "\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}",
    ] {
        for (name, _, read) in formats() {
            assert_refused(read(text, &limits), &format!("{name} {text:?}"));
        }
    }
}

#[test]
fn the_plain_readers_and_report_and_schema_readers_enforce_the_default() {
    // One byte more than the default: refused before any parsing, so the
    // text may be anything.
    let big = " ".repeat(DEFAULT_MAX_INPUT_BYTES as usize + 1);
    let refused = |r: Result<(), OmnistError>, name: &str| assert_refused(r, name);
    refused(read_json(&big).map(|_| ()), "read_json");
    refused(read_toml(&big).map(|_| ()), "read_toml");
    refused(read_yaml(&big).map(|_| ()), "read_yaml");
    refused(read_xml(&big).map(|_| ()), "read_xml");
    refused(read_xml_report(&big, None).map(|_| ()), "read_xml_report");
    let schema = parse_schema("record R {\n    \"a\": string,\n}\nroot R\n").unwrap();
    refused(
        read_xml_with_schema(&big, &schema).map(|_| ()),
        "read_xml_with_schema",
    );
    refused(
        Doc::from_format("json", &big).map(|_| ()),
        "Doc::from_format json",
    );
    refused(
        Doc::from_format("oml", &big).map(|_| ()),
        "Doc::from_format oml",
    );
    // `read_oml` keeps its `ParseError` signature: the same code, at 1:1.
    let e = read_oml(&big).unwrap_err();
    assert_eq!((e.code.as_str(), e.position().as_str()), (CODE, "1:1"));
}

#[test]
fn a_registered_format_is_bounded_by_from_format() {
    register_format(Format::new(
        "sizebound-test",
        |_t| Ok(Doc::from_raw(RawNode::Edges(vec![])).unwrap()),
        |_d| Ok(String::new()),
    ));
    let big = " ".repeat(DEFAULT_MAX_INPUT_BYTES as usize + 1);
    assert_refused(
        Doc::from_format("sizebound-test", &big).map(|_| ()),
        "registered",
    );
    assert!(Doc::from_format("sizebound-test", "").is_ok());
}

#[test]
fn the_default_is_64_mib_and_zero_selects_it() {
    assert_eq!(DEFAULT_MAX_INPUT_BYTES, 64 * 1024 * 1024);
    assert_eq!(Limits::default().max_input_bytes, DEFAULT_MAX_INPUT_BYTES);
    assert_eq!(
        Limits::default().effective_max_input_bytes(),
        DEFAULT_MAX_INPUT_BYTES
    );
    let zero = Limits::default().with_max_input_bytes(0);
    assert_eq!(zero.effective_max_input_bytes(), DEFAULT_MAX_INPUT_BYTES);
    // 0 does not mean "refuse everything".
    assert!(oml("a: 1", &zero).is_ok());
}

#[test]
fn the_ceiling_is_validated_not_clamped() {
    let at = Limits::default().with_max_input_bytes(MAX_INPUT_BYTES_CEILING);
    assert!(at.validate().is_ok());
    let over = Limits::default().with_max_input_bytes(MAX_INPUT_BYTES_CEILING + 1);
    let e = over.validate().unwrap_err();
    assert!(e.message.contains("max_input_bytes"), "{}", e.message);
    for (name, doc, read) in formats() {
        match read(doc, &over) {
            Err(OmnistError::Document(e)) => {
                assert!(e.code.is_none(), "{name}: misuse carries no code");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn the_refusal_says_how_to_raise_the_limit() {
    let e = input_size_error(5, "pass --x to raise it");
    assert_eq!(e.code.as_deref(), Some(CODE));
    assert_eq!(e.path, "$");
    assert!(e.message.contains("5 bytes"), "{}", e.message);
    assert!(e.message.ends_with("pass --x to raise it"), "{}", e.message);
    let limits = Limits::default().with_max_input_bytes(2);
    match oml("a: 1", &limits) {
        Err(OmnistError::Document(e)) => {
            assert!(
                e.message.contains("Limits::max_input_bytes"),
                "{}",
                e.message
            );
        }
        other => panic!("{other:?}"),
    }
}
