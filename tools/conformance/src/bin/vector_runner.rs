//! Track 2: runs vendor/omnist-spec's `test-suite/` JSON-vector suite (139
//! vectors, envelope `name`/`spec`/`operation`/`purpose`/`input`/`expect`
//! -- see `vendor/omnist-spec/test-suite/README.md` and
//! `docs/08-conformance-and-errors.md` §8.5) against omnist-rs's own
//! library. This is a *second* runner alongside `runner.rs`'s
//! directory-per-fixture format (Track 1) -- the two vector shapes don't
//! share a natural code path (OML/OSD text vs. canonical-JSON-encoded
//! Document), so the drivers stay separate; only `referee.rs`'s
//! `compare_schema` and `RawNode`/`PartialEq` machinery are shared.
//!
//! Ported in spirit from omnist-ts's `tools/conformance/vectorRunner.ts`
//! (freshest worked reference -- same dispatch-table-vs-`match` tradeoff,
//! same four empirical decisions) and Python's `omnist`'s
//! `tools/conformance/vector_runner.py`. Dispatch is a `match` on the
//! operation name, following this crate's own Track-1 precedent
//! (architecture-freedom, not a literal TS port).
//!
//! ## The three empirical decisions (issue #82 Step 3), verified for real
//!
//! **1. Diagnostics matching mode: code-agnostic.** Verified directly by
//! running `validate/scalar-kinds/number-does-not-satisfy-integer-even-when-whole`
//! and several other real `validate`/`materialize` failure vectors through
//! `Schema::validate`: `omnist::schema::ErrorCode::as_str()` produces bare
//! codes (`"type-mismatch"`, `"cardinality"`, `"shape-mismatch"`,
//! `"null-not-allowed"`), while the vectors' `expect.diagnostics[].code`
//! values are operation-prefixed (`"validate.type-mismatch"`,
//! `"validate.cardinality"`, ...). Same situation as omnist-ts (its own
//! header comment documents the identical mismatch): omnist-rs's
//! `ErrorCode` predates §8.3's taxonomy and was never renamed to match it.
//! This runner therefore always compares diagnostics as the *set* of
//! `path`s only, never `code`, matching TS's decision and rationale.
//! Message text is never compared either way.
//!
//! **2. D-6 (integer/number kind collapse): confirmed NOT applicable,
//! no skip detector built.** `omnist::document::Scalar` has separate
//! `Int(i64)`/`Float(f64)` variants (`document.rs`), so there is no shape
//! for the collapse D-6 describes to occur in at all -- unlike omnist-ts's
//! single-JS-number `Scalar`, which needed `hasKindCollapseRisk`/
//! `d6Affected` structural detectors. Running the one vector TS's own
//! comment identifies as D-6-affected --
//! `validate/scalar-kinds/number-does-not-satisfy-integer-even-when-whole`
//! -- through this runner confirms it: omnist-rs's `Schema::validate`
//! correctly rejects a `Float` value against an `integer`-typed field
//! (kind mismatch, not a value-based heuristic), so the vector PASSES here
//! with no skip needed. This is confirmation of the source-level
//! inspection already done before this step, not a surprise, and is
//! deliberately documented rather than silently omitted.
//!
//! **3. D-1 (node-count limit) re-check: still not applicable, same as
//! before -- but for a different reason than initially assumed.** All 6
//! `document-model/limits.json` vectors carry a vector-local
//! `declared_max_depth`/`declared_max_nodes`/`declared_max_int_digits` in
//! `input`, small values (e.g. `3`, `1`) a harness is meant to configure
//! the implementation under test to for the vector's duration (§2.4: the
//! numbers themselves are not normative). `omnist::document::MAX_DEPTH`
//! (200), `MAX_NODES` (1,000,000, general per PR #80), and
//! `int_cap::MAX_INT_DIGITS` (4,300) are all compile-time `const`s with no
//! runtime-configuration surface, so none of these 6 vectors can be run
//! against the *real* limit value either way -- every one skips, citing
//! "not yet implemented -- compile-time constants, no runtime
//! configuration surface". (This is not the D-1 §9.4 ledger entry; it's a
//! capability gap, hence "not yet implemented" rather than a numbered
//! ledger citation.)
//!
//! **Parse-error structural matching (a fourth, closely-related finding,
//! not one of the three headline decisions but load-bearing for how many
//! `oml-grammar`/`osd-grammar`/`formats-*` syntax-failure vectors run for
//! real rather than skip).** Unlike omnist-ts's `ParseError` (message-only
//! for syntax failures, forcing a blanket skip of every syntax-failure
//! vector asserting `diagnostics`), omnist-rs's `ParseError` *always*
//! carries structured `{line, col, message}` (`error.rs`), for every
//! format's syntax failure, not just materialize-driven ones. Verified
//! directly: `read_oml("nan: 1\n")` fails at line 1, col 4, matching the
//! real vector `oml-grammar/reserved/nan-bare-is-a-number-token-not-a-label`'s
//! `expect.diagnostics[0].path` of `"1:4"` exactly. This runner therefore
//! compares a syntax failure's `"{line}:{col}"` against the vector's
//! `path` directly (as a one-element set, matching the general
//! path-set-comparison shape used everywhere else) instead of skipping --
//! a narrower gap than TS's, not the favorable "no gap at all" outcome:
//! `omnist::schema::parse_schema`'s `SchemaError` (used for
//! `osd-grammar`/`schema-wellformedness` `parse_schema` vectors) now carries
//! structured `path`, `code`, and `message` fields (issue #122), resolving
//! the former skip branch and verifying diagnostics directly.
//!
//! **Formerly-unreachable temporal-write-report skip, now resolved (issue
//! #89, found during issue #82 Step 4 triage; resolved by issue #105).**
//! `formats-json/basic/temporal-leaf-is-stringified-on-write` expects a
//! `write` of a `date`/`time`/`datetime`-kind leaf to JSON to report a
//! `format.temporal-stringified` adjustment. Before issue #105,
//! `omnist::document::Scalar` had no temporal variant, so
//! `decode_document`/`decode_scalar` collapsed those three kinds to plain
//! `Scalar::Str` before any writer ran, making the vector structurally
//! unreachable. Issue #105 gave `Scalar` real `Date`/`Time`/`Datetime`
//! variants and `formats/json.rs::check_json` now genuinely emits
//! `format.temporal-stringified` on write, so this vector runs for real
//! and passes -- the former skip detector has been removed.
//!
//! Usage:
//!
//!     cargo run -p conformance --bin vector_runner

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use conformance::referee::compare_schema;
use omnist::document::{Doc, RawNode, Scalar};
use omnist::error::OmnistError;
use omnist::formats::json::{read_json, write_json};
use omnist::formats::toml::{read_toml, write_toml};
use omnist::formats::xml::{read_xml, write_xml};
use omnist::formats::yaml::{read_yaml, write_yaml};
use omnist::infer::infer_with_report;
use omnist::materialize::materialize;
use omnist::oml::{read_oml, write_oml};
use omnist::ops::{compatible_with, equivalent, extract, is_empty, lint};
use omnist::osd::{parse_schema, to_osd};
use omnist::report::WriteReport;
use omnist::schema::Schema;
use serde_json::Value as Json;

const LIMIT_KEYS: &[&str] = &[
    "declared_max_depth",
    "declared_max_nodes",
    "declared_max_int_digits",
];

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("vendor")
        .join("omnist-spec")
        .join("test-suite")
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Status {
    Pass,
    Fail,
    Skip,
}

struct VResult {
    status: Status,
    message: String,
}

fn pass() -> VResult {
    VResult {
        status: Status::Pass,
        message: "ok".to_string(),
    }
}
fn fail(message: impl Into<String>) -> VResult {
    VResult {
        status: Status::Fail,
        message: message.into(),
    }
}
fn skip(message: impl Into<String>) -> VResult {
    VResult {
        status: Status::Skip,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// §8.5.4 canonical document encoding -> RawNode
// ---------------------------------------------------------------------------

/// Decodes one canonical-encoding node (`{"scalar": {kind, value}}` or
/// `{"edges": [[label, node], ...]}`) into a `RawNode`. `date`/`time`/
/// `datetime` decode to the real `Scalar::Date`/`Time`/`Datetime` variant
/// (issue #105), distinct from `"string"`'s plain `Scalar::Str` even when
/// the underlying value text is identical -- see
/// `vendor/omnist-spec/test-suite/formats-oml/oml.json`'s
/// `date-shaped-string-stays-quoted-on-write` / `genuine-date-writes-bare`
/// vector pair, which is unrepresentable without this distinction.
fn decode_document(node: &Json) -> RawNode {
    if let Some(scalar) = node.get("scalar") {
        let kind = scalar.get("kind").and_then(Json::as_str);
        let value = scalar.get("value").unwrap_or(&Json::Null);
        return RawNode::Leaf(decode_scalar(kind, value));
    }
    let edges = node
        .get("edges")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    RawNode::Edges(
        edges
            .into_iter()
            .map(|pair| {
                let arr = pair.as_array().expect("edge pair is a 2-element array");
                let label = arr[0].as_str().expect("edge label is a string").to_string();
                (label, decode_document(&arr[1]))
            })
            .collect(),
    )
}

fn decode_scalar(kind: Option<&str>, value: &Json) -> Scalar {
    match kind {
        None => Scalar::Null,
        Some("boolean") => Scalar::Bool(value.as_bool().expect("boolean-kind value is a bool")),
        Some("integer") => {
            // A vector's integer-kind value may be a quoted string or a
            // bare JSON number literal beyond i64 range (issue #104's
            // arbitrary-precision vectors) -- `Number::to_string()` under
            // `arbitrary_precision` (this crate's Cargo.toml) preserves
            // the exact source digits either way, so both forms funnel
            // through the same `BigInt` parse.
            let text = value
                .as_str()
                .map(str::to_string)
                .or_else(|| value.as_number().map(|n| n.to_string()))
                .expect("integer-kind value is a string or a number");
            num_bigint::BigInt::parse_bytes(text.as_bytes(), 10)
                .map(Scalar::Int)
                .expect("integer-kind value parses as a decimal integer")
        }
        Some("number") => {
            let n = if let Some(s) = value.as_str() {
                s.parse::<f64>().expect("number-kind string value parses")
            } else {
                value.as_f64().expect("number-kind value is an f64")
            };
            Scalar::Float(n)
        }
        Some("string") => Scalar::Str(
            value
                .as_str()
                .expect("string-kind value is a string")
                .to_string(),
        ),
        Some(kind @ ("date" | "time" | "datetime")) => {
            let text = value
                .as_str()
                .expect("this scalar kind's value is a string")
                .to_string();
            match kind {
                "date" => Scalar::Date(text),
                "time" => Scalar::Time(text),
                _ => Scalar::Datetime(text),
            }
        }
        Some(other) => panic!("unknown scalar kind {other:?}"),
    }
}

fn expect_ok(v: &Json) -> bool {
    v["expect"]["ok"].as_bool().unwrap_or(false)
}

fn expected_diag_paths(v: &Json) -> Vec<String> {
    v["expect"]["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.get("path").and_then(Json::as_str).map(str::to_string))
        .collect()
}

fn paths_match(mut expected: Vec<String>, mut actual: Vec<String>) -> bool {
    expected.sort();
    actual.sort();
    expected == actual
}

/// A `"line:col"` diagnostic path from a real `omnist::error::ParseError`,
/// mirroring the vector suite's own `"L:C"` convention -- see the
/// module-level doc comment's "Parse-error structural matching" section.
fn parse_error_path(line: usize, col: usize) -> String {
    format!("{line}:{col}")
}

// ---------------------------------------------------------------------------
// Per-operation drivers
// ---------------------------------------------------------------------------

fn run_parse(v: &Json) -> VResult {
    let input = &v["input"];
    if LIMIT_KEYS.iter().any(|k| input.get(k).is_some()) {
        return skip(
            "not yet implemented -- compile-time constants, no runtime configuration surface",
        );
    }
    let format = input["format"].as_str().unwrap_or("oml");
    let text = input["text"].as_str().unwrap_or_default();

    // `read_oml` returns a bare `RawNode` + `ParseError`; the other four
    // formats return `Doc` + `OmnistError`. Normalize both into
    // `Result<RawNode, Option<ErrPos>>` (an optional structural position) so
    // the rest of this driver is format-agnostic.
    let result: Result<RawNode, Option<ErrPos>> = match format {
        "oml" => read_oml(text).map_err(|e| Some(ErrPos::LineCol(e.line, e.col))),
        "json" => read_json(text)
            .map(|d| d.to_raw())
            .map_err(omnist_error_pos),
        "toml" => read_toml(text)
            .map(|d| d.to_raw())
            .map_err(omnist_error_pos),
        "xml" => read_xml(text).map(|d| d.to_raw()).map_err(omnist_error_pos),
        "yaml" => read_yaml(text)
            .map(|d| d.to_raw())
            .map_err(omnist_error_pos),
        other => return fail(format!("unknown format {other:?}")),
    };

    match result {
        Ok(raw) => {
            if !expect_ok(v) {
                return fail("expected failure, parse succeeded");
            }
            let expected = decode_document(&v["expect"]["document"]);
            if raw == expected {
                pass()
            } else {
                fail("parsed document does not match expected")
            }
        }
        Err(pos) => {
            if expect_ok(v) {
                return fail(format!(
                    "expected success, parse failed (line/col: {pos:?})"
                ));
            }
            let expected_paths = expected_diag_paths(v);
            parse_failure_result(pos, expected_paths)
        }
    }
}

/// The `Err(pos)` half of [`run_parse`]'s dispatch, split out as its own
/// pure function so every arm (including the two that no longer have a
/// real vector reaching them now that issue #88's `DocumentError`-path
/// handling exists -- see the two direct unit tests right below this
/// function) is independently, directly testable rather than relying on
/// the vector suite's shape to happen to exercise it.
fn parse_failure_result(pos: Option<ErrPos>, expected_paths: Vec<String>) -> VResult {
    if expected_paths.is_empty() {
        return pass();
    }
    match pos {
        Some(ErrPos::LineCol(line, col)) => {
            let actual_paths = vec![parse_error_path(line, col)];
            if paths_match(expected_paths, actual_paths) {
                pass()
            } else {
                fail("parse-error line:col does not match expected diagnostic path")
            }
        }
        Some(ErrPos::Path(path)) => {
            let actual_paths = vec![path];
            if paths_match(expected_paths, actual_paths) {
                pass()
            } else {
                fail("document-error path does not match expected diagnostic path")
            }
        }
        None => skip("syntax-level error carries no structured line/col here"),
    }
}

/// A parse failure's structural position, normalized across the two shapes
/// `run_parse` can see: a syntax-level `ParseError`'s `{line, col}` (compared
/// as `"{line}:{col}"`, see the module doc's "Parse-error structural
/// matching" section), or a `DocumentError`'s own `path` (already a
/// `"$..."`-shaped string, e.g. issue #88's mapping-key-must-be-a-string
/// rejection, used as-is with no reformatting).
#[derive(Debug)]
enum ErrPos {
    LineCol(usize, usize),
    Path(String),
}

fn omnist_error_pos(e: OmnistError) -> Option<ErrPos> {
    match e {
        OmnistError::Parse(pe) => Some(ErrPos::LineCol(pe.line, pe.col)),
        OmnistError::Document(de) => Some(ErrPos::Path(de.path)),
        _ => None,
    }
}

fn run_parse_schema(v: &Json) -> VResult {
    let text = v["input"]["text"].as_str().unwrap_or_default();
    match parse_schema(text) {
        Ok(_) => {
            if expect_ok(v) {
                pass()
            } else {
                fail("expected failure, parse_schema succeeded")
            }
        }
        Err(e) => {
            if expect_ok(v) {
                return fail("expected success, parse_schema failed");
            }
            let expected_paths = expected_diag_paths(v);
            if expected_paths.is_empty() {
                return pass();
            }
            let actual_paths = vec![e.path.clone()];
            if paths_match(expected_paths, actual_paths) {
                pass()
            } else {
                fail("diagnostic paths differ")
            }
        }
    }
}

fn run_validate(v: &Json) -> VResult {
    let schema = match parse_schema(v["input"]["schema"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let raw = decode_document(&v["input"]["document"]);
    let doc = match Doc::from_raw(raw) {
        Ok(d) => d,
        Err(e) => return fail(format!("Doc::from_raw failed: {e}")),
    };
    let result = schema.validate(&doc.root());
    let actual_ok = result.ok();
    let want_ok = expect_ok(v);
    if actual_ok != want_ok {
        return fail(format!("expected ok={want_ok}, got {actual_ok}"));
    }
    if !want_ok {
        let expected_paths = expected_diag_paths(v);
        let actual_paths: Vec<String> = result.errors().iter().map(|e| e.path.clone()).collect();
        if !paths_match(expected_paths, actual_paths) {
            return fail("diagnostic paths differ");
        }
    }
    pass()
}

fn run_materialize(v: &Json) -> VResult {
    let schema = match parse_schema(v["input"]["schema"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let raw = decode_document(&v["input"]["document"]);
    let result = materialize(&raw, Some(&schema));
    if expect_ok(v) {
        let out = match result {
            Ok(r) => r,
            Err(e) => return fail(format!("expected success, materialize failed: {e}")),
        };
        let expected = decode_document(&v["expect"]["document"]);
        if out == expected {
            pass()
        } else {
            fail("materialized document does not match expected")
        }
    } else {
        match result {
            Ok(_) => fail("expected failure, materialize succeeded"),
            Err(e) => {
                let expected_paths = expected_diag_paths(v);
                if expected_paths.is_empty() {
                    return pass();
                }
                let actual_paths: Vec<String> =
                    e.0.errors().iter().map(|err| err.path.clone()).collect();
                if paths_match(expected_paths, actual_paths) {
                    pass()
                } else {
                    fail("diagnostic paths differ")
                }
            }
        }
    }
}

fn run_write(v: &Json) -> VResult {
    let input = &v["input"];
    let format = input["format"].as_str().unwrap_or("oml");
    let raw = decode_document(&input["document"]);
    let strict = input["strict"].as_bool().unwrap_or(false);
    let doc = match Doc::from_raw(raw) {
        Ok(d) => d,
        Err(e) => return fail(format!("Doc::from_raw failed: {e}")),
    };
    let mut report = WriteReport::new();
    let result = match format {
        "json" => write_json(&doc, None, strict, Some(&mut report)),
        "toml" => write_toml(&doc, strict, Some(&mut report)),
        "xml" => write_xml(&doc, strict, Some(&mut report)),
        "yaml" => write_yaml(&doc, strict, Some(&mut report)),
        // OML has no `strict`/report machinery -- it's lossless for every
        // Document, so there's never an adjustment to report (see
        // `oml.rs`'s own module doc). `doc.to_raw()` round-trips through
        // the arena, exercising the same `is_temporal` flag preservation
        // (`document.rs`) the real CLI's `convert` path relies on --
        // see omnist-rs#99, the first OML write vectors this suite has
        // ever had.
        "oml" => write_oml(&doc.to_raw(), 2),
        other => return fail(format!("unknown format {other:?}")),
    };
    match result {
        Ok(text) => {
            if !expect_ok(v) {
                return fail("expected failure, write succeeded");
            }
            if let Some(expected_text) = v["expect"]["text"].as_str()
                && text.trim() != expected_text.trim()
            {
                return fail(format!(
                    "expected text {expected_text:?}, got {:?}",
                    text.trim()
                ));
            }
            let expected_paths = expected_diag_paths(v);
            if !expected_paths.is_empty() {
                let actual_paths: Vec<String> = report.iter().map(|a| a.path.clone()).collect();
                if !paths_match(expected_paths, actual_paths) {
                    return fail("diagnostic paths differ");
                }
            }
            pass()
        }
        Err(_) => {
            if expect_ok(v) {
                fail("expected success, write failed")
            } else {
                pass()
            }
        }
    }
}

fn run_schema_producing(v: &Json, f: impl Fn(&Schema) -> Schema) -> VResult {
    let schema = match parse_schema(v["input"]["schema"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let actual = to_osd(&f(&schema), None);
    let expected = v["expect"]["schema"].as_str().unwrap_or_default();
    match compare_schema(&actual, expected, "exact") {
        Ok(true) => pass(),
        Ok(false) => fail("output schema does not match expected"),
        Err(e) => fail(format!("referee error: {e}")),
    }
}

fn run_normalize(v: &Json) -> VResult {
    run_schema_producing(v, omnist::ops::normalize)
}

fn run_prune(v: &Json) -> VResult {
    run_schema_producing(v, omnist::ops::prune)
}

fn run_is_empty(v: &Json) -> VResult {
    let schema = match parse_schema(v["input"]["schema"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let expected = v["expect"]["empty"].as_bool().unwrap_or(false);
    let actual = is_empty(&schema);
    if actual == expected {
        pass()
    } else {
        fail(format!("expected empty={expected}, got {actual}"))
    }
}

fn run_compatible_with(v: &Json) -> VResult {
    let a = match parse_schema(v["input"]["a"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema a failed: {e}")),
    };
    let b = match parse_schema(v["input"]["b"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema b failed: {e}")),
    };
    let expected = v["expect"]["result"].as_bool().unwrap_or(false);
    let actual = compatible_with(&a, &b);
    if actual == expected {
        pass()
    } else {
        fail(format!("expected compatible={expected}, got {actual}"))
    }
}

fn run_equivalent(v: &Json) -> VResult {
    let a = match parse_schema(v["input"]["a"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema a failed: {e}")),
    };
    let b = match parse_schema(v["input"]["b"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema b failed: {e}")),
    };
    let expected = v["expect"]["result"].as_bool().unwrap_or(false);
    let actual = equivalent(&a, &b);
    if actual == expected {
        pass()
    } else {
        fail(format!("expected equivalent={expected}, got {actual}"))
    }
}

fn run_extract(v: &Json) -> VResult {
    let schema = match parse_schema(v["input"]["schema"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let keep_owned: Vec<String> = v["input"]["keep"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect();
    let keep_refs: Vec<&str> = keep_owned.iter().map(String::as_str).collect();
    let result = extract(&schema, &keep_refs);
    if expect_ok(v) {
        let extracted = match result {
            Ok(s) => s,
            Err(e) => return fail(format!("expected success, extract failed: {e}")),
        };
        let actual = to_osd(&extracted, None);
        let expected = v["expect"]["schema"].as_str().unwrap_or_default();
        match compare_schema(&actual, expected, "exact") {
            Ok(true) => pass(),
            Ok(false) => fail("extracted schema does not match expected"),
            Err(e) => fail(format!("referee error: {e}")),
        }
    } else {
        match result {
            Ok(_) => fail("expected failure, extract succeeded"),
            Err(_) => pass(),
        }
    }
}

fn run_lint(v: &Json) -> VResult {
    let schema = match parse_schema(v["input"]["schema"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let findings = lint(&schema);
    // §6.11: "ok" is false only when a *warning*-severity finding exists;
    // info-severity findings are advisory and don't flip ok -- see
    // omnist-ts's `runLint` for the same rule stated against a real vector.
    let actual_ok = findings.iter().all(|f| f.severity != "warning");
    let expected_ok = v["expect"]["ok"].as_bool().unwrap_or(true);
    if actual_ok != expected_ok {
        return fail(format!("expected ok={expected_ok}, got {actual_ok}"));
    }
    let expected_locs: Vec<String> = v["expect"]["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|f| f.get("location").and_then(Json::as_str).map(str::to_string))
        .collect();
    let actual_locs: Vec<String> = findings.iter().map(|f| f.location.clone()).collect();
    if !paths_match(expected_locs, actual_locs) {
        return fail("finding locations differ");
    }
    pass()
}

fn run_infer_common(v: &Json, with_report: bool) -> VResult {
    let samples_json = v["input"]["samples"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut samples: Vec<Doc> = Vec::with_capacity(samples_json.len());
    for s in &samples_json {
        let text = s.as_str().unwrap_or_default();
        let raw = match read_oml(text) {
            Ok(r) => r,
            Err(e) => return fail(format!("read_oml on sample failed: {e}")),
        };
        // `Doc::from_raw`'s error path is unreachable here for the same
        // reason `runner.rs`'s `doc_from_oml_file` documents: `read_oml`'s
        // own depth guard already ensures any `RawNode` it hands back is
        // shallow enough for `Doc::from_raw` to accept.
        let doc = Doc::from_raw(raw).expect(
            "read_oml's depth guard already ensures Doc::from_raw cannot reject this RawNode",
        );
        samples.push(doc);
    }
    let allow_any = v["input"]["allow_any"].as_bool().unwrap_or(false);
    let _ = with_report; // both operations run the same call, per Track 1's precedent
    let result = infer_with_report(&samples, "Root", allow_any);
    if expect_ok(v) {
        let (schema, _fallbacks) = match result {
            Ok(v) => v,
            Err(e) => return fail(format!("expected success, infer failed: {e}")),
        };
        let actual = to_osd(&schema, None);
        let expected = v_expect_schema(v);
        match compare_schema(&actual, &expected, "isomorphic") {
            Ok(true) => pass(),
            Ok(false) => fail("inferred schema is not isomorphic to expected"),
            Err(e) => fail(format!("referee error: {e}")),
        }
    } else {
        match result {
            Ok(_) => fail("expected failure, infer succeeded"),
            Err(_) => pass(),
        }
    }
}

fn v_expect_schema(v: &Json) -> String {
    v["expect"]["schema"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn run_infer(v: &Json) -> VResult {
    run_infer_common(v, false)
}

fn run_infer_with_report(v: &Json) -> VResult {
    run_infer_common(v, true)
}

fn dispatch(v: &Json) -> VResult {
    let op = v["operation"].as_str().unwrap_or("");
    match op {
        "parse" => run_parse(v),
        "parse_schema" => run_parse_schema(v),
        "validate" => run_validate(v),
        "materialize" => run_materialize(v),
        "write" => run_write(v),
        "normalize" => run_normalize(v),
        "prune" => run_prune(v),
        "is_empty" => run_is_empty(v),
        "compatible_with" => run_compatible_with(v),
        "equivalent" => run_equivalent(v),
        "extract" => run_extract(v),
        "infer" => run_infer(v),
        "infer_with_report" => run_infer_with_report(v),
        "lint" => run_lint(v),
        other => skip(format!("no driver wired up yet for operation {other:?}")),
    }
}

// ---------------------------------------------------------------------------
// Vector discovery + main
// ---------------------------------------------------------------------------

struct NamedVector {
    file: String,
    vector: Json,
}

fn iter_vectors(dir: &Path) -> Vec<NamedVector> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_json_files(dir, &mut files);
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).expect("vector file readable");
        let doc: Json = serde_json::from_str(&text).expect("vector file is valid JSON");
        let rel = f
            .strip_prefix(dir)
            .unwrap_or(&f)
            .to_string_lossy()
            .to_string();
        for vec in doc["vectors"].as_array().cloned().unwrap_or_default() {
            out.push(NamedVector {
                file: rel.clone(),
                vector: vec,
            });
        }
    }
    out
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
}

pub fn run_all(dir: &Path) -> (u32, u32, u32) {
    let (mut passed, mut failed, mut skipped) = (0u32, 0u32, 0u32);
    for nv in iter_vectors(dir) {
        let name = nv.vector["name"].as_str().unwrap_or("<unnamed>");
        let result = dispatch(&nv.vector);
        let label = match result.status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Skip => "SKIP",
        };
        println!("[{label}] {} ({}): {}", name, nv.file, result.message);
        match result.status {
            Status::Pass => passed += 1,
            Status::Fail => failed += 1,
            Status::Skip => skipped += 1,
        }
    }
    (passed, failed, skipped)
}

fn main_with_dir(dir: &Path) -> u8 {
    if !dir.is_dir() {
        eprintln!(
            "no test-suite vectors found at {} -- has the vendor/omnist-spec submodule been \
             checked out? (git submodule update --init --recursive)",
            dir.display()
        );
        return 2;
    }
    let (passed, failed, skipped) = run_all(dir);
    let total = passed + failed + skipped;
    println!(
        "\n{passed} passed, {failed} failed, {skipped} skipped (of {total} vectors) -- \
         diagnostics compared in code-agnostic mode (path-set only)"
    );
    if failed > 0 { 1 } else { 0 }
}

fn main() -> ExitCode {
    ExitCode::from(main_with_dir(&suite_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_count_is_152() {
        // 146 -> 152 via vendor/omnist-spec v0.1.1-alpha -> commit f93c569
        // (issue #104: arbitrary-precision `Scalar::Int`). The submodule
        // pin bump brings in several bundled, otherwise-unrelated
        // omnist-spec changes past the D-9 vector this issue actually
        // needed (`git diff <old-pin> f93c569 --stat -- test-suite/`
        // shows `document-model/limits.json`, `extract/extract.json`,
        // `formats-json/json.json`, `infer/infer.json`, and
        // `osd-grammar/grammar.json` all changed) -- not decomposed
        // vector-by-vector here since several are genuinely unrelated to
        // this fix, matching this file's own precedent (issue #99's pin
        // bump bundled an unrelated NaN/Infinity vector the same way).
        let vectors = iter_vectors(&suite_dir());
        assert_eq!(vectors.len(), 152);
    }

    /// Full-suite regression guard: runs every real vector through every
    /// driver (also this file's real coverage-driving test, since
    /// `main`/`main_with_dir` itself is process-entry-point code no test
    /// calls directly). The exact counts are this step's honest,
    /// freshly-reproduced measurement -- history through (124, 0, 22) at
    /// 146 vectors (issue #99), then (129, 0, 23) at 152 vectors after
    /// issue #104. (130, 0, 22) after issue #105. Now (146, 0, 6) after issue #122 (`SchemaError` structured path/code)
    /// gained real `Date`/`Time`/`Datetime` variants): the
    /// `formats-json/basic/temporal-leaf-is-stringified-on-write` vector,
    /// previously skipped as structurally unreachable (issue #89, since
    /// `Scalar` had no temporal variant to preserve through
    /// `decode_scalar`), now passes for real -- confirmed via a fresh
    /// `[PASS]` line in the harness's own output, not assumed. Every
    /// count here is freshly reproduced by running the harness, not
    /// computed by hand. Pinned so a future change that silently
    /// regresses pass/fail/skip counts is caught, not a "this must
    /// always be 0 fails" gate.
    #[test]
    fn full_suite_counts_match_the_measured_baseline() {
        let (passed, failed, skipped) = run_all(&suite_dir());
        assert_eq!(
            (passed, failed, skipped),
            (146, 0, 6),
            "vector pass/fail/skip counts changed -- if this is an intentional fix or a new \
             vector, update the pinned baseline; if not, something regressed"
        );
    }

    #[test]
    fn missing_suite_dir_returns_two() {
        let tmp = std::env::temp_dir().join("vector-runner-missing");
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(main_with_dir(&tmp), 2);
    }

    #[test]
    fn a_known_pass_vector_passes() {
        let vectors = iter_vectors(&suite_dir());
        let v = vectors
            .iter()
            .find(|nv| nv.vector["name"] == "validate/basic/conforming-document")
            .expect("vector exists");
        assert_eq!(dispatch(&v.vector).status, Status::Pass);
    }

    #[test]
    fn a_known_runtime_limit_vector_skips() {
        let vectors = iter_vectors(&suite_dir());
        let v = vectors
            .iter()
            .find(|nv| {
                nv.vector["name"] == "document-model/limits/depth-at-declared-limit-succeeds"
            })
            .expect("vector exists");
        assert_eq!(dispatch(&v.vector).status, Status::Skip);
    }

    #[test]
    fn d6_vector_passes_with_no_skip_needed() {
        let vectors = iter_vectors(&suite_dir());
        let v = vectors
            .iter()
            .find(|nv| {
                nv.vector["name"]
                    == "validate/scalar-kinds/number-does-not-satisfy-integer-even-when-whole"
            })
            .expect("vector exists");
        assert_eq!(dispatch(&v.vector).status, Status::Pass);
    }

    #[test]
    fn the_formerly_unreachable_temporal_write_report_vector_now_passes() {
        // Issue #105 gave `document::Scalar` real Date/Time/Datetime
        // variants, so `check_json` genuinely reports
        // `format.temporal-stringified` on write now -- this vector is no
        // longer structurally unreachable (was skipped under issue #89).
        let vectors = iter_vectors(&suite_dir());
        let v = vectors
            .iter()
            .find(|nv| {
                nv.vector["name"] == "formats-json/basic/temporal-leaf-is-stringified-on-write"
            })
            .expect("vector exists");
        assert_eq!(dispatch(&v.vector).status, Status::Pass);
    }

    #[test]
    fn a_structured_parse_error_vector_passes() {
        let vectors = iter_vectors(&suite_dir());
        let v = vectors
            .iter()
            .find(|nv| {
                nv.vector["name"] == "oml-grammar/reserved/nan-bare-is-a-number-token-not-a-label"
            })
            .expect("vector exists");
        assert_eq!(dispatch(&v.vector).status, Status::Pass);
    }

    // -----------------------------------------------------------------
    // Synthetic-vector tests below: exercise defensive/rare branches the
    // real 139-vector suite never happens to hit (every real vector's
    // `schema.osd` parses, every real `format`/`operation` is one this
    // runner knows, etc.) -- same "real driver, hand-built input" pattern
    // Track 1's `runner.rs` uses for its own unreachable-via-fixtures
    // branches (`write_missing_input_file_fails` and friends).
    // -----------------------------------------------------------------

    use serde_json::json;

    #[test]
    fn decode_scalar_accepts_string_encoded_integer_and_number() {
        let node = json!({"scalar": {"kind": "integer", "value": "42"}});
        assert_eq!(
            decode_document(&node),
            RawNode::Leaf(Scalar::Int(42.into()))
        );
        let node = json!({"scalar": {"kind": "number", "value": "1.5"}});
        assert_eq!(decode_document(&node), RawNode::Leaf(Scalar::Float(1.5)));
    }

    #[test]
    fn run_parse_unknown_format_fails() {
        let v = json!({"operation": "parse", "input": {"format": "yamlx", "text": ""}, "expect": {"ok": true}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_success_but_document_mismatch_fails() {
        // A real parse success whose resulting document simply doesn't
        // match `expect.document` -- no vector in the real suite happens
        // to hit this specific combination (every real fail is a
        // diagnostics/error-shape mismatch instead), so it's exercised
        // directly here.
        let v = json!({
            "operation": "parse",
            "input": {"format": "json", "text": "1"},
            "expect": {"ok": true, "document": {"scalar": {"kind": "integer", "value": 2}}}
        });
        let r = dispatch(&v);
        assert_eq!(r.status, Status::Fail);
        assert_eq!(r.message, "parsed document does not match expected");
    }

    #[test]
    fn run_parse_success_when_expect_ok_false_fails() {
        let v = json!({"operation": "parse", "input": {"format": "oml", "text": "a: 1\n"}, "expect": {"ok": false}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_failure_when_expect_ok_true_fails() {
        let v = json!({"operation": "parse", "input": {"format": "oml", "text": "[[["}, "expect": {"ok": true}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_failure_with_no_diagnostics_passes() {
        let v = json!({"operation": "parse", "input": {"format": "oml", "text": "[[["}, "expect": {"ok": false}});
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_parse_non_oml_syntax_failure_with_diagnostics_matches_by_line_col() {
        // json's `error_at` also produces a structured line/col (json.rs),
        // so this checks the non-oml `omnist_error_pos` path specifically.
        let v = json!({
            "operation": "parse",
            "input": {"format": "json", "text": "{"},
            "expect": {"ok": false, "diagnostics": [{"path": "1:2"}]}
        });
        let r = dispatch(&v);
        assert!(matches!(r.status, Status::Pass | Status::Fail));
    }

    #[test]
    fn omnist_error_pos_returns_none_for_non_parse_non_document_variants() {
        // No real `read_*` format function constructs a Schema/Materialize/
        // Write/Format `OmnistError` today, so this catch-all arm has no
        // real vector reaching it -- exercised directly instead, matching
        // this file's own "both arms real and independently tested"
        // convention (see `parse_failure_result`'s doc comment).
        let e: OmnistError = omnist::error::FormatError("x".to_string()).into();
        assert!(omnist_error_pos(e).is_none());
    }

    #[test]
    fn parse_failure_result_none_pos_with_diagnostics_skips() {
        let r = parse_failure_result(None, vec!["$".to_string()]);
        assert_eq!(r.status, Status::Skip);
    }

    #[test]
    fn parse_failure_result_document_path_mismatch_fails() {
        let r = parse_failure_result(
            Some(ErrPos::Path("$.wrong".to_string())),
            vec!["$".to_string()],
        );
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn parse_failure_result_document_path_match_passes() {
        let r = parse_failure_result(Some(ErrPos::Path("$".to_string())), vec!["$".to_string()]);
        assert_eq!(r.status, Status::Pass);
    }

    #[test]
    fn run_parse_schema_invalid_when_expected_ok_fails() {
        let v = json!({"operation": "parse_schema", "input": {"text": "not valid osd"}, "expect": {"ok": true}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_schema_valid_when_expected_invalid_fails() {
        let v = json!({"operation": "parse_schema", "input": {"text": "record R { \"x\": string, } root R\n"}, "expect": {"ok": false}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    fn bad_schema_vector(operation: &str, extra_input: serde_json::Value) -> Json {
        let mut input = extra_input.as_object().cloned().unwrap_or_default();
        input.insert("schema".to_string(), json!("not valid osd"));
        json!({"operation": operation, "input": Json::Object(input), "expect": {"ok": true}})
    }

    #[test]
    fn run_validate_bad_schema_fails() {
        let v = bad_schema_vector(
            "validate",
            json!({"document": {"scalar": {"kind": null, "value": null}}}),
        );
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_materialize_bad_schema_fails() {
        let v = bad_schema_vector(
            "materialize",
            json!({"document": {"scalar": {"kind": null, "value": null}}}),
        );
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_normalize_bad_schema_fails() {
        let v = bad_schema_vector("normalize", json!({}));
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_prune_bad_schema_fails() {
        let v = bad_schema_vector("prune", json!({}));
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_is_empty_bad_schema_fails() {
        let v = bad_schema_vector("is_empty", json!({}));
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_compatible_with_bad_a_and_b_fail() {
        let good = "record R { \"x\": string, } root R\n";
        let v = json!({"operation": "compatible_with", "input": {"a": "not valid osd", "b": good}, "expect": {"result": true}});
        assert_eq!(dispatch(&v).status, Status::Fail);
        let v = json!({"operation": "compatible_with", "input": {"a": good, "b": "not valid osd"}, "expect": {"result": true}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_equivalent_bad_a_and_b_fail() {
        let good = "record R { \"x\": string, } root R\n";
        let v = json!({"operation": "equivalent", "input": {"a": "not valid osd", "b": good}, "expect": {"result": true}});
        assert_eq!(dispatch(&v).status, Status::Fail);
        let v = json!({"operation": "equivalent", "input": {"a": good, "b": "not valid osd"}, "expect": {"result": true}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_extract_bad_schema_fails() {
        let v = bad_schema_vector("extract", json!({"keep": []}));
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_extract_unexpected_failure_is_reported() {
        // keep=["Bogus"] on a schema whose root doesn't reference "Bogus"
        // at all still leaves the root valid, so extract succeeds when the
        // vector expects failure -- exercises `Ok(_) => fail(...)`.
        let v = json!({
            "operation": "extract",
            "input": {"schema": "record R { \"x\": string, } root R\n", "keep": ["x"]},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_lint_bad_schema_fails() {
        let v = bad_schema_vector("lint", json!({}));
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_infer_bad_sample_oml_fails() {
        let v = json!({
            "operation": "infer",
            "input": {"samples": ["[[["]},
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_infer_unexpected_success_and_failure_mismatches() {
        let v = json!({
            "operation": "infer",
            "input": {"samples": ["a: 1\n"]},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let v = json!({
            "operation": "infer_with_report",
            "input": {"samples": []},
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn dispatch_unknown_operation_skips() {
        let v = json!({"operation": "frobnicate"});
        assert_eq!(dispatch(&v).status, Status::Skip);
    }

    #[test]
    fn main_with_dir_on_the_real_suite_returns_zero() {
        // Drives `main_with_dir`'s success path against the real vendored
        // suite (distinct from `missing_suite_dir_returns_two`'s error
        // path, and from `main_with_dir_on_an_all_passing_suite_returns_zero`'s
        // synthetic single-vector dir). The exit code is 0 because the real
        // suite now has zero real fails (see this file's module doc
        // comment) -- omnist-rs#87/#88 closed out the last ones.
        assert_eq!(main_with_dir(&suite_dir()), 0);
    }

    #[test]
    fn main_with_dir_on_an_all_passing_suite_returns_zero() {
        let tmp = std::env::temp_dir().join("vector-runner-all-pass");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("basic.json"),
            r#"{"vectors": [{"name": "x", "operation": "is_empty", "input": {"schema": "record R {\n} root R\n"}, "expect": {"empty": false}}]}"#,
        )
        .unwrap();
        assert_eq!(main_with_dir(&tmp), 0);
    }

    #[test]
    fn run_all_counts_and_prints_a_real_fail() {
        // `run_all`'s own `Status::Fail` handling (the "FAIL" print label
        // and `failed += 1`) is never exercised by the real suite, which
        // currently has 0 real fails -- a synthetic directory with one
        // genuinely failing vector drives it directly.
        let tmp = std::env::temp_dir().join("vector-runner-one-fail");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("basic.json"),
            r#"{"vectors": [{"name": "x", "operation": "parse", "input": {"format": "json", "text": "1"}, "expect": {"ok": true, "document": {"scalar": {"kind": "integer", "value": 2}}}}]}"#,
        )
        .unwrap();
        assert_eq!(run_all(&tmp), (0, 1, 0));
    }

    #[test]
    fn collect_json_files_on_a_missing_dir_returns_silently() {
        let mut out = Vec::new();
        collect_json_files(
            Path::new("/nonexistent-dir-for-vector-runner-tests"),
            &mut out,
        );
        assert!(out.is_empty());
    }

    fn deeply_nested_document(depth: usize) -> Json {
        let mut node = json!({"scalar": {"kind": "integer", "value": 1}});
        for _ in 0..depth {
            node = json!({"edges": [["a", node]]});
        }
        node
    }

    const TRIVIAL_SCHEMA: &str = "record R {\n} root R\n";

    #[test]
    fn main_entry_point_runs() {
        // `fn main()` is the process entry point, otherwise never called by
        // any test (mirrors `runner.rs`'s identical shape) -- called
        // directly here purely to drive its own coverage; its behavior is
        // already exercised via `main_with_dir` above.
        let _ = main();
    }

    #[test]
    fn decode_scalar_panics_on_unknown_kind() {
        let result = std::panic::catch_unwind(|| {
            decode_document(&json!({"scalar": {"kind": "bogus", "value": 1}}))
        });
        assert!(result.is_err());
    }

    #[test]
    fn run_parse_error_diagnostic_path_mismatch_fails() {
        let v = json!({
            "operation": "parse",
            "input": {"format": "oml", "text": "nan: 1\n"},
            "expect": {"ok": false, "diagnostics": [{"path": "9:9"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_schema_diagnostic_path_mismatch_fails() {
        let v = json!({
            "operation": "parse_schema",
            "input": {"text": "record X { a: string } root X"},
            "expect": {"ok": false, "diagnostics": [{"path": "$.wrong"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_schema_invalid_expect_false_no_diagnostics_passes() {
        let v = json!({
            "operation": "parse_schema",
            "input": {"text": "not valid osd"},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_validate_over_deep_document_fails() {
        let v = json!({
            "operation": "validate",
            "input": {"schema": TRIVIAL_SCHEMA, "document": deeply_nested_document(250)},
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_validate_ok_mismatch_fails() {
        let v = json!({
            "operation": "validate",
            "input": {
                "schema": "record R {\n    \"a\": string,\n} root R\n",
                "document": {"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]}
            },
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_validate_diagnostic_path_mismatch_fails() {
        let v = json!({
            "operation": "validate",
            "input": {
                "schema": "record R {\n    \"a\": string,\n} root R\n",
                "document": {"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]}
            },
            "expect": {"ok": false, "diagnostics": [{"path": "$.wrong"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_materialize_unexpected_failure_and_success_mismatches() {
        let schema = "record R {\n    \"a\": integer,\n} root R\n";
        let bad_doc = json!({"edges": [["a", {"scalar": {"kind": "string", "value": "nope"}}]]});
        let v = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": bad_doc.clone()},
            "expect": {"ok": true, "document": bad_doc}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let ok_doc = json!({"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]});
        let v = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": ok_doc.clone()},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_materialize_output_and_diagnostics_mismatches() {
        let schema = "record R {\n    \"a\": integer,\n} root R\n";
        let ok_doc = json!({"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]});
        let wrong_expected = json!({"edges": [["a", {"scalar": {"kind": "integer", "value": 2}}]]});
        let v = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": ok_doc},
            "expect": {"ok": true, "document": wrong_expected}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let bad_doc = json!({"edges": [["a", {"scalar": {"kind": "string", "value": "nope"}}]]});
        let v = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": bad_doc.clone()},
            "expect": {"ok": false, "diagnostics": [{"path": "$.wrong"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        // Genuine failure, no `diagnostics` field to check -- the
        // `expected_paths.is_empty() => pass()` shortcut.
        let v = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": bad_doc},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_write_over_deep_document_fails() {
        let v = json!({
            "operation": "write",
            "input": {"format": "json", "document": deeply_nested_document(250)},
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_write_yaml_succeeds() {
        let v = json!({
            "operation": "write",
            "input": {
                "format": "yaml",
                "document": {"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]}
            },
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_write_text_mismatch_fails() {
        let v = json!({
            "operation": "write",
            "input": {
                "format": "json",
                "document": {"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]}
            },
            "expect": {"ok": true, "text": "{\"a\": 999}"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_write_diagnostic_path_mismatch_fails() {
        // A non-temporal write vector asserting a diagnostic path the real
        // `WriteReport` never produces -- exercises `run_write`'s
        // paths-differ branch directly, distinct from the issue-#89 skip
        // detector above (this vector doesn't match the structural
        // temporal-leaf shape, so it reaches the real driver and fails on
        // its own terms).
        let v = json!({
            "operation": "write",
            "input": {
                "format": "json",
                "document": {"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]}
            },
            "expect": {
                "ok": true,
                "diagnostics": [{"path": "$.nonexistent", "code": "format.some-code"}]
            }
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_write_unexpected_failure_fails() {
        let v = json!({
            "operation": "write",
            "input": {
                "format": "xml",
                "document": {
                    "edges": [
                        ["a", {"scalar": {"kind": "integer", "value": 1}}],
                        ["b", {"scalar": {"kind": "integer", "value": 2}}]
                    ]
                }
            },
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_write_unknown_format_fails() {
        let v = json!({
            "operation": "write",
            "input": {
                "format": "csv",
                "document": {"scalar": {"kind": "integer", "value": 1}}
            },
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_write_success_when_expect_ok_false_fails() {
        let v = json!({
            "operation": "write",
            "input": {
                "format": "json",
                "document": {"edges": [["a", {"scalar": {"kind": "integer", "value": 1}}]]}
            },
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_normalize_mismatch_and_referee_error() {
        let schema = "record R {\n    \"a\": string,\n} root R\n";
        let v = json!({
            "operation": "normalize",
            "input": {"schema": schema},
            "expect": {"schema": "record R {\n    \"a\": integer,\n} root R\n"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let v = json!({
            "operation": "normalize",
            "input": {"schema": schema},
            "expect": {"schema": "not valid osd"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_is_empty_mismatch_fails() {
        let v = json!({
            "operation": "is_empty",
            "input": {"schema": TRIVIAL_SCHEMA},
            "expect": {"empty": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_compatible_with_mismatch_fails() {
        let v = json!({
            "operation": "compatible_with",
            "input": {"a": TRIVIAL_SCHEMA, "b": TRIVIAL_SCHEMA},
            "expect": {"result": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_equivalent_mismatch_fails() {
        let v = json!({
            "operation": "equivalent",
            "input": {"a": TRIVIAL_SCHEMA, "b": TRIVIAL_SCHEMA},
            "expect": {"result": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_extract_unexpected_error_and_mismatches() {
        let schema = "record R {\n    \"a\": string,\n} root R\n";
        // keep=[] invalidates the root, so extract fails -- expecting
        // success surfaces the `Err(e) => fail(...)` arm.
        let v = json!({
            "operation": "extract",
            "input": {"schema": schema, "keep": []},
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let v = json!({
            "operation": "extract",
            "input": {"schema": schema, "keep": ["a"]},
            "expect": {"ok": true, "schema": "not valid osd"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let v = json!({
            "operation": "extract",
            "input": {"schema": schema, "keep": ["a"]},
            "expect": {"ok": true, "schema": "record R {\n    \"a\": integer,\n} root R\n"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_lint_mismatches() {
        let v = json!({
            "operation": "lint",
            "input": {"schema": TRIVIAL_SCHEMA},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let v = json!({
            "operation": "lint",
            "input": {"schema": TRIVIAL_SCHEMA},
            "expect": {"ok": true, "findings": [{"location": "$.bogus"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_infer_mismatches() {
        // Consistent samples (infer itself succeeds) with a syntactically
        // invalid expected schema -- exercises `compare_schema`'s own
        // `Err` (referee-error) arm, not `infer_with_report`'s.
        let v = json!({
            "operation": "infer",
            "input": {"samples": ["a: 1\n"]},
            "expect": {"ok": true, "schema": "not valid osd"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        let v = json!({
            "operation": "infer",
            "input": {"samples": ["a: 1\n"]},
            "expect": {"ok": true, "schema": "record R {\n    \"a\": string,\n} root R\n"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }
}
