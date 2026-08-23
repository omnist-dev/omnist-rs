//! Track 1: runs vendor/omnist-spec's `conformance/fixtures/<operation>/`
//! (11 operations, ~19 cases) against omnist-rs's own `omnist` library --
//! issue #82 Step 2. omnist-spec's `docs/conformance-harness.md` §3/§4/§7.
//!
//! Ported in spirit from omnist-ts's `tools/conformance/runner.ts` (freshest
//! worked reference, same own-referee/own-runners architecture) and
//! Python's `omnist`'s `tools/conformance/runner.py`. Follows Rust idiom,
//! not either one's syntax (workflow-playbook.md's "architecture freedom"):
//! dispatch is a `match` on the operation name rather than a literal port
//! of TS's lookup-table-of-functions, per issue #82's explicit call-out
//! that this is a reasonable architecture-freedom choice.
//!
//! ## Skip semantics (deviation note, per issue #82's Step 2 instructions)
//!
//! Like the TS reference, Track 1 has no per-case skip -- skip is only a
//! per-*operation* result, and only when no driver is wired for that
//! operation at all. All 11 operations are wired here, so in practice no
//! case is ever skipped; the mechanism is kept (rather than deleted) for
//! architectural parity with the TS/Python precedent and as a safety net
//! if a future omnist-spec revision adds a 12th operation directory this
//! runner doesn't yet know about.
//!
//! Usage:
//!
//!     cargo run -p conformance --bin runner [operation ...]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use conformance::referee::{compare_document, compare_schema};
use omnist::document::{Cursor, Doc, RawNode};
use omnist::infer::infer_with_report;
use omnist::materialize::materialize;
use omnist::oml::{read_oml, write_oml};
use omnist::ops::{compatible_with, equivalent, extract, is_empty, lint, normalize, prune};
use omnist::osd::{parse_schema, to_osd};
use omnist::schema::Schema;

const ALL_OPERATIONS: &[&str] = &[
    "write",
    "validate",
    "materialize",
    "normalize",
    "prune",
    "is_empty",
    "compatible_with",
    "equivalent",
    "extract",
    "infer",
    "lint",
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("vendor")
        .join("omnist-spec")
        .join("conformance")
        .join("fixtures")
}

#[derive(Debug, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
}

struct CaseResult {
    status: Status,
    message: String,
}

fn pass() -> CaseResult {
    CaseResult {
        status: Status::Pass,
        message: "ok".to_string(),
    }
}

fn fail(message: impl Into<String>) -> CaseResult {
    CaseResult {
        status: Status::Fail,
        message: message.into(),
    }
}

fn read(dir: &Path, name: &str) -> std::io::Result<String> {
    std::fs::read_to_string(dir.join(name))
}

fn read_required(dir: &Path, name: &str) -> Result<String, CaseResult> {
    read(dir, name).map_err(|e| fail(format!("missing {name}: {e}")))
}

fn read_trimmed_bool(dir: &Path, name: &str) -> Result<bool, CaseResult> {
    let text = read_required(dir, name)?;
    Ok(text.trim() == "true")
}

fn purpose(case_dir: &Path) -> String {
    read(case_dir, "purpose.txt")
        .ok()
        .and_then(|s| s.lines().next().map(str::to_string))
        .unwrap_or_default()
}

/// Parses `input.oml` into a `Doc` -- the shared first step for `validate`
/// and `infer`, which both need a `Doc` (not just a `RawNode`) to build a
/// `Cursor` or sample list.
///
/// `Doc::from_raw`'s error path is unreachable here: `read_oml`'s own depth
/// guard (see `referee.rs`'s `doc_to_raw_roundtrip_rejects_a_hand_built_over_deep_raw_node`
/// test and doc comment) already guarantees any `RawNode` it hands back is
/// shallow enough for `Doc::from_raw` to accept -- there is no fixture-file
/// route (only a hand-built `RawNode` bypassing the parser entirely) that
/// can reach that branch from here, so it's `.expect()`-documented rather
/// than left as an untested `Err` arm.
fn doc_from_oml_file(dir: &Path, name: &str) -> Result<Doc, CaseResult> {
    let text = read_required(dir, name)?;
    let raw: RawNode = read_oml(&text).map_err(|e| fail(format!("parse {name}: {e}")))?;
    Ok(Doc::from_raw(raw)
        .expect("read_oml's depth guard already ensures Doc::from_raw cannot reject this RawNode"))
}

// ---------------------------------------------------------------------------
// Per-operation drivers
// ---------------------------------------------------------------------------

fn run_write(dir: &Path) -> CaseResult {
    let input = match read_required(dir, "input.oml") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let expected = match read_required(dir, "expected.oml") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let raw = match read_oml(&input) {
        Ok(r) => r,
        Err(e) => return fail(format!("read_oml failed: {e}")),
    };
    // `write_oml` DOES have a producing `Err` path: `oml/writer.rs`'s
    // `write_edges`/`write_edges_compact` call `check_write_depth(...)?`
    // (`document.rs`), which returns `Err` for over-`MAX_DEPTH` input.
    // It's unreachable here specifically because `raw` came from
    // `read_oml` above, whose parser already enforces `MAX_DEPTH` at parse
    // time (`oml/parser.rs`'s `parse_brace_value`) -- the same reasoning
    // `doc_from_oml_file` below documents for its own `.expect()`, not
    // "this function has no Err path at all". `.expect()`-documented
    // rather than an untested `Err` arm with no real trigger.
    let actual = write_oml(&raw, 2)
        .expect("read_oml's depth guard already bounds this RawNode within write_oml's own limit");
    match compare_document(&actual, &expected) {
        Ok(true) => pass(),
        Ok(false) => fail("output does not match expected (structural comparison)"),
        Err(e) => fail(format!("referee error: {e}")),
    }
}

fn run_validate(dir: &Path) -> CaseResult {
    let expect_ok = match read_trimmed_bool(dir, "expected/ok.txt") {
        Ok(b) => b,
        Err(r) => return r,
    };
    let schema_text = match read_required(dir, "schema.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let schema = match parse_schema(&schema_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let doc = match doc_from_oml_file(dir, "input.oml") {
        Ok(d) => d,
        Err(r) => return r,
    };
    let cursor: Cursor<'_> = doc.root();
    let result = schema.validate(&cursor);
    let actual_ok = result.ok();
    if actual_ok == expect_ok {
        pass()
    } else {
        fail(format!("expected ok={expect_ok}, got {actual_ok}"))
    }
}

fn run_materialize(dir: &Path) -> CaseResult {
    let expect_ok = match read_trimmed_bool(dir, "expected/ok.txt") {
        Ok(b) => b,
        Err(r) => return r,
    };
    let schema_text = match read_required(dir, "schema.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let schema = match parse_schema(&schema_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let input_text = match read_required(dir, "input.oml") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let input_raw = match read_oml(&input_text) {
        Ok(r) => r,
        Err(e) => return fail(format!("read_oml failed: {e}")),
    };
    let result = materialize(&input_raw, Some(&schema));
    if expect_ok {
        let out_raw = match result {
            Ok(r) => r,
            Err(e) => return fail(format!("expected success, materialize failed: {e}")),
        };
        // Same reasoning as run_write's `write_oml` call: `write_oml` does
        // have a producing `Err` path (`check_write_depth`), but it's
        // unreachable here since `materialize` only upgrades scalar
        // values in place -- it never adds nesting -- so `out_raw`'s
        // depth is still bounded by `input_raw`'s, which `read_oml`'s
        // parser already enforced within `MAX_DEPTH` at parse time.
        let actual = write_oml(&out_raw, 2)
            .expect("materialize preserves depth, so read_oml's depth guard still bounds this");
        let expected = match read_required(dir, "expected/output.oml") {
            Ok(s) => s,
            Err(r) => return r,
        };
        match compare_document(&actual, &expected) {
            Ok(true) => pass(),
            Ok(false) => fail("materialized output does not match expected"),
            Err(e) => fail(format!("referee error: {e}")),
        }
    } else {
        match result {
            Ok(_) => fail("expected failure, materialize succeeded"),
            Err(_) => pass(),
        }
    }
}

fn run_normalize(dir: &Path) -> CaseResult {
    let input_text = match read_required(dir, "input.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let schema = match parse_schema(&input_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let normalized: Schema = normalize(&schema);
    let actual = to_osd(&normalized, None);
    let expected = match read_required(dir, "expected.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    match compare_schema(&actual, &expected, "exact") {
        Ok(true) => pass(),
        Ok(false) => fail("output schema does not match expected (exact structural comparison)"),
        Err(e) => fail(format!("referee error: {e}")),
    }
}

fn run_prune(dir: &Path) -> CaseResult {
    let input_text = match read_required(dir, "input.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let schema = match parse_schema(&input_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let pruned: Schema = prune(&schema);
    let actual = to_osd(&pruned, None);
    let expected = match read_required(dir, "expected.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    match compare_schema(&actual, &expected, "exact") {
        Ok(true) => pass(),
        Ok(false) => fail("output schema does not match expected (exact structural comparison)"),
        Err(e) => fail(format!("referee error: {e}")),
    }
}

fn run_is_empty(dir: &Path) -> CaseResult {
    let input_text = match read_required(dir, "input.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let schema = match parse_schema(&input_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let expected = match read_trimmed_bool(dir, "expected.txt") {
        Ok(b) => b,
        Err(r) => return r,
    };
    let actual = is_empty(&schema);
    if actual == expected {
        pass()
    } else {
        fail(format!("expected empty={expected}, got {actual}"))
    }
}

fn run_compatible_with(dir: &Path) -> CaseResult {
    let a_text = match read_required(dir, "a.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let b_text = match read_required(dir, "b.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let a = match parse_schema(&a_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema a.osd failed: {e}")),
    };
    let b = match parse_schema(&b_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema b.osd failed: {e}")),
    };
    let expected = match read_trimmed_bool(dir, "expected.txt") {
        Ok(b) => b,
        Err(r) => return r,
    };
    let actual = compatible_with(&a, &b);
    if actual == expected {
        pass()
    } else {
        fail(format!("expected compatible={expected}, got {actual}"))
    }
}

fn run_equivalent(dir: &Path) -> CaseResult {
    let a_text = match read_required(dir, "a.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let b_text = match read_required(dir, "b.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let a = match parse_schema(&a_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema a.osd failed: {e}")),
    };
    let b = match parse_schema(&b_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema b.osd failed: {e}")),
    };
    let expected = match read_trimmed_bool(dir, "expected.txt") {
        Ok(b) => b,
        Err(r) => return r,
    };
    let actual = equivalent(&a, &b);
    if actual == expected {
        pass()
    } else {
        fail(format!("expected equivalent={expected}, got {actual}"))
    }
}

fn run_extract(dir: &Path) -> CaseResult {
    let keep_text = match read_required(dir, "keep.txt") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let keep: Vec<&str> = keep_text
        .trim()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let schema_text = match read_required(dir, "schema.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let schema = match parse_schema(&schema_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let expect_ok = match read_trimmed_bool(dir, "expected/ok.txt") {
        Ok(b) => b,
        Err(r) => return r,
    };
    let result = extract(&schema, &keep);
    if expect_ok {
        let extracted = match result {
            Ok(s) => s,
            Err(e) => return fail(format!("expected success, extract failed: {e}")),
        };
        let actual = to_osd(&extracted, None);
        let expected = match read_required(dir, "expected/output.osd") {
            Ok(s) => s,
            Err(r) => return r,
        };
        match compare_schema(&actual, &expected, "exact") {
            Ok(true) => pass(),
            Ok(false) => fail("extracted schema does not match expected"),
            Err(e) => fail(format!("referee error: {e}")),
        }
    } else {
        match result {
            Ok(_) => fail("expected failure (keep set invalidates root), extract succeeded"),
            Err(_) => pass(),
        }
    }
}

/// `infer`'s default root name (`docs/06-schema-algebra.md`: `root_name =
/// "Root"`) -- no fixture case overrides it, so it's not read from a file.
const INFER_ROOT_NAME: &str = "Root";

fn run_infer(dir: &Path) -> CaseResult {
    let samples_dir = dir.join("samples");
    let mut sample_files: Vec<PathBuf> = match std::fs::read_dir(&samples_dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(e) => return fail(format!("missing samples/ dir: {e}")),
    };
    sample_files.sort();

    let allow_any_path = dir.join("allow_any.txt");
    let allow_any = allow_any_path.exists()
        && std::fs::read_to_string(&allow_any_path)
            .map(|s| s.trim() == "true")
            .unwrap_or(false);

    let mut samples: Vec<Doc> = Vec::with_capacity(sample_files.len());
    for f in &sample_files {
        let text = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => return fail(format!("reading sample {}: {e}", f.display())),
        };
        let raw = match read_oml(&text) {
            Ok(r) => r,
            Err(e) => return fail(format!("read_oml on sample {}: {e}", f.display())),
        };
        // Same reasoning as `doc_from_oml_file`'s doc comment: read_oml's
        // depth guard already ensures Doc::from_raw cannot reject this.
        samples.push(Doc::from_raw(raw).expect(
            "read_oml's depth guard already ensures Doc::from_raw cannot reject this RawNode",
        ));
    }

    let expect_ok = match read_trimmed_bool(dir, "expected/ok.txt") {
        Ok(b) => b,
        Err(r) => return r,
    };

    // Always via `infer_with_report`, discarding the report, regardless of
    // `allow_any` -- unifies the two fixture shapes (with/without
    // `allow_any.txt`) behind one call instead of branching between
    // `infer`/`infer_with_report` per issue #82's note that this needed
    // confirming empirically. `infer_with_report(..., false)` and `infer`
    // are equivalent per `infer.rs`'s own doc comment ("[`infer`] ...
    // Always infers with `allow_any: false`").
    let result = infer_with_report(&samples, INFER_ROOT_NAME, allow_any);
    if expect_ok {
        let (schema, _fallbacks) = match result {
            Ok(v) => v,
            Err(e) => return fail(format!("expected success, infer failed: {e}")),
        };
        let actual = to_osd(&schema, None);
        let expected = match read_required(dir, "expected/output.osd") {
            Ok(s) => s,
            Err(r) => return r,
        };
        // isomorphic, not exact -- §6.10: infer's generated record names
        // are implementation-derived, never canonical (see referee.rs's
        // header docs for why isomorphic and not exact/equivalent).
        match compare_schema(&actual, &expected, "isomorphic") {
            Ok(true) => pass(),
            Ok(false) => fail("inferred schema is not isomorphic to expected"),
            Err(e) => fail(format!("referee error: {e}")),
        }
    } else {
        match result {
            Ok(_) => fail("expected failure (ambiguous type, no allow_any), infer succeeded"),
            Err(_) => pass(),
        }
    }
}

fn run_lint(dir: &Path) -> CaseResult {
    let input_text = match read_required(dir, "input.osd") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let schema = match parse_schema(&input_text) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    // Findings MUST already be sorted deterministically by (code, location)
    // per §6.11 -- a direct ordered-list comparison (not set/unordered) is
    // itself part of the conformance check, not just convenient.
    //
    // `code` itself is deliberately excluded from the comparison tuple
    // below: per docs/conformance-harness.md ("lint findings' code field is
    // compared code-agnostically", mirroring §8.5.2 rule 4 for Track 2's
    // diagnostics), a fixture's expected.json may still carry the
    // pre-§8.3-namespacing bare code (`unreachable-record`) recorded against
    // the reference implementation, while an implementation that has
    // already adopted §8.3's namespaced codes (`lint.unreachable-record`)
    // must not be marked failing for that -- `severity` and `location` are
    // the real comparison; `code` stays informational until omnist-spec's
    // D-4 closes.
    let findings = lint(&schema);
    let actual_ok = findings.is_empty();
    let actual: Vec<(&str, String)> = findings
        .iter()
        .map(|f| (f.severity, f.location.clone()))
        .collect();

    let expected_text = match read_required(dir, "expected.json") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let expected_json: serde_json::Value = match serde_json::from_str(&expected_text) {
        Ok(v) => v,
        Err(e) => return fail(format!("expected.json is not valid JSON: {e}")),
    };
    let expected_ok = expected_json
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let expected: Vec<(String, String)> = expected_json
        .get("findings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            (
                f.get("severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                f.get("location")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    let actual_owned: Vec<(String, String)> = actual
        .iter()
        .map(|(s, l)| (s.to_string(), l.clone()))
        .collect();

    if actual_ok == expected_ok && actual_owned == expected {
        pass()
    } else {
        fail(format!(
            "expected ok={expected_ok} findings={expected:?}, got ok={actual_ok} findings={actual_owned:?}"
        ))
    }
}

type Runner = fn(&Path) -> CaseResult;

fn runner_for(operation: &str) -> Option<Runner> {
    match operation {
        "write" => Some(run_write),
        "validate" => Some(run_validate),
        "materialize" => Some(run_materialize),
        "normalize" => Some(run_normalize),
        "prune" => Some(run_prune),
        "is_empty" => Some(run_is_empty),
        "compatible_with" => Some(run_compatible_with),
        "equivalent" => Some(run_equivalent),
        "extract" => Some(run_extract),
        "infer" => Some(run_infer),
        "lint" => Some(run_lint),
        _ => None,
    }
}

/// Runs one operation directory, returning `(passed, failed, skipped)`.
fn run_operation(operation: &str, fixtures_dir: &Path) -> (u32, u32, u32) {
    let op_dir = fixtures_dir.join(operation);
    let Ok(read_dir) = std::fs::read_dir(&op_dir) else {
        return (0, 0, 0);
    };
    let mut cases: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    if cases.is_empty() {
        return (0, 0, 0);
    }

    let runner = runner_for(operation);
    let (mut passed, mut failed, mut skipped) = (0u32, 0u32, 0u32);
    for case_dir in &cases {
        let case_name = case_dir.file_name().unwrap().to_string_lossy();
        let why = purpose(case_dir);
        let Some(runner) = runner else {
            println!(
                "[SKIP] {operation}/{case_name} ({why}): no runner wired up yet for this operation"
            );
            skipped += 1;
            continue;
        };
        let result = runner(case_dir);
        let label = match result.status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
        };
        println!(
            "[{label}] {operation}/{case_name} ({why}): {}",
            result.message
        );
        match result.status {
            Status::Pass => passed += 1,
            Status::Fail => failed += 1,
        }
    }
    (passed, failed, skipped)
}

fn main_with_args(operations: &[String], fixtures_dir: &Path) -> u8 {
    if !fixtures_dir.is_dir() {
        eprintln!(
            "no fixtures found at {} -- has the vendor/omnist-spec submodule been checked out? \
             (git submodule update --init --recursive)",
            fixtures_dir.display()
        );
        return 2;
    }

    let ops: Vec<String> = if operations.is_empty() {
        let mut v: Vec<String> = ALL_OPERATIONS.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    } else {
        operations.to_vec()
    };

    let (mut total_pass, mut total_fail, mut total_skip) = (0u32, 0u32, 0u32);
    for op in &ops {
        let (p, f, s) = run_operation(op, fixtures_dir);
        total_pass += p;
        total_fail += f;
        total_skip += s;
    }

    println!(
        "\n{total_pass} passed, {total_fail} failed, {total_skip} skipped (across {} operation(s))",
        ops.len()
    );
    if total_fail > 0 { 1 } else { 0 }
}

fn main() -> ExitCode {
    ExitCode::from(main_with_args(
        &std::env::args().skip(1).collect::<Vec<String>>(),
        &fixtures_dir(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_real_fixtures_pass() {
        let code = main_with_args(&[], &fixtures_dir());
        assert_eq!(code, 0);
    }

    #[test]
    fn real_fixture_case_count_is_nineteen() {
        // Every real operation directory under the pinned submodule exists
        // (11/11), so `read_dir`'s `Err` path (missing directory) is not
        // reachable through this specific test -- `.unwrap_or(0)` documents
        // that rather than leaving an always-taken `Ok` branch dressed up
        // as a two-armed match.
        let total: usize = ALL_OPERATIONS
            .iter()
            .map(|op| {
                std::fs::read_dir(fixtures_dir().join(op))
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter(|e| e.path().is_dir())
                            .count()
                    })
                    .unwrap_or(0)
            })
            .sum();
        assert_eq!(total, 19);
    }

    #[test]
    fn missing_fixtures_dir_returns_two() {
        let tmp = std::env::temp_dir().join("conformance-runner-missing");
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(main_with_args(&[], &tmp), 2);
    }

    #[test]
    fn requesting_a_single_known_operation_runs_only_that_one() {
        let code = main_with_args(&["write".to_string()], &fixtures_dir());
        assert_eq!(code, 0);
    }

    #[test]
    fn requesting_an_unknown_operation_with_no_fixture_dir_is_a_no_op() {
        // `run_operation` returns (0,0,0) when the operation subdirectory
        // doesn't exist at all -- distinct from "wired but zero cases".
        let code = main_with_args(&["not-a-real-operation".to_string()], &fixtures_dir());
        assert_eq!(code, 0);
    }

    #[test]
    fn a_wired_operation_with_no_cases_reports_zero_zero_zero() {
        let tmp = std::env::temp_dir().join("conformance-runner-empty-op");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("write")).unwrap();
        let (p, f, s) = run_operation("write", &tmp);
        assert_eq!((p, f, s), (0, 0, 0));
    }

    #[test]
    fn an_operation_directory_with_no_runner_wired_is_skipped() {
        let tmp = std::env::temp_dir().join("conformance-runner-unwired-op");
        let _ = std::fs::remove_dir_all(&tmp);
        let case = tmp.join("frobnicate").join("some-case");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(case.join("purpose.txt"), "happy-path\ntest\n").unwrap();
        let (p, f, s) = run_operation("frobnicate", &tmp);
        assert_eq!((p, f, s), (0, 0, 1));
    }

    #[test]
    fn status_debug_format_is_derived_and_readable() {
        // `Status`'s `Debug` derive is otherwise only invoked by
        // `assert_eq!`'s failure-message path, which a fully-passing test
        // suite never exercises -- covered directly here instead of
        // relying on an intentionally-failing assertion.
        assert_eq!(format!("{:?}", Status::Pass), "Pass");
        assert_eq!(format!("{:?}", Status::Fail), "Fail");
    }

    #[test]
    fn write_missing_input_file_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-write-missing-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("expected.oml"), "a: 1\n").unwrap();
        let r = run_write(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn write_missing_expected_file_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-write-missing-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.oml"), "a: 1\n").unwrap();
        let r = run_write(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn write_mismatched_output_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-write-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.oml"), "a: 1\n").unwrap();
        std::fs::write(tmp.join("expected.oml"), "a: 2\n").unwrap();
        let r = run_write(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn write_unparseable_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-write-bad-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.oml"), "[[[not valid\n").unwrap();
        std::fs::write(tmp.join("expected.oml"), "a: 1\n").unwrap();
        let r = run_write(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn validate_missing_schema_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-validate-missing-schema");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("input.oml"), "a: 1\n").unwrap();
        let r = run_validate(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn is_empty_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-is-empty-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("input.osd"),
            "record R {\n    \"x\": string,\n}\nroot R\n",
        )
        .unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_is_empty(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn extract_missing_keep_file_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-missing-keep");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn infer_missing_samples_dir_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-missing-samples");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn lint_bad_json_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-lint-bad-json");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("input.osd"),
            "record R {\n    \"x\": string,\n}\nroot R\n",
        )
        .unwrap();
        std::fs::write(tmp.join("expected.json"), "not json").unwrap();
        let r = run_lint(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    const SIMPLE_SCHEMA: &str = "record R {\n    \"x\": string,\n}\nroot R\n";

    // --- run_write: remaining branches ---------------------------------

    #[test]
    fn write_bad_expected_oml_hits_referee_error_path() {
        let tmp = std::env::temp_dir().join("conformance-runner-write-bad-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.oml"), "a: 1\n").unwrap();
        std::fs::write(tmp.join("expected.oml"), "[[[not valid\n").unwrap();
        let r = run_write(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("referee error"), "{}", r.message);
    }

    // --- run_validate: remaining branches -------------------------------

    #[test]
    fn validate_missing_expected_ok_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-validate-missing-expected-ok");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_validate(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn validate_missing_input_oml_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-validate-missing-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_validate(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn validate_unparseable_input_oml_fails() {
        // Exercises `doc_from_oml_file`'s `read_oml` `Err` arm specifically
        // (distinct from a missing input.oml file).
        let tmp = std::env::temp_dir().join("conformance-runner-validate-bad-input-oml");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "[[[not valid\n").unwrap();
        let r = run_validate(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("parse input.oml"), "{}", r.message);
    }

    #[test]
    fn validate_bad_schema_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-validate-bad-schema");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), "not valid osd").unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_validate(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn validate_result_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-validate-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        // Schema requires "x"; input conforms, so actual ok=true, but the
        // fixture (wrongly) expects ok=false.
        std::fs::write(tmp.join("expected/ok.txt"), "false\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_validate(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("expected ok=false"), "{}", r.message);
    }

    // --- run_materialize: remaining branches ----------------------------

    #[test]
    fn materialize_missing_expected_ok_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-missing-expected-ok");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn materialize_bad_input_oml_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-bad-input-oml");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "[[[not valid\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("read_oml failed"), "{}", r.message);
    }

    #[test]
    fn materialize_bad_expected_output_hits_referee_error_path() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-bad-expected-output");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        std::fs::write(tmp.join("expected/output.oml"), "[[[not valid\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("referee error"), "{}", r.message);
    }

    #[test]
    fn materialize_missing_schema_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-missing-schema");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn materialize_bad_schema_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-bad-schema");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), "not valid osd").unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn materialize_missing_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-missing-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn materialize_expected_success_but_actually_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-unexpected-fail");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        // Missing mandatory "x" -- materialize will fail the shape check.
        std::fs::write(tmp.join("input.oml"), "y: \"a\"\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.message.contains("expected success, materialize failed"),
            "{}",
            r.message
        );
    }

    #[test]
    fn materialize_missing_expected_output_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-missing-output");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn materialize_output_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-output-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        std::fs::write(tmp.join("expected/output.oml"), "x: \"different\"\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.message.contains("does not match expected"),
            "{}",
            r.message
        );
    }

    #[test]
    fn materialize_expected_failure_but_actually_succeeds() {
        let tmp = std::env::temp_dir().join("conformance-runner-materialize-unexpected-success");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "false\n").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("input.oml"), "x: \"a\"\n").unwrap();
        let r = run_materialize(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("expected failure"), "{}", r.message);
    }

    // --- run_normalize / run_prune: remaining branches ------------------

    #[test]
    fn normalize_missing_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-normalize-missing-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = run_normalize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn normalize_bad_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-normalize-bad-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), "not valid osd").unwrap();
        let r = run_normalize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn normalize_missing_expected_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-normalize-missing-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_normalize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn normalize_output_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-normalize-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(
            tmp.join("expected.osd"),
            "record Other {\n    \"y\": string,\n}\nroot Other\n",
        )
        .unwrap();
        let r = run_normalize(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn normalize_referee_error_on_bad_expected() {
        let tmp = std::env::temp_dir().join("conformance-runner-normalize-bad-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.osd"), "not valid osd").unwrap();
        let r = run_normalize(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("referee error"), "{}", r.message);
    }

    #[test]
    fn prune_missing_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-prune-missing-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = run_prune(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn prune_bad_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-prune-bad-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), "not valid osd").unwrap();
        let r = run_prune(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn prune_missing_expected_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-prune-missing-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_prune(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn prune_referee_error_on_bad_expected() {
        let tmp = std::env::temp_dir().join("conformance-runner-prune-bad-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.osd"), "not valid osd").unwrap();
        let r = run_prune(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("referee error"), "{}", r.message);
    }

    #[test]
    fn prune_output_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-prune-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(
            tmp.join("expected.osd"),
            "record Other {\n    \"y\": string,\n}\nroot Other\n",
        )
        .unwrap();
        let r = run_prune(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    // --- run_is_empty / run_compatible_with / run_equivalent ------------

    #[test]
    fn is_empty_missing_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-is-empty-missing-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = run_is_empty(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn is_empty_bad_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-is-empty-bad-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), "not valid osd").unwrap();
        let r = run_is_empty(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn is_empty_missing_expected_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-is-empty-missing-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_is_empty(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn compatible_with_missing_a_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-compatible-missing-a");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_compatible_with(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn compatible_with_missing_b_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-compatible-missing-b");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_compatible_with(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn compatible_with_bad_a_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-compatible-bad-a");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), "not valid osd").unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_compatible_with(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn compatible_with_bad_b_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-compatible-bad-b");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("b.osd"), "not valid osd").unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_compatible_with(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn compatible_with_missing_expected_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-compatible-missing-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_compatible_with(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn compatible_with_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-compatible-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        // Identical schemas ARE compatible, so expecting false is wrong.
        std::fs::write(tmp.join("expected.txt"), "false\n").unwrap();
        let r = run_compatible_with(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn equivalent_missing_a_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-equivalent-missing-a");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_equivalent(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn equivalent_missing_b_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-equivalent-missing-b");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_equivalent(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn equivalent_bad_a_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-equivalent-bad-a");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), "not valid osd").unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_equivalent(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn equivalent_bad_b_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-equivalent-bad-b");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("b.osd"), "not valid osd").unwrap();
        std::fs::write(tmp.join("expected.txt"), "true\n").unwrap();
        let r = run_equivalent(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn equivalent_missing_expected_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-equivalent-missing-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_equivalent(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn equivalent_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-equivalent-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("b.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected.txt"), "false\n").unwrap();
        let r = run_equivalent(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    // --- run_extract: remaining branches ---------------------------------

    #[test]
    fn extract_missing_schema_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-missing-schema");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("keep.txt"), "x").unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn extract_bad_schema_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-bad-schema");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("keep.txt"), "x").unwrap();
        std::fs::write(tmp.join("schema.osd"), "not valid osd").unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn extract_missing_expected_ok_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-missing-expected-ok");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("keep.txt"), "x").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn extract_expected_success_but_actually_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-unexpected-fail");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        // Dropping the only field ("x") invalidates root R -- extract fails.
        std::fs::write(tmp.join("keep.txt"), "nonexistent").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.message.contains("expected success, extract failed"),
            "{}",
            r.message
        );
    }

    #[test]
    fn extract_missing_expected_output_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-missing-output");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("keep.txt"), "x").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn extract_output_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-output-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("keep.txt"), "x").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(
            tmp.join("expected/output.osd"),
            "record Other {\n    \"y\": string,\n}\nroot Other\n",
        )
        .unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn extract_referee_error_on_bad_expected_output() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-bad-expected-output");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("keep.txt"), "x").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("expected/output.osd"), "not valid osd").unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("referee error"), "{}", r.message);
    }

    #[test]
    fn extract_expected_failure_but_actually_succeeds() {
        let tmp = std::env::temp_dir().join("conformance-runner-extract-unexpected-success");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::write(tmp.join("keep.txt"), "x").unwrap();
        std::fs::write(tmp.join("schema.osd"), SIMPLE_SCHEMA).unwrap();
        std::fs::write(tmp.join("expected/ok.txt"), "false\n").unwrap();
        let r = run_extract(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("expected failure"), "{}", r.message);
    }

    // --- run_infer: remaining branches ------------------------------------

    fn write_sample(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn infer_sample_read_failure_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-sample-is-a-dir");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        // A directory named like a sample file: `read_dir` lists it, but
        // `read_to_string` on it is a real IO error -- exercises the
        // reading-failure branch distinct from a parse failure.
        std::fs::create_dir_all(samples.join("1.oml")).unwrap();
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("reading sample"), "{}", r.message);
    }

    #[test]
    fn infer_unparseable_sample_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-bad-sample");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        std::fs::create_dir_all(&samples).unwrap();
        write_sample(&samples, "1.oml", "[[[not valid\n");
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("read_oml on sample"), "{}", r.message);
    }

    #[test]
    fn infer_missing_expected_ok_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-missing-expected-ok");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        std::fs::create_dir_all(&samples).unwrap();
        write_sample(&samples, "1.oml", "x: \"a\"\n");
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn infer_expected_success_but_actually_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-unexpected-fail");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::create_dir_all(&samples).unwrap();
        // Same label with conflicting scalar kinds across samples, no
        // allow_any.txt (defaults to false) -- infer errors (ambiguous).
        write_sample(&samples, "1.oml", "x: \"a\"\n");
        write_sample(&samples, "2.oml", "x: 1\n");
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.message.contains("expected success, infer failed"),
            "{}",
            r.message
        );
    }

    #[test]
    fn infer_missing_expected_output_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-missing-output");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::create_dir_all(&samples).unwrap();
        write_sample(&samples, "1.oml", "x: \"a\"\n");
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn infer_output_not_isomorphic_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::create_dir_all(&samples).unwrap();
        write_sample(&samples, "1.oml", "x: \"a\"\n");
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(
            tmp.join("expected/output.osd"),
            "record Root {\n    \"y\": string,\n}\nroot Root\n",
        )
        .unwrap();
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("not isomorphic"), "{}", r.message);
    }

    #[test]
    fn infer_referee_error_on_bad_expected_output() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-bad-expected-output");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::create_dir_all(&samples).unwrap();
        write_sample(&samples, "1.oml", "x: \"a\"\n");
        std::fs::write(tmp.join("expected/ok.txt"), "true\n").unwrap();
        std::fs::write(tmp.join("expected/output.osd"), "not valid osd").unwrap();
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("referee error"), "{}", r.message);
    }

    #[test]
    fn infer_expected_failure_but_actually_succeeds() {
        let tmp = std::env::temp_dir().join("conformance-runner-infer-unexpected-success");
        let _ = std::fs::remove_dir_all(&tmp);
        let samples = tmp.join("samples");
        std::fs::create_dir_all(tmp.join("expected")).unwrap();
        std::fs::create_dir_all(&samples).unwrap();
        write_sample(&samples, "1.oml", "x: \"a\"\n");
        std::fs::write(tmp.join("expected/ok.txt"), "false\n").unwrap();
        let r = run_infer(&tmp);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("expected failure"), "{}", r.message);
    }

    // --- run_lint: remaining branches --------------------------------------

    #[test]
    fn lint_missing_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-lint-missing-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let r = run_lint(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn lint_bad_input_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-lint-bad-input");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), "not valid osd").unwrap();
        let r = run_lint(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn lint_missing_expected_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-lint-missing-expected");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        let r = run_lint(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn lint_result_mismatch_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-lint-mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("input.osd"), SIMPLE_SCHEMA).unwrap();
        // SIMPLE_SCHEMA has no findings; claim it has one to force a mismatch.
        std::fs::write(
            tmp.join("expected.json"),
            r#"{"ok": false, "findings": [{"code": "unreachable-record", "severity": "warning", "location": "Nope", "message": "x"}]}"#,
        )
        .unwrap();
        let r = run_lint(&tmp);
        assert_eq!(r.status, Status::Fail);
    }

    // --- run_operation: FAIL-branch coverage (distinct from calling a
    // driver directly -- this exercises run_operation's own PASS/FAIL
    // printing and counters) --------------------------------------------

    #[test]
    fn run_operation_counts_a_failing_case_and_prints_fail() {
        let tmp = std::env::temp_dir().join("conformance-runner-operation-with-failure");
        let _ = std::fs::remove_dir_all(&tmp);
        let case = tmp.join("write").join("01-broken");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(case.join("purpose.txt"), "happy-path\ntest\n").unwrap();
        std::fs::write(case.join("input.oml"), "a: 1\n").unwrap();
        std::fs::write(case.join("expected.oml"), "a: 2\n").unwrap();
        let (passed, failed, skipped) = run_operation("write", &tmp);
        assert_eq!((passed, failed, skipped), (0, 1, 0));
    }

    #[test]
    fn main_with_args_reports_nonzero_exit_when_any_case_fails() {
        let tmp = std::env::temp_dir().join("conformance-runner-main-with-failure");
        let _ = std::fs::remove_dir_all(&tmp);
        let case = tmp.join("write").join("01-broken");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(case.join("purpose.txt"), "happy-path\ntest\n").unwrap();
        std::fs::write(case.join("input.oml"), "a: 1\n").unwrap();
        std::fs::write(case.join("expected.oml"), "a: 2\n").unwrap();
        let code = main_with_args(&["write".to_string()], &tmp);
        assert_eq!(code, 1);
    }
}
