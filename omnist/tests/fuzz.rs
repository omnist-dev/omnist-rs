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
//!   checks), `prune` is idempotent, `extract`'s result only ever keeps
//!   the requested labels, and (issue #32) `compatible_with` is reflexive
//!   and holds for cardinality-narrowed schema variants.
//! - Edge-case generators for extreme integers (near i64's 19-digit
//!   range -- see `document.rs`'s doc comment on why this port has no
//!   4300-digit guard to fuzz), temporal literals near calendar
//!   boundaries, and format-tricky characters.
//! - (issue #32) A bounded, opt-in cross-implementation oracle harness
//!   (`cross_implementation_oracle_bounded_sample`) that shells out to a
//!   live Python `omnist` install and compares `compatible_with`/
//!   `is_empty`/`normalize`+`prune` results against the same schemas
//!   parsed on both sides.
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
//!
//! ## Regression-smoke-test evidence (issue #32 acceptance criterion)
//!
//! Manually verified against a real injected regression: inverting
//! `omnist::ops::subschema::record_sub`'s cardinality-subset comparison
//! (`fb.min <= fa.min` -> `fb.min < fa.min`, so a field whose min is
//! unchanged between A and B -- the common case, including every
//! reflexive `compatible_with(s, s)` -- is wrongly rejected) made both
//! `compatible_with_is_reflexive` and
//! `compatible_with_holds_for_narrowed_cardinality` fail immediately with
//! shrunk counterexamples. Reverting the change made both pass again.
//! See the PR description for the exact diff and captured failure
//! output -- deliberately not left in the tree broken.
//!
//! The same injected bug did *not* trip
//! `cross_implementation_oracle_bounded_sample` even at 300 cases
//! (well above the harness's default budget of 20): that test samples
//! two *independently*-fuzzed schemas, and the bug only manifests when a
//! shared-label field's min is otherwise equal between the two sides --
//! rare for unrelated random schemas, common for the narrowed-variant
//! property above (which perturbs a copy of the same schema). This is a
//! real, useful data point about the oracle's blind spot, not a flaw
//! papered over: independent-pair sampling is well-suited to catching
//! cross-implementation *structural* drift (the kind #26/#32 were most
//! worried about, e.g. a wholesale misreading of the algorithm), but a
//! targeted same-schema property like `compatible_with_holds_for_
//! narrowed_cardinality` is what actually catches a narrow, one-
//! comparison cardinality regression -- which is exactly why this PR
//! adds both rather than relying on the oracle alone.

use std::sync::LazyLock;

use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::{json, toml, xml, yaml};
use omnist::oml;
use omnist::ops::{compatible_with, extract, is_empty, is_isomorphic, normalize, prune};
use omnist::osd;
use omnist::schema::{self, Field, FieldType, Record, Ref, Schema};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

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

/// XML-only leaf generator (omnist-rs#93): since omnist-rs#86, `Str` is
/// the *only* scalar kind `check_xml` accepts as losslessly writable --
/// `Null`/`Bool`/`Int`/`Float` are all now reported as `value.stringified`
/// (XML has no native typed literals, so every non-string scalar reads
/// back as a plain string). `arb_scalar` above is shared by every format's
/// round-trip property and stays fully general on purpose (JSON/YAML/TOML/
/// OML all still need the full scalar mix); narrowing it would silently
/// weaken those properties' coverage too. Instead, `xml_round_trips_when_
/// lossless` gets its own tree built from a string-only leaf, so the
/// `prop_assume!(check_xml(...).is_empty())` filter below has a real
/// chance of passing rather than exhausting proptest's global-reject
/// budget on trees that are almost never XML-lossless by construction
/// (each generated tree has 4-in-8 odds *per leaf* of drawing a non-string
/// scalar, and most trees have several leaves).
fn arb_xml_scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        "[a-zA-Z0-9 _.-]{0,12}".prop_map(Value::Str),
        arb_tricky_string().prop_map(Value::Str),
        arb_temporal_string().prop_map(Value::Str),
    ]
}

/// XML-only counterpart to `arb_value`, built from `arb_xml_scalar`
/// instead of `arb_scalar` -- otherwise identical shape/depth/breadth
/// bounds, so the property still exercises the same tree structures
/// (nesting, branching, empty objects) that the other formats' properties
/// do, just with a leaf kind that XML can actually round-trip losslessly.
fn arb_xml_value(depth: u32) -> BoxedStrategy<Value> {
    if depth == 0 {
        arb_xml_scalar().boxed()
    } else {
        let leaf = arb_xml_scalar();
        let recurse = arb_xml_value(depth - 1);
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

/// XML-only counterpart to `arb_single_root_value`, using `arb_xml_value`.
fn arb_single_root_xml_value(depth: u32) -> impl Strategy<Value = Value> {
    (arb_label(), arb_xml_value(depth)).prop_map(|(k, v)| {
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
    fn xml_round_trips_when_lossless(v in arb_single_root_xml_value(3)) {
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
                        // `Any` (issue #29), added per issue #32's follow-up
                        // comment: the generator previously only ever
                        // produced Scalar/Ref, so every existing schema
                        // property (normalize/prune/extract) silently never
                        // exercised the `any` arm either. A single `Just`
                        // arm alongside the two `Scalar` arms and the `Ref`
                        // arm gives it roughly a 1-in-4 chance per field,
                        // matching Python's `_seeded_random_family`'s
                        // "small fixed probability" intent (see
                        // `~/dev/omnist/tools/semantic_oracle.py`) without
                        // dominating the schema shapes generated.
                        Just(FieldType::Any),
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

// ---------------------------------------------------------------------------
// subschema / compatible_with (issue #32 -- #26's own unmet acceptance item)
// ---------------------------------------------------------------------------

/// One of three cardinality-only narrowings applied to a single field of
/// `s`'s root record, each of which can only *shrink* the set of documents
/// the root record accepts while leaving every field's type untouched:
///
/// - tighten `max` down by one (when `max > min`),
/// - tighten `min` up by one (when `min < max`, or `max` is unbounded),
/// - or force the field to never be emitted (`max = Some(0)`, only when
///   `min == 0` -- otherwise this would make the record unsatisfiable
///   rather than merely narrower).
///
/// This mirrors the issue's suggested "structurally-perturbed variant --
/// narrower cardinality, dropped field" shape more directly than
/// full-blown structural mutation: it's cheap to state a ground-truth
/// expectation for (the narrowed schema is *always* a subschema of the
/// original, by construction, regardless of what the fields' types are),
/// which is exactly what makes it a useful property-test oracle for
/// `compatible_with` without needing Python's hand-built "vindicated
/// universe" -- that universe exists to *label* arbitrary schema pairs
/// ground-truth true/false; this generator instead only ever produces
/// pairs already known to be true by how they were built.
fn narrow_root_field(s: &Schema, field_idx: usize, choice: u8) -> Option<Schema> {
    let root_name = s.root().name.clone();
    let mut env = s.env().clone();
    let root = env.get(&root_name)?.clone();
    let fields = root.fields();
    if fields.is_empty() {
        return None;
    }
    let idx = field_idx % fields.len();
    let f = &fields[idx];
    let (new_min, new_max) = match choice % 3 {
        0 if f.max.is_some_and(|m| m > f.min) => (f.min, f.max.map(|m| m - 1)),
        1 if f.max.is_none_or(|m| f.min < m) => (f.min + 1, f.max),
        2 if f.min == 0 => (0, Some(0)),
        _ => return None,
    };
    let mut new_fields: Vec<Field> = fields.to_vec();
    new_fields[idx] = Field::new(f.label.clone(), f.ty.clone(), new_min, new_max).ok()?;
    env.insert(root_name.clone(), Record::new(new_fields).ok()?);
    Schema::new(Ref::new(root_name), env).ok()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(*CASES))]

    /// `compatible_with` is reflexive: every schema is a subschema of
    /// itself.
    #[test]
    fn compatible_with_is_reflexive(s in arb_schema(3)) {
        prop_assert!(compatible_with(&s, &s));
    }

    /// A schema whose root record has one field cardinality-narrowed (see
    /// [`narrow_root_field`]) accepts a subset of what the original
    /// accepts -- `compatible_with(narrowed, original)` must hold. This is
    /// #26's own unmet acceptance item: `subschema`/`compatible_with` had
    /// zero property-based coverage before issue #32.
    #[test]
    fn compatible_with_holds_for_narrowed_cardinality(
        s in arb_schema(3),
        field_idx in 0usize..8,
        choice in 0u8..3,
    ) {
        if let Some(narrowed) = narrow_root_field(&s, field_idx, choice) {
            prop_assert!(compatible_with(&narrowed, &s));
        }
    }

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

// ---------------------------------------------------------------------------
// Cross-implementation oracle harness (issue #32 -- #26's other unmet
// acceptance item)
// ---------------------------------------------------------------------------
//
// Performance tradeoff: this does NOT run as part of the default
// `cargo test --workspace` / the existing `fuzz` proptest properties
// above. Each case here pays for a fresh `python3` process start plus
// import of `omnist` (tens of milliseconds each), two to three orders of
// magnitude slower per-case than the pure-Rust properties, which is
// exactly why issue #26's own spec (quoted in #32) called for this to be
// "a bounded, budgeted job", not something that runs at the same
// per-property case count (150-250) as the rest of this file.
//
// The harness is entirely opt-in, gated on `OMNIST_ORACLE_PYTHON` (path
// to the venv's `python3`, e.g. `~/dev/venvs/omnist/bin/python3`) being
// set: if it isn't, the test prints why it's skipping and returns
// success rather than failing every contributor's machine and every
// non-`fuzz`-job CI run that doesn't have the sibling `omnist-dev/omnist`
// checkout available. See `.github/workflows/ci.yml`'s `fuzz` job for how
// CI provides it.
static ORACLE_CASES: LazyLock<u32> = LazyLock::new(|| {
    std::env::var("OMNIST_ORACLE_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20)
});

fn oracle_python() -> Option<String> {
    std::env::var("OMNIST_ORACLE_PYTHON").ok()
}

/// Extract a `"key": <bool>` value from the oracle script's one-line JSON
/// output. Hand-rolled instead of pulling in `serde_json` as a new dev-
/// dependency, since the shape here is fixed and fully controlled by
/// `oracle_check.py` alongside it.
fn extract_bool(json: &str, key: &str) -> bool {
    let needle = format!("\"{key}\": ");
    let start = json
        .find(&needle)
        .unwrap_or_else(|| panic!("oracle_check.py output missing {key:?}: {json}"))
        + needle.len();
    json[start..].starts_with("true")
}

/// For a bounded sample of independently-fuzzed schema pairs, shells out
/// to the live Python implementation (via `oracle_check.py`) and checks
/// that `compatible_with`, `is_empty`, and `is_empty(prune(normalize(_)))`
/// agree between the two implementations -- the cross-implementation
/// oracle harness #26 was supposed to include per its own acceptance
/// checklist (quoted in issue #32).
#[test]
fn cross_implementation_oracle_bounded_sample() {
    let Some(python) = oracle_python() else {
        eprintln!(
            "skipping cross_implementation_oracle_bounded_sample: set \
             OMNIST_ORACLE_PYTHON to a python3 executable with `omnist` \
             installed (e.g. `~/dev/venvs/omnist/bin/python3`, after \
             `pip install -e ~/dev/omnist`) to run the live-Python oracle. \
             See .github/workflows/ci.yml's `fuzz` job for how CI wires \
             this up."
        );
        return;
    };
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/oracle_check.py"
    );

    let mut runner = TestRunner::default();
    let pair_strategy = (arb_schema(3), arb_schema(3));
    let dir = std::env::temp_dir();

    for i in 0..*ORACLE_CASES {
        let tree = pair_strategy
            .new_tree(&mut runner)
            .expect("strategy generation should not fail");
        let (a, b) = tree.current();

        let a_path = dir.join(format!("omnist-oracle-{}-{}-a.osd", std::process::id(), i));
        let b_path = dir.join(format!("omnist-oracle-{}-{}-b.osd", std::process::id(), i));
        std::fs::write(&a_path, osd::to_osd(&a, None)).unwrap();
        std::fs::write(&b_path, osd::to_osd(&b, None)).unwrap();

        let output = std::process::Command::new(&python)
            .arg(script)
            .arg(&a_path)
            .arg(&b_path)
            .output()
            .expect("failed to invoke OMNIST_ORACLE_PYTHON");

        std::fs::remove_file(&a_path).ok();
        std::fs::remove_file(&b_path).ok();

        assert!(
            output.status.success(),
            "oracle_check.py failed (case {i}):\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        let stdout = String::from_utf8_lossy(&output.stdout);

        let rust_compat = compatible_with(&a, &b);
        let rust_empty_a = is_empty(&a);
        let rust_np_empty_a = is_empty(&prune(&normalize(&a)));

        let py_compat = extract_bool(&stdout, "compatible_a_b");
        let py_empty_a = extract_bool(&stdout, "is_empty_a");
        let py_np_empty_a = extract_bool(&stdout, "normalize_prune_is_empty_a");

        assert_eq!(
            rust_compat,
            py_compat,
            "compatible_with disagreement (case {i}):\na = {}\nb = {}",
            osd::to_osd(&a, Some(2)),
            osd::to_osd(&b, Some(2)),
        );
        assert_eq!(
            rust_empty_a,
            py_empty_a,
            "is_empty disagreement (case {i}):\na = {}",
            osd::to_osd(&a, Some(2)),
        );
        assert_eq!(
            rust_np_empty_a,
            py_np_empty_a,
            "is_empty(prune(normalize(_))) disagreement (case {i}):\na = {}",
            osd::to_osd(&a, Some(2)),
        );
    }
}
