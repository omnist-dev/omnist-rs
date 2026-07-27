//! Tests mirroring Python's `tests/test_canonical.py::TestRegistry` (five
//! cases: builtins registered, `get_format` round-trip, unknown-format
//! error, register a plugin, plugin-without-check errors on `check_format`),
//! plus coverage for the other three builtins' registry-wrapper closures
//! (yaml/toml/xml/oml -- `get_format_round_trips_json` above only exercises
//! `json`'s).

use super::*;
use crate::document::Doc;
use crate::error::OmnistError;

/// `get_format` doesn't require `Format: Debug`, so its `Err` is unwrapped
/// via a `match` here rather than `.unwrap_err()` (which would need
/// `Format: Debug` too, for the `Ok` arm's panic message) -- avoids adding a
/// `Debug` impl to [`Format`] purely to satisfy a test helper.
fn expect_unknown(name: &str) -> OmnistError {
    match get_format(name) {
        Err(e) => e,
        Ok(_) => panic!("expected {name:?} to be unregistered"),
    }
}

#[test]
fn builtins_are_all_registered() {
    let names = formats();
    for expected in ["json", "yaml", "toml", "xml", "oml"] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
}

#[test]
fn get_format_round_trips_json() {
    let fmt = get_format("json").unwrap();
    let doc = (fmt.read)(r#"{"a": 1}"#).unwrap();
    assert_eq!((fmt.write)(&doc).unwrap(), r#"{"a": 1}"#);
    assert!(fmt.check.as_ref().unwrap()(&doc).is_ok());
}

#[test]
fn get_format_round_trips_yaml() {
    let fmt = get_format("yaml").unwrap();
    let doc = (fmt.read)("a: 1\n").unwrap();
    assert_eq!((fmt.write)(&doc).unwrap(), "a: 1");
    assert!(fmt.check.as_ref().unwrap()(&doc).is_ok());
}

#[test]
fn get_format_round_trips_toml() {
    let fmt = get_format("toml").unwrap();
    let doc = (fmt.read)("a = 1\n").unwrap();
    assert_eq!((fmt.write)(&doc).unwrap(), "a = 1\n");
    assert!(fmt.check.as_ref().unwrap()(&doc).is_ok());
}

#[test]
fn get_format_round_trips_xml() {
    let fmt = get_format("xml").unwrap();
    let doc = (fmt.read)("<a>1</a>").unwrap();
    assert_eq!((fmt.write)(&doc).unwrap(), "<a>1</a>");
    assert!(fmt.check.as_ref().unwrap()(&doc).is_ok());
}

#[test]
fn get_format_round_trips_oml() {
    let fmt = get_format("oml").unwrap();
    let doc = (fmt.read)("a: 1").unwrap();
    assert_eq!((fmt.write)(&doc).unwrap(), "a: 1");
    // OML is lossless for every Doc, so `check_oml` always reports clean --
    // this is the one builtin `check` whose interesting behavior *is*
    // always returning an empty report (see `oml::check_oml`'s doc
    // comment).
    let report = fmt.check.as_ref().unwrap()(&doc);
    assert!(report.is_ok());
}

#[test]
fn doc_from_to_check_format_dispatch_through_registry() {
    // Doc::from_format/to_format/check_format (issue #31) go through
    // get_format rather than a fixed match -- confirmed here for a builtin,
    // separately from the plugin-based tests below.
    let d = Doc::from_format("json", r#"{"a": 1}"#).unwrap();
    assert_eq!(d.to_format("json").unwrap(), r#"{"a": 1}"#);
    assert!(d.check_format("json").unwrap().is_ok());
}

#[test]
fn unknown_format_raises_omnist_error() {
    let err = expect_unknown("nope");
    assert!(matches!(err, OmnistError::Format(_)));
    assert!(err.to_string().contains("unknown format"));
    assert!(err.to_string().contains("nope"));
}

#[test]
fn unknown_format_error_lists_registered_names() {
    let err = expect_unknown("nope");
    let msg = err.to_string();
    assert!(msg.contains("json"));
    assert!(msg.contains("oml"));
}

#[test]
fn doc_from_format_propagates_unknown_format_error() {
    let err = Doc::from_format("nope", "x").unwrap_err();
    assert!(matches!(err, OmnistError::Format(_)));
}

#[test]
fn doc_to_format_propagates_unknown_format_error() {
    let d = Doc::from_format("json", "1").unwrap();
    let err = d.to_format("nope").unwrap_err();
    assert!(matches!(err, OmnistError::Format(_)));
}

fn lines_read(text: &str) -> Result<Doc, OmnistError> {
    let edges: Vec<(String, crate::document::RawNode)> = text
        .split_whitespace()
        .map(|x| {
            (
                "n".to_string(),
                crate::document::RawNode::Leaf(crate::document::Scalar::Int(x.parse().unwrap())),
            )
        })
        .collect();
    Doc::from_raw(crate::document::RawNode::Edges(edges)).map_err(Into::into)
}

fn lines_write(doc: &Doc) -> Result<String, OmnistError> {
    let raw = doc.to_raw();
    let crate::document::RawNode::Edges(edges) = raw else {
        return Ok(String::new());
    };
    Ok(edges
        .iter()
        .map(|(_, v)| match v {
            crate::document::RawNode::Leaf(crate::document::Scalar::Int(i)) => i.to_string(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" "))
}

#[test]
fn register_a_plugin_and_use_it_via_doc() {
    // Registry-mutating tests each use a name unique to themselves (rather
    // than a shared literal like "lines") so they can't collide with each
    // other regardless of execution order under parallel test threads --
    // the global registry (see `registry.rs`'s module doc: "this crate's
    // only piece of global mutable state") is process-wide, so two tests
    // registering the same name concurrently can otherwise observe each
    // other's writes (issue #58).
    let name = "lines-register_a_plugin_and_use_it_via_doc";
    register_format(Format::new(name, lines_read, lines_write));

    assert!(formats().contains(&name.to_string()));
    let d = Doc::from_format(name, "1 2 3").unwrap();
    assert_eq!(d.to_format(name).unwrap(), "1 2 3");
}

#[test]
fn plugin_without_check_errors_cleanly_on_check_format() {
    let name = "nocheck-plugin_without_check_errors_cleanly_on_check_format";
    register_format(Format::new(name, lines_read, lines_write));

    let d = Doc::from_format(name, "1 2 3").unwrap();
    let err = d.check_format(name).unwrap_err();
    assert!(matches!(err, OmnistError::Document(_)));
    assert!(err.to_string().contains("has no check"));
}

#[test]
fn plugin_replaces_earlier_registration_of_the_same_name() {
    // register_format is "register (or replace)" -- registering the same
    // name again under a writer that always emits a fixed string confirms
    // the second registration wins, matching Python's plain-dict-assignment
    // semantics (`_REGISTRY[fmt.name] = fmt`). The name itself is unique to
    // this test (see comment on `register_a_plugin_and_use_it_via_doc`)
    // since what's under test is same-name replacement *within* one test,
    // not a fixed shared name across tests.
    let name = "lines-plugin_replaces_earlier_registration_of_the_same_name";
    register_format(Format::new(name, lines_read, lines_write));
    register_format(Format::new(name, lines_read, |_doc| {
        Ok("replaced".to_string())
    }));
    let d = Doc::from_format(name, "1 2 3").unwrap();
    assert_eq!(d.to_format(name).unwrap(), "replaced");
}

#[test]
fn plugin_with_check_is_usable_via_check_format() {
    let name = "withcheck-plugin_with_check_is_usable_via_check_format";
    register_format(
        Format::new(name, lines_read, lines_write)
            .with_check(|_doc| crate::report::WriteReport::new()),
    );
    let d = Doc::from_format(name, "1 2 3").unwrap();
    assert!(d.check_format(name).unwrap().is_ok());
}
