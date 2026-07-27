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
//! `ops` algebra (`compatible_with`, `normalize`, `is_empty`, `lint`), the
//! JSON/YAML/TOML/XML format codecs, `infer`, and `materialize`. Issue #61
//! expanded this from #40's original 44 fixtures to a much larger corpus:
//! `test_lint.py`'s 9 cases (previously wrongly excluded as "tooling"), and
//! `test_canonical.py`'s `TestDocument`/`TestInfer`/`TestValidation`/
//! `TestOsdRobustness`/`TestTemporalBoundary`/`TestOperations` expanded from
//! ~1 fixture/class toward ~1 fixture/method, plus a lighter pass over
//! `test_depth_guards.py`. Deliberately *not* covered by this corpus (see
//! `extract_fixtures.py`'s module docstring for the full rationale on each):
//!
//! - `test_any_core.py` / `test_any_grammar.py` — the `any` type's OSD
//!   *grammar edge cases* (basic `any`-field parsing already has Rust
//!   support and is exercised indirectly via the lint fixtures' any-field
//!   check) are still a later PR per the v1.0 `any` decision.
//! - `test_public_api.py` — freezes *Python's* `omnist.__all__` import
//!   surface; not a cross-language concept.
//! - `test_cli.py` / `test_cli_examples.py` / `test_cli_fuzz.py` — Python
//!   CLI/argparse plumbing, not a Document/Schema API call.
//! - `test_examples*.py`, `test_docs.py`, `test_check_doc_examples.py`,
//!   `test_grammar_docs.py` — doc-example/README/packaging generators for
//!   the Python repo's own tooling.
//! - `test_fuzz.py` — already ported at omnist-rs issue #26
//!   (`omnist/tests/fuzz.rs`), which includes a live cross-implementation
//!   oracle.
//! - `test_semantic_oracle.py` — exercises `tools/semantic_oracle.py`, the
//!   Python-only dev tool `fuzz.rs`'s oracle already shells out to.
//! - The other ~20 classes in `test_canonical.py` (`TestExtract`,
//!   `TestNormalizePartitionRefinement`, `TestEmptySchemas`, registry/
//!   plugin/repr-dunder checks, etc.) — out of scope for issue #61, which
//!   named only the six classes above; most cover algorithms not yet
//!   ported or Python-specific plumbing with no Rust call site to replay.
//!
//! No genuine Python bug was found by this pass: every fixture's
//! `expected`/`error_contains` value is asserted as-is against Rust with
//! no divergence needed. (Running tally per the port's cross-
//! implementation bug policy: issue #4 -> omnist-dev/omnist#255, issue #42
//! -> omnist-dev/omnist#256; this pass adds no new entries.)

use omnist::document::{Doc, RawNode, Scalar, Value};
use omnist::materialize;
use omnist::oml::{read_oml, write_oml};
use omnist::ops::{compatible_with, is_empty, lint};
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
        fixtures.len() >= 120,
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
                let s = parse_schema(input)
                    .unwrap_or_else(|e| panic!("fixture {note:?}: parse_schema failed: {e}"));
                if let Some(root) = fx.get("expected_root").and_then(|v| v.as_str()) {
                    assert_eq!(
                        s.root().name,
                        root,
                        "fixture {note:?}: parsed schema's root name diverged"
                    );
                }
            }
            // OSD parser robustness / TestValidation's OSD-error tests: the
            // input is rejected -- Python's SchemaError wording is Python-
            // specific, so (matching `oml_parse_error`'s precedent) only
            // "Rust also rejects this input" is asserted, not exact text.
            "osd_parse_error" => {
                let input = fx["input"].as_str().unwrap();
                let err = parse_schema(input).err().unwrap_or_else(|| {
                    panic!("fixture {note:?}: expected parse_schema({input:?}) to fail")
                });
                let _ = err.to_string(); // Display must not panic
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

            // --- issue #61 additions below --------------------------------

            // test_lint.py: omnist.ops.lint's four structural checks,
            // compared as (code, severity, location) triples -- message
            // text uses Python `repr()` vs Rust `Debug` quoting and isn't
            // asserted (see the extractor's `lint_triples` comment).
            "lint_case" => {
                let s = parse_schema(fx["schema"].as_str().unwrap()).unwrap();
                let findings = lint(&s);
                let actual: Vec<(String, String, String)> = findings
                    .iter()
                    .map(|f| {
                        (
                            f.code.to_string(),
                            f.severity.to_string(),
                            f.location.clone(),
                        )
                    })
                    .collect();
                let expected: Vec<(String, String, String)> = fx["expected"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|triple| {
                        let t = triple.as_array().unwrap();
                        (
                            t[0].as_str().unwrap().to_string(),
                            t[1].as_str().unwrap().to_string(),
                            t[2].as_str().unwrap().to_string(),
                        )
                    })
                    .collect();
                assert_eq!(actual, expected, "fixture {note:?}: lint findings diverged");
            }
            // test_lint_finding_is_frozen: Rust's `LintFinding` has no
            // setter (Python enforces immutability via a frozen dataclass
            // instead) -- this exercises construction + the derived
            // Clone/PartialEq rather than a mutation-attempt/exception.
            "lint_finding_shape" => {
                let code = fx["code"].as_str().unwrap();
                let severity = fx["severity"].as_str().unwrap();
                let location = fx["location"].as_str().unwrap().to_string();
                let message = fx["message"].as_str().unwrap().to_string();
                let code_static: &'static str = match code {
                    "unsatisfiable-record" => "unsatisfiable-record",
                    "unreachable-record" => "unreachable-record",
                    "duplicate-record" => "duplicate-record",
                    "any-field" => "any-field",
                    other => panic!("fixture {note:?}: unrecognized lint code {other:?}"),
                };
                let severity_static: &'static str = match severity {
                    "warning" => "warning",
                    "info" => "info",
                    other => panic!("fixture {note:?}: unrecognized severity {other:?}"),
                };
                let finding = omnist::ops::LintFinding {
                    code: code_static,
                    severity: severity_static,
                    location: location.clone(),
                    message: message.clone(),
                };
                assert_eq!(
                    finding.clone(),
                    finding,
                    "fixture {note:?}: LintFinding PartialEq broken"
                );
                assert_eq!(finding.location, location);
                assert_eq!(finding.message, message);
            }
            // test_lint_does_not_mutate: lint(&s) borrows, never mutates.
            "lint_no_mutation" => {
                let text = fx["schema"].as_str().unwrap();
                let s = parse_schema(text).unwrap();
                let before: Vec<String> = s.env().keys().cloned().collect();
                let _ = lint(&s);
                let after: Vec<String> = s.env().keys().cloned().collect();
                assert_eq!(
                    before, after,
                    "fixture {note:?}: lint(s) must not mutate s's env"
                );
            }

            // test_canonical.py TestDocument: labels()/count() navigation.
            "doc_query" => {
                let raw = decode_raw(&fx["initial"]);
                let d = Doc::from_raw(raw)
                    .unwrap_or_else(|e| panic!("fixture {note:?}: Doc::from_raw failed: {e}"));
                let root = d.root();
                let expected_labels: Vec<String> = fx["expected_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                assert_eq!(
                    root.labels(),
                    expected_labels,
                    "fixture {note:?}: labels() diverged"
                );
                for (label, cnt) in fx["expected_counts"].as_object().unwrap() {
                    let expected_cnt = cnt.as_u64().unwrap() as usize;
                    assert_eq!(
                        root.count(label),
                        expected_cnt,
                        "fixture {note:?}: count({label:?}) diverged"
                    );
                }
            }
            // test_to_data_is_edge_list: `to_raw()` (not `to_data()`, which
            // intentionally dedups repeated labels for structural-equality
            // use -- see document.rs's own doc comment) is this crate's
            // edge-list-preserving projection, the direct analogue of
            // Python's `Doc.to_data()`.
            "doc_to_data" => {
                let raw = decode_raw(&fx["initial"]);
                let d = Doc::from_raw(raw).unwrap();
                let expected = decode_raw(&fx["expected"]);
                assert_raw_eq(&note, &expected, &d.to_raw());
            }
            // test_editing / test_set_*: replay an add/set/remove op
            // sequence against the root, then compare the final edge list.
            "doc_ops" => {
                let raw = decode_raw(&fx["initial"]);
                let mut d = Doc::from_raw(raw).unwrap();
                let root_id = d.root().id();
                for op in fx["ops"].as_array().unwrap() {
                    apply_doc_op(&mut d, root_id, op, &note);
                }
                let expected = decode_raw(&fx["expected"]);
                assert_raw_eq(&note, &expected, &d.to_raw());
            }
            // test_set_replace_all_matches_remove_then_add_docstring_contract:
            // the one documented divergence between set() and remove()+add()
            // -- same starting doc, two op sequences, two different results.
            "doc_ops_pair" => {
                let mut da = Doc::from_raw(decode_raw(&fx["initial"])).unwrap();
                let ida = da.root().id();
                for op in fx["ops_a"].as_array().unwrap() {
                    apply_doc_op(&mut da, ida, op, &note);
                }
                let mut db = Doc::from_raw(decode_raw(&fx["initial"])).unwrap();
                let idb = db.root().id();
                for op in fx["ops_b"].as_array().unwrap() {
                    apply_doc_op(&mut db, idb, op, &note);
                }
                assert_raw_eq(
                    &format!("{note} (a)"),
                    &decode_raw(&fx["expected_a"]),
                    &da.to_raw(),
                );
                assert_raw_eq(
                    &format!("{note} (b)"),
                    &decode_raw(&fx["expected_b"]),
                    &db.to_raw(),
                );
            }

            // test_canonical.py TestInfer: infer() + the resulting schema's
            // validate() on each (doc, expected_ok) check.
            "infer_case" => {
                let samples: Vec<Doc> = fx["samples"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| Doc::from_raw(json_object_to_raw(s)).unwrap())
                    .collect();
                let s = omnist::infer(&samples, "Root")
                    .unwrap_or_else(|e| panic!("fixture {note:?}: infer failed: {e}"));
                for check in fx["checks"].as_array().unwrap() {
                    let arr = check.as_array().unwrap();
                    let d = Doc::from_raw(json_object_to_raw(&arr[0])).unwrap();
                    let expected_ok = arr[1].as_bool().unwrap();
                    assert_eq!(
                        s.validate(&d.root()).ok(),
                        expected_ok,
                        "fixture {note:?}: infer+validate diverged for {:?}",
                        arr[0]
                    );
                }
            }
            "infer_error" => {
                let samples: Vec<Doc> = fx["samples"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| Doc::from_raw(json_object_to_raw(s)).unwrap())
                    .collect();
                assert!(
                    omnist::infer(&samples, "Root").is_err(),
                    "fixture {note:?}: expected infer to fail on conflicting scalar kinds"
                );
            }
            "infer_order_independent" => {
                let sa: Vec<Doc> = fx["samples_a"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| Doc::from_raw(json_object_to_raw(s)).unwrap())
                    .collect();
                let sb: Vec<Doc> = fx["samples_b"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| Doc::from_raw(json_object_to_raw(s)).unwrap())
                    .collect();
                let schema_a = omnist::infer(&sa, "Root").unwrap();
                let schema_b = omnist::infer(&sb, "Root").unwrap();
                assert!(
                    omnist::ops::equivalent(&schema_a, &schema_b),
                    "fixture {note:?}: order-independence diverged"
                );
                let root_rec = schema_a.env().get("Root").expect("Root record must exist");
                let port = root_rec.field("port").expect("port field must exist");
                let expected_min = fx["expected_port_min"].as_u64().unwrap() as usize;
                let expected_max = fx["expected_port_max"].as_u64().map(|v| v as usize);
                assert_eq!(
                    port.min, expected_min,
                    "fixture {note:?}: port.min diverged"
                );
                assert_eq!(
                    port.max, expected_max,
                    "fixture {note:?}: port.max diverged"
                );
                for check in fx["checks"].as_array().unwrap() {
                    let arr = check.as_array().unwrap();
                    let d = Doc::from_raw(json_object_to_raw(&arr[0])).unwrap();
                    let expected_ok = arr[1].as_bool().unwrap();
                    assert_eq!(schema_a.validate(&d.root()).ok(), expected_ok);
                }
            }

            // test_closed_rejects_unexpected: check the stable error code,
            // not the English message text.
            "schema_validate_error_code" => {
                let s = parse_schema(fx["schema"].as_str().unwrap()).unwrap();
                let raw = json_object_to_raw(&fx["doc_json_input"]);
                let d = Doc::from_raw(raw).unwrap();
                let result = s.validate(&d.root());
                let expected_code = fx["expected_code"].as_str().unwrap();
                assert!(
                    !result.ok(),
                    "fixture {note:?}: expected validation to fail"
                );
                assert!(
                    result
                        .errors()
                        .iter()
                        .any(|e| e.code.as_str() == expected_code),
                    "fixture {note:?}: expected error code {expected_code:?} not found in {:?}",
                    result.errors()
                );
            }

            // test_equivalent_reordered
            "schema_equivalent" => {
                let a = parse_schema(fx["schema_a"].as_str().unwrap()).unwrap();
                let b = parse_schema(fx["schema_b"].as_str().unwrap()).unwrap();
                let expected = fx["expected"].as_bool().unwrap();
                assert_eq!(
                    omnist::ops::equivalent(&a, &b),
                    expected,
                    "fixture {note:?}: equivalent diverged"
                );
            }
            // test_normalize_merges_identical: env shrinks and stays
            // equivalent.
            "schema_normalize_merges" => {
                let s = parse_schema(fx["schema"].as_str().unwrap()).unwrap();
                let n = omnist::ops::normalize(&s);
                let before = fx["expected_env_before"].as_u64().unwrap() as usize;
                let after = fx["expected_env_after"].as_u64().unwrap() as usize;
                assert_eq!(
                    s.env().len(),
                    before,
                    "fixture {note:?}: env-before diverged"
                );
                assert_eq!(n.env().len(), after, "fixture {note:?}: env-after diverged");
                assert!(
                    n.env().len() < s.env().len(),
                    "fixture {note:?}: normalize must shrink env"
                );
                assert!(
                    omnist::ops::equivalent(&s, &n),
                    "fixture {note:?}: normalize output not equivalent"
                );
            }

            // test_depth_guards.py (lighter pass): Rust centralizes its
            // depth guard at Doc construction, not per-writer -- see the
            // extractor's comment on why one from_raw-level fixture pair
            // covers the same guarantee test_depth_guards.py checks per
            // format.
            "doc_construct_depth_error" => {
                let depth = fx["depth"].as_u64().unwrap() as usize;
                let err = Doc::from_raw(deep_node(depth))
                    .err()
                    .unwrap_or_else(|| panic!("fixture {note:?}: expected a DocumentError"));
                let msg = err.to_string();
                assert!(
                    msg.contains("nesting exceeds the maximum depth"),
                    "fixture {note:?}: unexpected error message: {msg}"
                );
            }
            "doc_construct_depth_ok" => {
                let depth = fx["depth"].as_u64().unwrap() as usize;
                Doc::from_raw(deep_node(depth))
                    .unwrap_or_else(|e| panic!("fixture {note:?}: Doc::from_raw failed: {e}"));
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
///
/// A JSON array value (`{"tags": ["a", "b"]}`) is expanded into *repeated*
/// edges under the same label (`tags: "a"`, `tags: "b"`) -- matching
/// Python's `doc()` convention that "many x" is the label `x` occurring
/// more than once, not a field pointing to an array (issue #61's
/// `TestInfer`/`TestDocument` fixtures need this; #40's original fixtures
/// never exercised a list-valued field).
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
    fn value_to_raw(v: &J) -> RawNode {
        match v {
            J::Object(_) => json_object_to_raw(v),
            other => RawNode::Leaf(scalar(other)),
        }
    }
    match v {
        J::Object(map) => {
            let mut edges = Vec::new();
            for (k, val) in map {
                match val {
                    J::Array(items) => {
                        for item in items {
                            edges.push((k.clone(), value_to_raw(item)));
                        }
                    }
                    other => edges.push((k.clone(), value_to_raw(other))),
                }
            }
            RawNode::Edges(edges)
        }
        other => RawNode::Leaf(scalar(other)),
    }
}

/// Decode an `enc()`-tagged *scalar* JSON value (`{"$int": 7}`) into this
/// crate's [`Value`] -- the mutable "plain input" type `Doc::add`/`Doc::set`
/// take, as opposed to [`RawNode`] (`Doc::from_raw`'s already-built-tree
/// input). Only scalars appear as op values in the `doc_ops`/`doc_ops_pair`
/// fixtures below (no fixture here builds a nested subtree via add/set).
fn scalar_value_from_enc(v: &J) -> Value {
    let obj = v.as_object().expect("encoded scalar must be a JSON object");
    if let Some(b) = obj.get("$null") {
        assert_eq!(b, &J::Bool(true));
        return Value::Null;
    }
    if let Some(J::Bool(b)) = obj.get("$bool") {
        return Value::Bool(*b);
    }
    if let Some(n) = obj.get("$int") {
        return Value::Int(n.as_i64().expect("$int must fit in i64"));
    }
    if let Some(n) = obj.get("$float") {
        let f = match n {
            J::String(s) if s == "nan" => f64::NAN,
            J::String(s) if s == "inf" => f64::INFINITY,
            J::String(s) if s == "-inf" => f64::NEG_INFINITY,
            other => other.as_f64().expect("$float must be a JSON number"),
        };
        return Value::Float(f);
    }
    if let Some(J::String(s)) = obj.get("$str") {
        return Value::Str(s.clone());
    }
    panic!("unrecognized encoded scalar value: {v:?}");
}

/// Replay one `{"op": "add"|"set"|"remove", "label": ..., "value": ...}`
/// entry (from a `doc_ops`/`doc_ops_pair` fixture) against `d`'s root.
fn apply_doc_op(d: &mut Doc, at: omnist::document::NodeId, op: &J, note: &str) {
    let kind = op["op"].as_str().expect("op needs a `op` string");
    let label = op["label"].as_str().expect("op needs a `label` string");
    match kind {
        "add" => {
            let value = scalar_value_from_enc(&op["value"]);
            d.add(at, "$", label, &value)
                .unwrap_or_else(|e| panic!("fixture {note:?}: add({label:?}) failed: {e}"));
        }
        "set" => {
            let value = scalar_value_from_enc(&op["value"]);
            d.set(at, "$", label, &value)
                .unwrap_or_else(|e| panic!("fixture {note:?}: set({label:?}) failed: {e}"));
        }
        "remove" => {
            d.remove(at, "$", label)
                .unwrap_or_else(|e| panic!("fixture {note:?}: remove({label:?}) failed: {e}"));
        }
        other => panic!("fixture {note:?}: unrecognized doc op {other:?}"),
    }
}
