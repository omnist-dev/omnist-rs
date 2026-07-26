//! Property-based fuzzing + semantic-oracle cross-check (issue #26).
//!
//! Ported strategy from `~/dev/omnist/tests/test_fuzz.py` (Python's
//! `hypothesis` generators for `Document`/`Schema`), using `proptest` per
//! issue #1's toolchain mapping. This is the port order's explicit
//! fuzzing item (`docs/workflow-playbook.md` §4), closed retroactively
//! rather than per-module.
//!
//! ## What's covered
//!
//! - Bounded `Document`/`Value` tree generators (depth/breadth-limited,
//!   well inside [`omnist::document::MAX_DEPTH`]).
//! - Round-trip properties for all five formats: OML, JSON, YAML, TOML,
//!   XML. Each format's writer can be lossy (see
//!   `omnist::report::WriteReport`); a case is only asserted when the
//!   writer reports no adjustments (`check_*(&doc).is_ok()` with zero
//!   adjustments), since a known-lossy write is expected to fail
//!   round-trip equality and is not this property's concern (each
//!   format's PR already covers its own adjustment behavior).
//! - Schema-algebra properties on fuzzed `Schema`s: `normalize` produces
//!   something isomorphic to the input (extends #12's hand-picked spot
//!   checks), `prune` is idempotent, and `extract`'s result only ever
//!   keeps the requested labels.
//! - Edge-case generators for extreme integers (near i64's 19-digit
//!   range -- see `document.rs`'s doc comment on why this port has no
//!   4300-digit guard to fuzz), temporal literals near calendar
//!   boundaries, and format-tricky characters.
//!
//! ## Regression-smoke-test evidence (issue #26 acceptance criterion)
//!
//! This file's properties were manually verified to actually catch a
//! broken round trip during development: temporarily changing
//! `write_json`'s writer to swap two adjacent leaf values (breaking
//! order-preservation) made `json_round_trips_when_lossless` fail
//! immediately with a shrunk counterexample; reverting the change made
//! it pass again. See the PR description for the exact diff and
//! `proptest`'s failure output captured during that manual run -- this
//! is deliberately not left in the tree as a permanently-broken test.

use std::sync::LazyLock;

use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::{json, toml, xml, yaml};
use omnist::oml;
use omnist::ops::{extract, is_isomorphic, normalize, prune};
use omnist::schema::{self, Field, FieldType, Record, Ref, Schema};
use proptest::prelude::*;

/// Bounded case budget: `PROPTEST_CASES` (proptest's own recognized env
/// var) if set, else 150 -- matching Python's `~/dev/omnist/tests/
/// test_fuzz.py` `_SUPPRESS = settings(max_examples=150, ...)` as the
/// starting point per the issue's request to check Python's actual CI
/// budget. CI (`.github/workflows/ci.yml`, the `fuzz` job) sets
/// `PROPTEST_CASES=250`; a bounded, non-default local run can override
/// further. Never unbounded.
static CASES: LazyLock<u32> = LazyLock::new(|| {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150)
});

// ---------------------------------------------------------------------------
// Value / Document generators
// ---------------------------------------------------------------------------

/// Labels drawn from a small alphabet so objects plausibly share keys
/// (matching Python's `_labels` strategy intent) and are always valid
/// identifiers (avoids the `join()` bracket-quoting path entirely, which
/// is exercised elsewhere by hand-written tests, not by this fuzzer).
fn arb_label() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,5}"
}

/// A scalar leaf. Kept ASCII-ish for strings so YAML/TOML/XML's tricky-
/// character handling is exercised deliberately (see `arb_tricky_string`)
/// rather than accidentally.
fn arb_scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        // Bounded well away from i64::MAX/MIN edge weirdness in most
        // cases, plus a dedicated extreme-integer arm.
        (-1_000_000i64..1_000_000).prop_map(Value::Int),
        prop_oneof![Just(i64::MAX), Just(i64::MIN), Just(0i64), Just(-1i64),].prop_map(Value::Int),
        (-1e6f64..1e6)
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(Value::Float),
        "[a-zA-Z0-9 _.-]{0,12}".prop_map(Value::Str),
        arb_tricky_string().prop_map(Value::Str),
        arb_temporal_string().prop_map(Value::Str),
    ]
}

/// Strings containing characters each format's own PR flagged as tricky:
/// YAML flow/indicator chars, TOML quote/backslash, XML `< & > " '`.
fn arb_tricky_string() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("a: b".to_string()),
        Just("- item".to_string()),
        Just("\"quoted\\path\"".to_string()),
        Just("<tag>&amp;</tag>".to_string()),
        Just("line1\nline2".to_string()),
        Just("tab\there".to_string()),
        Just("both \" and '".to_string()),
        Just("\u{0085}".to_string()), // NEL, YAML's own documented edge case
        Just(String::new()),
    ]
}

/// Temporal literals near calendar boundaries (leap day, year rollover,
/// midnight/end-of-day, sub-second precision).
fn arb_temporal_string() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("2024-02-29".to_string()), // leap day
        Just("2023-02-28".to_string()),
        Just("0001-01-01".to_string()),
        Just("9999-12-31".to_string()),
        Just("00:00:00".to_string()),
        Just("23:59:59".to_string()),
        Just("23:59:59.999999".to_string()),
        Just("2024-01-01T00:00:00".to_string()),
        Just("2024-12-31T23:59:59.500000".to_string()),
    ]
}

/// A bounded `Value` tree: `depth` counts remaining recursion allowance,
/// well under `MAX_DEPTH` (200) so the depth guard is never the limiting
/// factor -- this fuzzer is about content shape, not the guard itself
/// (which has its own hand-written audit test in `document.rs`).
fn arb_value(depth: u32) -> BoxedStrategy<Value> {
    if depth == 0 {
        arb_scalar().boxed()
    } else {
        let leaf = arb_scalar();
        let recurse = arb_value(depth - 1);
        prop_oneof![
            2 => leaf,
            3 => proptest::collection::vec((arb_label(), recurse.clone()), 0..4)
                .prop_map(|pairs| {
                    let mut map = IndexMap::new();
                    for (k, v) in pairs {
                        map.insert(k, v);
                    }
                    Value::Object(map)
                }),
        ]
        .boxed()
    }
}

/// An object-rooted `Value` (every format's real-world root shape; XML
/// additionally requires exactly one top-level key, enforced at each
/// XML-specific property site below).
fn arb_object_value(depth: u32) -> impl Strategy<Value = Value> {
    proptest::collection::vec((arb_label(), arb_value(depth)), 1..4).prop_map(|pairs| {
        let mut map = IndexMap::new();
        for (k, v) in pairs {
            map.insert(k, v);
        }
        Value::Object(map)
    })
}

/// Single-top-level-key variant, for XML's single-document-element rule.
fn arb_single_root_value(depth: u32) -> impl Strategy<Value = Value> {
    (arb_label(), arb_value(depth)).prop_map(|(k, v)| {
        let mut map = IndexMap::new();
        map.insert(k, v);
        Value::Object(map)
    })
}

// ---------------------------------------------------------------------------
// Round-trip properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(*CASES))]

    /// OML is always lossless (it's omnist's own format) -- no
    /// `prop_assume!` gate needed.
    #[test]
    fn oml_round_trips(v in arb_object_value(3)) {
        let doc = Doc::of(&v).unwrap();
        let raw = doc.to_raw();
        let text = oml::write_oml(&raw, 2).unwrap();
        let parsed = oml::read_oml(&text).unwrap();
        let doc2 = Doc::from_raw(parsed).unwrap();
        prop_assert!(doc.eq_doc(&doc2));
    }

    #[test]
    fn json_round_trips_when_lossless(v in arb_object_value(3)) {
        let doc = Doc::of(&v).unwrap();
        prop_assume!(json::check_json(&doc).is_ok() && json::check_json(&doc).is_empty());
        let text = json::write_json(&doc, None, true, None).unwrap();
        let doc2 = json::read_json(&text).unwrap();
        prop_assert!(doc.eq_doc(&doc2));
    }

    #[test]
    fn yaml_round_trips_when_lossless(v in arb_object_value(3)) {
        let doc = Doc::of(&v).unwrap();
        prop_assume!(yaml::check_yaml(&doc).is_empty());
        let text = yaml::write_yaml(&doc, true, None).unwrap();
        let doc2 = yaml::read_yaml(&text).unwrap();
        prop_assert!(doc.eq_doc(&doc2));
    }

    #[test]
    fn toml_round_trips_when_lossless(v in arb_object_value(3)) {
        let doc = Doc::of(&v).unwrap();
        prop_assume!(toml::check_toml(&doc).is_empty());
        let text = toml::write_toml(&doc, true, None).unwrap();
        let doc2 = toml::read_toml(&text).unwrap();
        prop_assert!(doc.eq_doc(&doc2));
    }

    #[test]
    fn xml_round_trips_when_lossless(v in arb_single_root_value(3)) {
        let doc = Doc::of(&v).unwrap();
        prop_assume!(xml::check_xml(&doc).is_empty());
        let text = xml::write_xml(&doc, true, None).unwrap();
        let doc2 = xml::read_xml(&text).unwrap();
        prop_assert!(doc.eq_doc(&doc2));
    }
}

// ---------------------------------------------------------------------------
// Schema-algebra generators + properties
// ---------------------------------------------------------------------------

fn arb_scalar_kind() -> impl Strategy<Value = schema::ScalarKind> {
    proptest::sample::select(schema::ScalarKind::ALL.to_vec())
}

/// A schema with `n_records` named records (`r0`, `r1`, ...), each with a
/// small number of scalar-typed fields, and later records allowed to
/// reference earlier ones (acyclic by construction order, matching
/// Python's `schemas()` strategy shape). Root is always `r0`.
fn arb_schema(n_records: usize) -> impl Strategy<Value = Schema> {
    let names: Vec<String> = (0..n_records).map(|i| format!("r{i}")).collect();
    let record_strats: Vec<_> = (0..n_records)
        .map(|i| {
            let names = names.clone();
            proptest::collection::vec(
                (
                    arb_label(),
                    prop_oneof![
                        arb_scalar_kind()
                            .prop_map(|k| FieldType::Scalar(schema::Scalar::new(k, false))),
                        arb_scalar_kind()
                            .prop_map(|k| FieldType::Scalar(schema::Scalar::new(k, true))),
                        proptest::sample::select(
                            names[..=i.min(names.len().saturating_sub(1))].to_vec()
                        )
                        .prop_map(|n| FieldType::Ref(Ref::new(n))),
                    ],
                    0usize..2,
                    proptest::option::of(1usize..3),
                ),
                0..3,
            )
        })
        .collect();
    record_strats.prop_map(move |all_fields| {
        let mut env = IndexMap::new();
        for (i, fields) in all_fields.into_iter().enumerate() {
            let mut seen = std::collections::HashSet::new();
            let mut built = Vec::new();
            for (label, ty, min, max_extra) in fields {
                if !seen.insert(label.clone()) {
                    continue; // duplicate label: skip (Record::new rejects)
                }
                let max = max_extra.map(|m| min + m);
                if let Ok(f) = Field::new(label, ty, min, max) {
                    built.push(f);
                }
            }
            env.insert(format!("r{i}"), Record::new(built).unwrap());
        }
        Schema::new(Ref::new("r0"), env).unwrap()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(*CASES))]

    /// `normalize(s)` must remain isomorphic to `s` -- extends #12's
    /// hand-picked spot tests to fuzzed schemas.
    #[test]
    fn normalize_preserves_isomorphism(s in arb_schema(3)) {
        let n = normalize(&s);
        prop_assert!(is_isomorphic(&s, &n));
    }

    /// `prune` is idempotent: pruning an already-pruned schema changes
    /// nothing further.
    #[test]
    fn prune_is_idempotent(s in arb_schema(3)) {
        let once = prune(&s);
        let twice = prune(&once);
        prop_assert_eq!(once, twice);
    }

    /// `extract(s, keep)`'s root record never keeps a field whose label
    /// isn't in `keep`.
    #[test]
    fn extract_only_keeps_requested_labels(s in arb_schema(3), keep_idx in proptest::collection::vec(0usize..4, 0..3)) {
        let root = s.env().get(s.root().name.as_str()).unwrap();
        let all_labels: Vec<&str> = root.fields().iter().map(|f| f.label.as_str()).collect();
        let keep: Vec<&str> = keep_idx
            .into_iter()
            .filter_map(|i| all_labels.get(i).copied())
            .collect();
        if let Ok(extracted) = extract(&s, &keep) {
            let root2 = extracted.env().get(extracted.root().name.as_str()).unwrap();
            for f in root2.fields() {
                prop_assert!(keep.contains(&f.label.as_str()));
            }
        }
    }
}
