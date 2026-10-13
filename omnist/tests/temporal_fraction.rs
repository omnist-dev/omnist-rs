//! Readers and writers never alter a temporal value (E-37, bug wave after
//! spec v0.35.0-beta): the Document model holds a fractional second of at
//! most six digits (the OML grammar's `1*6DIGIT`), so a reader fails rather
//! than truncate a non-zero seventh digit, and a writer never emits a
//! literal its own reader rejects.

use omnist::document::{Doc, RawNode, Scalar};
use std::time::{Duration, Instant};

fn leaf(s: Scalar) -> Doc {
    Doc::from_raw(RawNode::Edges(vec![("t".to_string(), RawNode::Leaf(s))])).unwrap()
}

fn read_t(fmt: &str, text: &str) -> Result<Scalar, String> {
    let doc = Doc::from_format(fmt, text).map_err(|e| e.to_string())?;
    Ok(doc.root().get_one("t").unwrap().value().unwrap().clone())
}

#[test]
fn toml_and_yaml_readers_keep_offsets_and_up_to_six_digits() {
    for (fmt, text, want) in [
        (
            "toml",
            "t = 2024-01-15T12:00:00.123456+01:00\n",
            "2024-01-15T12:00:00.123456+01:00",
        ),
        (
            "toml",
            "t = 2024-01-15T12:00:00.5-07:30\n",
            "2024-01-15T12:00:00.500000-07:30",
        ),
        (
            "yaml",
            "t: 2024-01-15T12:00:00.123456+01:00\n",
            "2024-01-15T12:00:00.123456+01:00",
        ),
        // Zeros beyond the sixth digit are insignificant.
        (
            "toml",
            "t = 2024-01-15T12:00:00.1234560+01:00\n",
            "2024-01-15T12:00:00.123456+01:00",
        ),
        (
            "yaml",
            "t: 2024-01-15T12:00:00.123456000+01:00\n",
            "2024-01-15T12:00:00.123456+01:00",
        ),
    ] {
        assert_eq!(
            read_t(fmt, text),
            Ok(Scalar::Datetime(want.to_string())),
            "{fmt} {text:?}"
        );
    }
}

#[test]
fn toml_and_yaml_readers_fail_on_a_nonzero_digit_past_the_sixth() {
    for (fmt, text) in [
        ("toml", "t = 12:00:00.1234567\n"),
        ("toml", "t = 12:00:00.000000001\n"),
        ("toml", "t = 12:00:00.123456789\n"),
        ("toml", "t = 2024-01-15T12:00:00.1234567+01:00\n"),
        ("toml", "a = 1\n[x]\nt = 2024-01-15T12:00:00.1234567Z\n"),
        ("yaml", "t: 2024-01-15T12:00:00.1234567+01:00\n"),
        ("yaml", "t: 2024-01-15T12:00:00.000000001\n"),
    ] {
        let err = read_t(fmt, text).unwrap_err();
        assert!(err.contains("six digits"), "{fmt} {text:?}: {err}");
    }
}

#[test]
fn toml_fraction_failure_is_positioned_at_the_literal() {
    let err = read_t("toml", "a = 1\nt = 12:00:00.1234567\n").unwrap_err();
    assert!(err.contains("line 2"), "{err}");
}

#[test]
fn oml_writer_never_emits_a_fraction_its_reader_rejects() {
    // Exact within six digits: the extra zeros are dropped.
    for (s, want) in [
        ("12:00:00.1234560", "t: 12:00:00.123456"),
        ("12:00:00.123400000", "t: 12:00:00.123400"),
    ] {
        let text = leaf(Scalar::Time(s.to_string())).to_format("oml").unwrap();
        assert_eq!(text, want);
        assert!(Doc::from_format("oml", &text).is_ok());
    }
    // Not exact: stringified, reported, and it reads back.
    for scalar in [
        Scalar::Time("12:00:00.1234567".to_string()),
        Scalar::Time("12:00:00.123456789".to_string()),
        Scalar::Datetime("2024-01-15T12:00:00.1234567+01:00".to_string()),
    ] {
        let doc = leaf(scalar);
        let text = doc.to_format("oml").unwrap();
        assert!(text.contains('"'), "{text}");
        assert!(Doc::from_format("oml", &text).is_ok(), "{text}");
        let rep = doc.check_format("oml").unwrap();
        assert_eq!(rep.adjustments().len(), 1);
        assert_eq!(rep.adjustments()[0].code, "format.temporal-stringified");
        assert_eq!(rep.adjustments()[0].path, "$.t");
    }
    // A six-digit value is untouched and reports nothing.
    let doc = leaf(Scalar::Time("12:00:00.123456".to_string()));
    assert!(doc.check_format("oml").unwrap().is_empty());
}

#[test]
fn toml_and_yaml_writers_stringify_what_their_readers_would_reject() {
    for fmt in ["toml", "yaml"] {
        for scalar in [
            Scalar::Time("12:00:00.1234567".to_string()),
            Scalar::Datetime("2024-01-15T12:00:00.1234567+01:00".to_string()),
        ] {
            let doc = leaf(scalar);
            let text = doc.to_format(fmt).unwrap();
            assert!(text.contains('"'), "{fmt}: {text}");
            assert!(Doc::from_format(fmt, &text).is_ok(), "{fmt}: {text}");
            let rep = doc.check_format(fmt).unwrap();
            assert_eq!(rep.adjustments()[0].code, "format.temporal-stringified");
        }
        // Exact within six digits: still the native literal (or, for a YAML
        // time, its quoted string), trimmed.
        let doc = leaf(Scalar::Datetime(
            "2024-01-15T12:00:00.1234560+01:00".to_string(),
        ));
        let text = doc.to_format(fmt).unwrap();
        assert!(text.contains("12:00:00.123456+01:00"), "{text}");
        assert!(!text.contains('"'), "{text}");
    }
    let doc = leaf(Scalar::Time("12:00:00.1234560".to_string()));
    assert_eq!(doc.to_format("toml").unwrap().trim(), "t = 12:00:00.123456");
}

fn within(what: &str, limit: Duration, f: impl FnOnce()) {
    let t = Instant::now();
    f();
    assert!(t.elapsed() < limit, "{what} took {:?}", t.elapsed());
}

/// Hostile XML: an 8 MB attribute and a 100,000-deep document give their
/// result quickly (a generous bound; the measured times are milliseconds).
#[test]
fn hostile_xml_scales_linearly_and_never_overflows_the_stack() {
    for mb in [1usize, 2, 4, 8] {
        let xml = format!("<r a=\"{}\"><c/></r>", "a".repeat(mb << 20));
        within("xml attribute", Duration::from_secs(10), || {
            assert!(Doc::from_format("xml", &xml).is_ok());
        });
    }
    for depth in [1_000usize, 10_000, 100_000] {
        let xml = format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
        within("xml depth", Duration::from_secs(10), || {
            let err = Doc::from_format("xml", &xml).unwrap_err().to_string();
            assert!(err.contains("maximum depth"), "{err}");
        });
    }
}
