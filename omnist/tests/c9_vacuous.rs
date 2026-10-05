//! C-9 (omnist-spec v0.32.0-beta): every writer MUST fail with
//! `write.unsupported-value` on a string value or an edge label with no UTF-8
//! encoding (a UTF-16 lone surrogate, a surrogate-escape artefact, or, in Go,
//! a byte sequence that is not well-formed UTF-8).
//!
//! In this port the rule is vacuous, and these tests pin why, so a future
//! change that opens a route cannot do it silently:
//!
//! * a Rust `String` (and `&str`, `char`) cannot hold an unencodable string,
//!   so every Document, `RawNode` and `Value` this crate builds is encodable
//!   by type; there is no byte, `OsString` or `Cow<[u8]>` entry point in the
//!   `omnist` library;
//! * the one byte entry point, the CLI's, decodes strictly (D-14) and never
//!   repairs (`String::from_utf8`, no `U+FFFD`);
//! * no reader turns an invalid escape into a replacement character: each
//!   refuses a lone surrogate escape (a reader that accepted one would have to
//!   substitute, which is the Go-style bug);
//! * no production code converts bytes to text lossily, except two audited
//!   sites in the XML reader that work on slices of an already valid `&str`.

use std::path::{Path, PathBuf};

use omnist::formats::json::read_json;
use omnist::formats::toml::read_toml;
use omnist::formats::xml::read_xml;
use omnist::formats::yaml::read_yaml;
use omnist::oml::read_oml;

/// Every reader refuses a lone surrogate, as a string or as a label, in each
/// spelling its grammar has for a code point: none replaces it.
#[test]
fn no_reader_turns_a_lone_surrogate_into_a_replacement_character() {
    type Read = fn(&str) -> bool;
    let readers: [(&str, Read, &[&str]); 5] = [
        (
            "json",
            |t| read_json(t).is_ok(),
            &[
                r#"{"a":"\ud800"}"#,
                r#"{"a":"\udc00"}"#,
                r#"{"a":"\ud800x"}"#,
                r#"{"\ud800":1}"#,
                r#"{"a":"\udbffA"}"#,
            ],
        ),
        (
            "yaml",
            |t| read_yaml(t).is_ok(),
            &[r#"a: "\ud800""#, r#"a: "\udc00""#, r#""\ud800": 1"#],
        ),
        (
            "toml",
            |t| read_toml(t).is_ok(),
            &[r#"a = "\ud800""#, r#"a = "\uDC00""#, r#""\ud800" = 1"#],
        ),
        (
            "xml",
            |t| read_xml(t).is_ok(),
            &["<a>&#xD800;</a>", "<a>&#55296;</a>", "<a>&#xDFFF;</a>"],
        ),
        (
            "oml",
            |t| read_oml(t).is_ok(),
            &[r#"a: "\ud800""#, r#"a: "\udc00""#, r#""\ud800": 1"#],
        ),
    ];
    for (name, read, inputs) in readers {
        for text in inputs {
            assert!(!read(text), "{name} accepted {text}");
        }
    }
    // The paired form is a real character, not a replacement.
    let doc = read_json(r#"{"a":"😀"}"#).unwrap();
    let json = doc.to_format("json").unwrap();
    assert!(
        json.contains('\u{1F600}') && !json.contains('\u{fffd}'),
        "{json}"
    );
}

/// `text` without its `#[cfg(test)]` items: an inline `mod tests { .. }` is
/// dropped by matching its braces, an out-of-line `mod tests;` has no text.
fn without_test_modules(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "#[cfg(test)]".len()..];
        let brace = after.find('{');
        let semi = after.find(';');
        match (brace, semi) {
            (Some(b), s) if s.is_none_or(|s| b < s) => {
                let mut depth = 0usize;
                let mut end = after.len();
                for (i, c) in after[b..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = b + i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                rest = &after[end..];
            }
            (_, Some(s)) => rest = &after[s + 1..],
            _ => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// The production text of every non-test `.rs` file under `dir`, with its
/// path.
fn production_sources(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            production_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name == "tests.rs" || name.ends_with("_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            out.push((path, without_test_modules(&text)));
        }
    }
}

/// No production code converts bytes to text lossily or without a check.
/// The two `from_utf8_lossy` calls in `formats/xml.rs` are the audited
/// exceptions: the XML reader is `Reader::from_str` over an already valid
/// `&str`, and a CDATA body or an attribute value is a slice cut at ASCII
/// delimiters (`<![CDATA[`, `]]>`, a quote), so it is valid UTF-8 and the
/// replacement never fires.
#[test]
fn production_code_has_no_unaudited_lossy_conversion() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    production_sources(&manifest.join("src"), &mut sources);
    production_sources(
        &manifest.join("..").join("omnist-cli").join("src"),
        &mut sources,
    );
    assert!(sources.len() > 20, "the scan found the sources");
    let needles = [
        "from_utf8_lossy",
        "to_string_lossy",
        "from_utf8_unchecked",
        "from_utf16_lossy",
        "from_u32_unchecked",
    ];
    let mut found: Vec<(String, &str, usize)> = Vec::new();
    for (path, text) in &sources {
        for needle in needles {
            let n = text.matches(needle).count();
            if n > 0 {
                let file = path.file_name().unwrap().to_string_lossy().to_string();
                found.push((file, needle, n));
            }
        }
    }
    assert_eq!(
        found,
        vec![("xml.rs".to_string(), "from_utf8_lossy", 2)],
        "a new lossy byte-to-text conversion needs a C-9 audit (docs/limitations.md)"
    );
}
