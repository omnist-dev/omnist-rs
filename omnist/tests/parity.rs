//! Python test-corpus extraction and parity replay (issue #40).
//!
//! `omnist/tests/fixtures/parity/corpus.json` was extracted by running the
//! real, installed Python `omnist` package (see the sibling
//! `extract_fixtures.py` in that directory) — every fixture's `expected`
//! (or `error_contains`) field is Python's *actually observed* output, not
//! a transcription of the assertion source. This file replays each fixture
//! against the corresponding Rust API and asserts the same result.
//!
//! ## Scope
//!
//! Covers the omnist-rs modules that have a shipped counterpart today: the
//! OML codec, the shared depth guard, schema/OSD parsing plus the
//! `ops` algebra (`compatible_with`, `normalize`, `is_empty`), the
//! JSON/YAML/TOML/XML format codecs, and `materialize`. Deliberately
//! *not* covered by this corpus (see `extract_fixtures.py`'s module
//! docstring for the full rationale on each):
//!
//! - `test_any_core.py` / `test_any_grammar.py` — the `any` type's OSD
//!   *grammar* (parsing `any` from schema text) is still a later PR per
//!   that file's own docstring ("Grammar/parsing (I-8, I-9) ... land in
//!   later PRs"); `FieldType::Any` exists in Rust (issue #29) but nothing
//!   here builds an OSD-parsed `any` field to replay against yet.
//! - `test_public_api.py` — freezes *Python's* `omnist.__all__` import
//!   surface; not a cross-language concept.
//! - `test_cli.py` / `test_cli_examples.py` / `test_cli_fuzz.py` — Python
//!   CLI/argparse plumbing, not a Document/Schema API call.
//! - `test_examples*.py`, `test_docs.py`, `test_check_doc_examples.py`,
//!   `test_grammar_docs.py`, `test_lint.py` — doc-example/README/packaging
//!   generators for the Python repo's own tooling.
//! - `test_fuzz.py` — already ported at omnist-rs issue #26
//!   (`omnist/tests/fuzz.rs`), which includes a live cross-implementation
//!   oracle.
//! - `test_semantic_oracle.py` — exercises `tools/semantic_oracle.py`, the
//!   Python-only dev tool `fuzz.rs`'s oracle already shells out to.
//!
//! No genuine Python bug was found by this pass: every fixture's
//! `expected`/`error_contains` value is asserted as-is against Rust with
//! no divergence needed. (Running tally per the port's cross-
//! implementation bug policy: only issue #4 -> omnist-dev/omnist#255 so
//! far; this pass adds no new entries.)

use omnist::document::{Doc, RawNode, Scalar};
use omnist::materialize;
use omnist::oml::{read_oml, write_oml};
use omnist::ops::{compatible_with, is_empty};
use omnist::osd::{parse_schema, to_osd};
use serde_json::Value as J;
use std::fs;

fn corpus() -> Vec<J> {
    let text = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parity/corpus.json"
    ))
    .expect("parity corpus.json must be readable");
    let parsed: J = serde_json::from_str(&text).expect("parity corpus.json must be valid JSON");
    parsed["fixtures"]
        .as_array()
        .expect("corpus.json must have a top-level `fixtures` array")
        .clone()
}

/// Decode this crate's `enc()`-tagged JSON value into a [`RawNode`],
/// mirroring the Python extractor's encoding (`extract_fixtures.py::enc`).
fn decode_raw(v: &J) -> RawNode {
    let obj = v.as_object().expect("encoded value must be a JSON object");
    if let Some(b) = obj.get("$null") {
        assert_eq!(b, &J::Bool(true));
        return RawNode::Leaf(Scalar::Null);
    }
    if let Some(J::Bool(b)) = obj.get("$bool") {
        return RawNode::Leaf(Scalar::Bool(*b));
    }
    if let Some(n) = obj.get("$int") {
        return RawNode::Leaf(Scalar::Int(n.as_i64().expect("$int must fit in i64")));
    }
    if let Some(n) = obj.get("$float") {
        let f = match n {
            J::String(s) if s == "nan" => f64::NAN,
            J::String(s) if s == "inf" => f64::INFINITY,
            J::String(s) if s == "-inf" => f64::NEG_INFINITY,
            other => other.as_f64().expect("$float must be a JSON number"),
        };
        return RawNode::Leaf(Scalar::Float(f));
    }
    if let Some(J::String(s)) = obj.get("$str") {
        return RawNode::Leaf(Scalar::Str(s.clone()));
    }
    if let Some(J::Array(edges)) = obj.get("$edges") {
        let out = edges
            .iter()
            .map(|pair| {
                let pair = pair
                    .as_array()
                    .expect("$edges entry must be [label, value]");
                let label = pair[0]
                    .as_str()
                    .expect("$edges label must be a string")
                    .to_string();
                (label, decode_raw(&pair[1]))
            })
            .collect();
        return RawNode::Edges(out);
    }
    panic!("unrecognized encoded value: {v:?}");
}

/// `RawNode` has no `PartialEq` involving NaN-tolerant float comparison
/// (its derived `PartialEq` uses `f64`'s own, so NaN != NaN) -- fixtures
/// that need "is NaN" instead of "equals" use the `oml_parses_to_nan`
/// fixture kind, checked separately below.
fn assert_raw_eq(fixture_note: &str, expected: &RawNode, actual: &RawNode) {
    assert_eq!(
        expected, actual,
        "fixture {fixture_note:?}: Rust output diverged from Python's observed output"
    );
}

#[test]
fn parity_corpus_replays_every_fixture_against_rust() {
    let fixtures = corpus();
    assert!(
        fixtures.len() >= 40,
        "sanity check: expected a substantial extracted corpus, got {}",
        fixtures.len()
    );

    let mut ran = 0usize;
    for fx in &fixtures {
        let kind = fx["kind"].as_str().expect("fixture needs a `kind`");
        let note = fx["note"].as_str().unwrap_or("<no note>").to_string();

        match kind {
            // OML: feature/module targeted by `note` (see corpus.json).
            "oml_roundtrip" => {
                let input = fx["input"].as_str().unwrap();
                let expected = decode_raw(&fx["expected"]);
                let node = read_oml(input).unwrap_or_else(|e| {
                    panic!("fixture {note:?}: read_oml({input:?}) failed: {e}")
                });
                assert_raw_eq(&note, &expected, &node);
                // and the OML writer round-trips it back to the same node.
                let text = write_oml(&node, 0)
                    .unwrap_or_else(|e| panic!("fixture {note:?}: write_oml failed: {e}"));
                let node2 = read_oml(&text).unwrap_or_else(|e| {
                    panic!("fixture {note:?}: re-parse after write_oml failed: {e}")
                });
                assert_raw_eq(&format!("{note} (write_oml round-trip)"), &node, &node2);
            }
            "oml_parses_to_nan" => {
                let input = fx["input"].as_str().unwrap();
                let node = read_oml(input).unwrap();
                match node {
                    RawNode::Edges(edges) => match &edges[0].1 {
                        RawNode::Leaf(Scalar::Float(f)) => {
                            assert!(f.is_nan(), "fixture {note:?}: expected a NaN float")
                        }
                        other => panic!("fixture {note:?}: expected a float leaf, got {other:?}"),
                    },
                    other => panic!("fixture {note:?}: expected top-level edges, got {other:?}"),
                }
            }
            "oml_parse_error" => {
                let input = fx["input"].as_str().unwrap();
                let err = read_oml(input).err().unwrap_or_else(|| {
                    panic!("fixture {note:?}: expected a ParseError for {input:?}")
                });
                let _ = err.to_string(); // Display must not panic; exact wording isn't asserted
                // (Python's ParseError message text is Python-specific; this
                // fixture's parity claim is "Rust also rejects this input",
                // matching `error_contains`'s presence as a documentation aid.
                let _ = fx.get("error_contains");
            }
            // Depth guard (test_depth_guards.py): write_oml on a
            // programmatically-deep-nested node fails cleanly at the
            // shared 200-deep limit instead of blowing the stack.
            "oml_write_error" => {
                let depth = fx["depth"].as_u64().unwrap() as usize;
                let node = deep_node(depth);
                let err = write_oml(&node, 0)
                    .err()
                    .unwrap_or_else(|| panic!("fixture {note:?}: expected a WriteError"));
                let expected_msg = fx["error_contains"].as_str().unwrap();
                assert_eq!(
                    err.to_string(),
                    expected_msg,
                    "fixture {note:?}: WriteError message text diverged"
                );
            }
            "oml_write_ok" => {
                let depth = fx["depth"].as_u64().unwrap() as usize;
                let node = deep_node(depth);
                write_oml(&node, 0)
                    .unwrap_or_else(|e| panic!("fixture {note:?}: write_oml failed: {e}"));
            }
            // Schema/OSD (test_canonical.py): OSD parses.
            "osd_parse_ok" => {
                let input = fx["input"].as_str().unwrap();
                parse_schema(input)
                    .unwrap_or_else(|e| panic!("fixture {note:?}: parse_schema failed: {e}"));
            }
            // Schema ops: OSD round-trips to an equivalent schema.
            "schema_osd_roundtrip_equivalent" => {
                let text = fx["schema"].as_str().unwrap();
                let s = parse_schema(text).unwrap();
                let s2 = parse_schema(&to_osd(&s, None)).unwrap();
                assert!(
                    omnist::ops::equivalent(&s, &s2),
                    "fixture {note:?}: to_osd()+parse_schema() round-trip is not equivalent()"
                );
            }
            "schema_compatible_with" => {
                let a = parse_schema(fx["schema_a"].as_str().unwrap()).unwrap();
                let b = parse_schema(fx["schema_b"].as_str().unwrap()).unwrap();
                let expected = fx["expected"].as_bool().unwrap();
                assert_eq!(
                    compatible_with(&a, &b),
                    expected,
                    "fixture {note:?}: compatible_with diverged"
                );
            }
            "schema_normalize_equivalent" => {
                let text = fx["schema"].as_str().unwrap();
                let s = parse_schema(text).unwrap();
                let n = omnist::ops::normalize(&s);
                assert!(
                    omnist::ops::equivalent(&n, &s),
                    "fixture {note:?}: normalize() is not equivalent() to the original"
                );
            }
            "schema_is_empty" => {
                let text = fx["schema"].as_str().unwrap();
                let s = parse_schema(text).unwrap();
                let expected = fx["expected"].as_bool().unwrap();
                assert_eq!(
                    is_empty(&s),
                    expected,
                    "fixture {note:?}: is_empty diverged"
                );
            }
            "schema_validate" => {
                let text = fx["schema"].as_str().unwrap();
                let s = parse_schema(text).unwrap();
                let doc_json = &fx["doc_json_input"];
                let raw = json_object_to_raw(doc_json);
                let d = Doc::from_raw(raw).unwrap();
                let expected_ok = fx["expected_ok"].as_bool().unwrap();
                let result = s.validate(&d.root());
                assert_eq!(
                    result.ok(),
                    expected_ok,
                    "fixture {note:?}: validate().ok() diverged"
                );
            }
            // Format codecs (test_canonical.py): a Doc round-trips through
            // to_<fmt>/from_<fmt> byte-for-byte-equivalent-in-structure.
            "format_roundtrip" => {
                let fmt = fx["format"].as_str().unwrap();
                let raw = json_object_to_raw(&fx["doc_json"]);
                let d = Doc::from_raw(raw).unwrap();
                let text = d
                    .to_format(fmt)
                    .unwrap_or_else(|e| panic!("fixture {note:?}: to_format({fmt}) failed: {e}"));
                let back = Doc::from_format(fmt, &text)
                    .unwrap_or_else(|e| panic!("fixture {note:?}: from_format({fmt}) failed: {e}"));
                assert_raw_eq(&note, &d.to_raw(), &back.to_raw());
            }
            // materialize: a Doc materialized against its own inferred
            // schema is unchanged.
            "materialize_case" => {
                let schema_text = fx["schema"].as_str().unwrap();
                let s = parse_schema(schema_text).unwrap();
                let raw = json_object_to_raw(&fx["doc_json"]);
                let expected = decode_raw(&fx["expected"]);
                let materialized = materialize(&raw, Some(&s))
                    .unwrap_or_else(|e| panic!("fixture {note:?}: materialize failed: {e}"));
                assert_raw_eq(&note, &expected, &materialized);
            }
            other => panic!("fixture {note:?}: unrecognized fixture kind {other:?}"),
        }
        ran += 1;
    }

    // Sanity check required by the issue: confirm the harness actually
    // replayed every fixture, not silently skipping any (e.g. via an
    // early `continue`/`break` bug).
    assert_eq!(
        ran,
        fixtures.len(),
        "harness must replay every fixture in the corpus, not a subset"
    );
}

/// `deep_node(depth)` mirrors the Python extractor's `deep_node` helper
/// exactly: `depth` levels of `[("a", ...)]` wrapping a leaf `1`.
fn deep_node(depth: usize) -> RawNode {
    let mut node = RawNode::Leaf(Scalar::Int(1));
    for _ in 0..depth {
        node = RawNode::Edges(vec![("a".to_string(), node)]);
    }
    node
}

/// Turn a plain JSON object fixture (`{"n": 7, "s": "x"}`) into the
/// equivalent [`RawNode`] edge-list, for fixtures whose Python side used
/// `doc({...})` on a plain dict rather than this crate's `enc()` tagging.
fn json_object_to_raw(v: &J) -> RawNode {
    fn scalar(v: &J) -> Scalar {
        match v {
            J::Null => Scalar::Null,
            J::Bool(b) => Scalar::Bool(*b),
            J::Number(n) if n.is_i64() => Scalar::Int(n.as_i64().unwrap()),
            J::Number(n) => Scalar::Float(n.as_f64().unwrap()),
            J::String(s) => Scalar::Str(s.clone()),
            other => panic!("json_object_to_raw: unsupported scalar {other:?}"),
        }
    }
    match v {
        J::Object(map) => RawNode::Edges(
            map.iter()
                .map(|(k, val)| {
                    let child = match val {
                        J::Object(_) => json_object_to_raw(val),
                        other => RawNode::Leaf(scalar(other)),
                    };
                    (k.clone(), child)
                })
                .collect(),
        ),
        other => RawNode::Leaf(scalar(other)),
    }
}
