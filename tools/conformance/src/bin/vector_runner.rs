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
//! ## Comparison mode: (path, code) sets, per section 8.5.2
//!
//! Every failing vector is compared as a **set of `(path, code)` pairs**
//! (E-17: message text never compared, order never compared, exact match --
//! an extra diagnostic fails a vector just as a missing one does). This
//! runner is NOT in code-agnostic mode any more (it was, up to spec
//! v0.9.1-beta): the library's errors now carry the structured fields the
//! taxonomy needs --
//!
//! - `ParseError { line, col, code, message }` (OML and the four codecs;
//!   `position()` is the `line:col` text-position path of E-11),
//! - `SchemaError { path, code, message }` (OSD, `infer`, `extract`),
//! - `DocumentError { path, code: Option<_>, message }` (document limits,
//!   the data-XML profile refusals),
//! - `WriteError { path, code, .. }` and `WriteReport` adjustments (write),
//! - `ValidationError { path, code }` (validate/materialize, namespaced per
//!   family by `ErrorCode::as_str`).
//!
//! A vector whose actual error carries no code or no path FAILS; it is never
//! skipped for lack of structure.
//!
//! ## Skips are only ever E-20 "not yet implemented"
//!
//! Three categories exist, all E-20 "not yet implemented" (no E-21
//! documented divergence applies to this port, and a skip reason is checked
//! by the `every_skip_reason_is_true_for_its_vector` test against the
//! vector's own input, so it cannot drift):
//!
//! 1. `document-model/limits.json` (6): the vector declares
//!    `declared_max_depth`/`declared_max_nodes`/`declared_max_int_digits`,
//!    and this port's limits are compile-time constants with no runtime
//!    configuration surface, so the boundary cannot be pinned.
//! 2. `formats-yaml/alias-expansion.json` (6): every vector declares
//!    `declared_max_alias_expansion`, and D-18 (section 2.4.1) is not
//!    implemented -- tracked as DIV-3 (section 9.4). Never run against the
//!    port's own default limit, which would be a false pass.
//! 3. `extensions-osd-oml/` (28): the OSD-OML extension operations
//!    (`parse_schema_oml`, `schema_from_document`, `schema_to_document`,
//!    `write_schema_oml`) have no implementation in this port yet.
//!
//! **D-6 (integer/number kind collapse) does not apply**:
//! `omnist::document::Scalar` has separate `Int`/`Float` variants, so there
//! is nothing to skip.
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
use omnist::formats::xml::{read_xml_report, write_xml};
use omnist::formats::yaml::{read_yaml, write_yaml};
use omnist::infer::infer_with_report;
use omnist::materialize::materialize;
use omnist::oml::{read_oml, write_oml};
use omnist::ops::{compatible_with, equivalent, extract, is_empty, lint};
use omnist::osd::{parse_schema, to_osd};
use omnist::report::WriteReport;
use omnist::schema::{ErrorFamily, Schema};
use omnist_cli::{Fmt, parse_schema_bytes, read_document_bytes, read_oml_bytes};
use serde_json::Value as Json;

/// Every `declared_max_*` key in `test-suite/README.md`'s allowlist. A key
/// missing from this list is NOT skipped -- the vector would run against the
/// port's own default limit, which is a false pass on a boundary vector.
const LIMIT_KEYS: &[&str] = &[
    "declared_max_depth",
    "declared_max_nodes",
    "declared_max_int_digits",
    // D-18 (section 2.4.1), new in v0.18.0-beta.
    "declared_max_alias_expansion",
];

/// The E-20 "not yet implemented" skip reason for a vector carrying a
/// `declared_max_*` key, or `None` if it carries none. The wording depends on
/// the key, so the reason is true for THAT vector.
fn limit_skip_reason(input: &Json) -> Option<String> {
    if input.get("declared_max_alias_expansion").is_some() {
        return Some(
            "not yet implemented: D-18 alias expansion limit (section 2.4.1) is not enforced by \
             this port's YAML reader; tracked as DIV-3 (section 9.4) and omnist-rs#180"
                .to_string(),
        );
    }
    LIMIT_KEYS
        .iter()
        .find(|k| input.get(**k).is_some())
        .map(|k| {
            format!(
                "not yet implemented: {k} needs a runtime-configurable limit; this port's \
                 limits are compile-time constants (omnist-rs#181)"
            )
        })
}

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

/// omnist-spec Sec8.5.3: strip whitespace strictly between '>' and '<'
/// before comparing a `write` vector's expected/actual text for XML. Safe
/// because this crate never produces mixed-content XML (Document model
/// Sec2: a node has either child edges or one scalar value, never both),
/// so this whitespace can only ever be inter-tag formatting, never real
/// text data -- this function is deliberately unconditional (no
/// mixed-content guard) on that basis, matching the spec's own stated
/// scoping, not a general-purpose XML formatter.
fn normalize_xml_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '>' {
            while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
                chars.next();
            }
        }
    }
    out
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

/// One diagnostic as section 8.5.2 compares it: a `(path, code)` pair.
type Diag = (String, String);

fn expected_diags(v: &Json) -> Vec<Diag> {
    v["expect"]["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|d| {
            (
                d["path"].as_str().unwrap_or_default().to_string(),
                d["code"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// E-17 rules 2 and 3: compare as a SET, exactly.
fn diags_match(expected: &[Diag], actual: &[Diag]) -> bool {
    let e: std::collections::BTreeSet<&Diag> = expected.iter().collect();
    let a: std::collections::BTreeSet<&Diag> = actual.iter().collect();
    e == a
}

/// Pass if the diagnostic sets match exactly, else fail showing both.
fn check_diags(expected: &[Diag], actual: &[Diag]) -> VResult {
    if diags_match(expected, actual) {
        pass()
    } else {
        fail(format!(
            "diagnostics differ: expected {expected:?}, got {actual:?}"
        ))
    }
}

// ---------------------------------------------------------------------------
// Per-operation drivers
// ---------------------------------------------------------------------------

/// A read-side vector's input (E-27): source text, or the bytes of a
/// `bytes_hex` field.
#[derive(Debug)]
enum Source {
    Text(String),
    Bytes(Vec<u8>),
}

/// Reads a read-side vector's input: exactly one of `text` and `bytes_hex`
/// (E-27). `bytes_hex` is decoded to the raw bytes and nothing else -- no
/// trimming, no case folding -- and is NEVER decoded to text here: the caller
/// hands the bytes to the CLI's byte-oriented entry point, whose own
/// [`decode_input`](omnist_cli::decode_input) is the D-14 check.
fn source_of(input: &Json) -> Result<Source, String> {
    match (input.get("text"), input.get("bytes_hex")) {
        (Some(_), Some(_)) => Err(
            "the input carries both `text` and `bytes_hex`; exactly one is allowed (E-27)".into(),
        ),
        (None, None) => Err(
            "the input carries neither `text` nor `bytes_hex`; exactly one is required (E-27)"
                .into(),
        ),
        (Some(text), None) => match text.as_str() {
            Some(text) => Ok(Source::Text(text.to_string())),
            None => Err("`text` is not a string".into()),
        },
        (None, Some(hex)) => match hex.as_str() {
            Some(hex) => decode_hex(hex).map(Source::Bytes),
            None => Err("`bytes_hex` is not a string".into()),
        },
    }
}

/// Strict `bytes_hex` decoding: lowercase hexadecimal, two digits per byte,
/// no separators, no prefix (E-27).
fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err(format!("`bytes_hex` {hex:?} has an odd number of digits"));
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    hex.as_bytes()
        .chunks(2)
        .map(|pair| match (digit(pair[0]), digit(pair[1])) {
            (Some(hi), Some(lo)) => Ok(hi << 4 | lo),
            _ => Err(format!("`bytes_hex` {hex:?} is not lowercase hexadecimal")),
        })
        .collect()
}

/// The CLI's `--from` value for a vector's `format`, or `None` if unknown.
fn cli_format(format: &str) -> Option<Fmt> {
    match format {
        "json" => Some(Fmt::Json),
        "yaml" => Some(Fmt::Yaml),
        "toml" => Some(Fmt::Toml),
        "xml" => Some(Fmt::Xml),
        "oml" => Some(Fmt::Oml),
        _ => None,
    }
}

fn run_parse(v: &Json) -> VResult {
    let input = &v["input"];
    if let Some(reason) = limit_skip_reason(input) {
        return skip(reason);
    }
    let format = input["format"].as_str().unwrap_or("oml");
    let source = match source_of(input) {
        Ok(s) => s,
        Err(message) => return fail(message),
    };

    // `format.attribute-dropped`/`format.namespace-dropped` (spec section
    // 8.3.8, D-3) are read-time diagnostics only XML's reader emits, via
    // `read_xml_report`'s `report`; the other formats have none.
    let mut xml_report = WriteReport::new();
    let result: Result<RawNode, OmnistError> = match (source, format) {
        (Source::Bytes(bytes), "oml") => read_oml_bytes(bytes),
        (Source::Bytes(bytes), other) => match cli_format(other) {
            Some(fmt) => read_document_bytes(fmt, bytes, None).map(|d| d.to_raw()),
            None => return fail(format!("unknown format {other:?}")),
        },
        (Source::Text(text), "oml") => read_oml(&text).map_err(OmnistError::from),
        (Source::Text(text), "json") => read_json(&text).map(|d| d.to_raw()),
        (Source::Text(text), "toml") => read_toml(&text).map(|d| d.to_raw()),
        (Source::Text(text), "xml") => {
            read_xml_report(&text, Some(&mut xml_report)).map(|d| d.to_raw())
        }
        (Source::Text(text), "yaml") => read_yaml(&text).map(|d| d.to_raw()),
        (Source::Text(_), other) => return fail(format!("unknown format {other:?}")),
    };

    match result {
        Ok(raw) => {
            if !expect_ok(v) {
                return fail("expected failure, parse succeeded");
            }
            let expected = decode_document(&v["expect"]["document"]);
            if raw != expected {
                return fail("parsed document does not match expected");
            }
            // A successful read may still report diagnostics (XML's dropped
            // attributes/namespaces); the set must match exactly, empty
            // included.
            let actual: Vec<Diag> = xml_report
                .adjustments()
                .iter()
                .map(|a| (a.path.clone(), a.code.clone()))
                .collect();
            check_diags(&expected_diags(v), &actual)
        }
        Err(e) => {
            if expect_ok(v) {
                return fail(format!("expected success, parse failed: {e}"));
            }
            parse_failure_result(&e, &expected_diags(v))
        }
    }
}
fn parse_failure_result(e: &OmnistError, expected: &[Diag]) -> VResult {
    match error_diag(e) {
        Some(actual) => check_diags(expected, &[actual]),
        None => fail(format!(
            "the read failed with an error that carries no structured (path, code): {e}"
        )),
    }
}
/// The `(path, code)` a read failure carries: a syntax-level `ParseError`'s
/// `line:col` text position (E-11) and `parse.*` code, or a `DocumentError`'s
/// own Document path and code (limits, the data-XML profile refusals).
/// `None` if the error carries no structured code -- the caller FAILS the
/// vector then; it is never a skip.
fn error_diag(e: &OmnistError) -> Option<Diag> {
    match e {
        OmnistError::Parse(pe) => Some((pe.position(), pe.code.clone())),
        OmnistError::Schema(se) => Some((se.path.clone(), se.code.clone())),
        OmnistError::Document(de) => de.code.clone().map(|c| (de.path.clone(), c)),
        _ => None,
    }
}

fn run_parse_schema(v: &Json) -> VResult {
    let source = match source_of(&v["input"]) {
        Ok(s) => s,
        Err(message) => return fail(message),
    };
    let result: Result<Schema, OmnistError> = match source {
        Source::Bytes(bytes) => parse_schema_bytes(bytes),
        Source::Text(text) => parse_schema(&text).map_err(OmnistError::from),
    };
    match result {
        Ok(_) => {
            if expect_ok(v) {
                pass()
            } else {
                fail("expected failure, parse_schema succeeded")
            }
        }
        Err(e) => {
            if expect_ok(v) {
                return fail(format!("expected success, parse_schema failed: {e}"));
            }
            // parse_schema_bytes/parse_schema only ever fail with `Parse`
            // (D-14, via decode_input) or `Schema` (OSD's own tokenizer/
            // parser) -- both are structured per `error_diag`, unlike
            // `run_parse`'s wider `OmnistError`, so there is no untested
            // "no structured code" case to report here.
            let actual = error_diag(&e).expect(
                "a parse_schema failure is always a Parse or Schema error, both structured",
            );
            check_diags(&expected_diags(v), &[actual])
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
    let actual: Vec<Diag> = result
        .errors()
        .iter()
        .map(|e| {
            (
                e.path.clone(),
                e.code.as_str(ErrorFamily::Validate).to_string(),
            )
        })
        .collect();
    check_diags(&expected_diags(v), &actual)
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
                let actual: Vec<Diag> =
                    e.0.errors()
                        .iter()
                        .map(|err| {
                            (
                                err.path.clone(),
                                err.code.as_str(ErrorFamily::Materialize).to_string(),
                            )
                        })
                        .collect();
                check_diags(&expected_diags(v), &actual)
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
    let report_diags: Vec<Diag> = report
        .iter()
        .map(|a| (a.path.clone(), a.code.clone()))
        .collect();
    match result {
        Ok(text) => {
            if !expect_ok(v) {
                return fail("expected failure, write succeeded");
            }
            if let Some(expected_text) = v["expect"]["text"].as_str() {
                let (got, want) = if format == "xml" {
                    (
                        normalize_xml_whitespace(text.trim()),
                        normalize_xml_whitespace(expected_text.trim()),
                    )
                } else {
                    (text.trim().to_string(), expected_text.trim().to_string())
                };
                if got != want {
                    return fail(format!(
                        "expected text {expected_text:?}, got {:?}",
                        text.trim()
                    ));
                }
            }
            // A successful write may carry diagnostics alongside `ok: true`
            // (section 8.5.3); the set must match exactly, empty included.
            check_diags(&expected_diags(v), &report_diags)
        }
        Err(e) => {
            if expect_ok(v) {
                return fail(format!("expected success, write failed: {e}"));
            }
            match (&e.path, &e.code) {
                (Some(path), Some(code)) => {
                    check_diags(&expected_diags(v), &[(path.clone(), code.clone())])
                }
                _ => fail(format!(
                    "the write failed with an error that carries no structured (path, code): {e}"
                )),
            }
        }
    }
}
fn run_schema_producing(v: &Json, f: impl Fn(&Schema) -> Schema) -> VResult {
    let schema = match parse_schema(v["input"]["schema"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return fail(format!("parse_schema failed: {e}")),
    };
    let actual = to_osd(&f(&schema), None)
        .expect("vector-suite schemas carry no C0-control label for OSD-14 to reject");
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
        let actual = to_osd(&extracted, None)
            .expect("vector-suite schemas carry no C0-control label for OSD-14 to reject");
        let expected = v["expect"]["schema"].as_str().unwrap_or_default();
        match compare_schema(&actual, expected, "exact") {
            Ok(true) => pass(),
            Ok(false) => fail("extracted schema does not match expected"),
            Err(e) => fail(format!("referee error: {e}")),
        }
    } else {
        match result {
            Ok(_) => fail("expected failure, extract succeeded"),
            Err(e) => check_diags(&expected_diags(v), &[(e.path.clone(), e.code.clone())]),
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
    let expected: std::collections::BTreeSet<(String, String, String)> = v["expect"]["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|f| {
            (
                f["code"].as_str().unwrap_or_default().to_string(),
                f["severity"].as_str().unwrap_or_default().to_string(),
                f["location"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let actual: std::collections::BTreeSet<(String, String, String)> = findings
        .iter()
        .map(|f| {
            (
                f.code.to_string(),
                f.severity.to_string(),
                f.location.clone(),
            )
        })
        .collect();
    if expected != actual {
        return fail(format!(
            "findings (code, severity, location) differ: expected {expected:?}, got {actual:?}"
        ));
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
        let (schema, fallbacks) = match result {
            Ok(v) => v,
            Err(e) => return fail(format!("expected success, infer failed: {e}")),
        };
        let actual = to_osd(&schema, None)
            .expect("vector-suite schemas carry no C0-control label for OSD-14 to reject");
        let expected = v_expect_schema(v);
        match compare_schema(&actual, &expected, "isomorphic") {
            Ok(true) => {}
            Ok(false) => return fail("inferred schema is not isomorphic to expected"),
            Err(e) => return fail(format!("referee error: {e}")),
        }
        // S-21: every opening to `any` must be reported (`fallbacks`), and
        // none may occur when `allow_any` is off. `fallbacks` is compared by
        // location (reason text is message-like, never compared, E-17).
        if let Some(expected_fallbacks) = v["expect"].get("fallbacks").and_then(Json::as_array) {
            let want: std::collections::BTreeSet<String> = expected_fallbacks
                .iter()
                .map(|f| f["location"].as_str().unwrap_or_default().to_string())
                .collect();
            let got: std::collections::BTreeSet<String> =
                fallbacks.iter().map(|f| f.location.clone()).collect();
            if want != got || expected_fallbacks.len() != fallbacks.len() {
                return fail(format!("fallbacks differ: expected {want:?}, got {got:?}"));
            }
        }
        pass()
    } else {
        match result {
            Ok(_) => fail("expected failure, infer succeeded"),
            Err(e) => check_diags(&expected_diags(v), &[(e.path.clone(), e.code.clone())]),
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

/// The four operations that belong to the OSD-OML extension
/// (`docs/extensions/osd-oml.md`, section E.11).
const EXTENSION_OPERATIONS: &[&str] = &[
    "parse_schema_oml",
    "schema_from_document",
    "schema_to_document",
    "write_schema_oml",
];

fn dispatch(v: &Json) -> VResult {
    let op = v["operation"].as_str().unwrap_or("");
    // E-27: `bytes_hex` belongs to the three read-side drivers and no other;
    // a vector giving it to any other operation is malformed, not runnable.
    if v["input"].get("bytes_hex").is_some()
        && !matches!(op, "parse" | "parse_schema" | "parse_schema_oml")
    {
        return fail(format!(
            "operation {op:?} does not accept `bytes_hex` (E-27)"
        ));
    }
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
        // The OSD-OML extension (docs/extensions/osd-oml.md): not implemented
        // by this port. E-20 "not yet implemented".
        op if EXTENSION_OPERATIONS.contains(&op) => skip(format!(
            "not yet implemented: {op} belongs to the OSD-OML extension, which this port does \
             not implement (omnist-rs#175)"
        )),
        // An operation this runner knows nothing about is a FAIL, never a
        // silent skip: a new spec operation must be wired up or explicitly
        // classified.
        other => fail(format!("no driver for unknown operation {other:?}")),
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

/// Runs the whole suite in `dir` and returns the process exit code: `2` if
/// the suite is missing, `1` if ANY vector fails, else `0`. Per section 8.5.5
/// (E-22) a conformant run fails the build on a nonzero fail count and never
/// on a nonzero skip count; there is deliberately no per-vector allowlist of
/// tolerated failures.
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
         diagnostics compared as (path, code) sets (section 8.5.2)"
    );
    if failed > 0 {
        eprintln!("{failed} vector(s) failed");
        return 1;
    }
    0
}

fn main() -> ExitCode {
    ExitCode::from(main_with_dir(&suite_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnist::error::SchemaError;

    // ------------------------------------------------------------- E-27

    #[test]
    fn source_of_rejects_both_text_and_bytes_hex() {
        let input = json!({"format": "json", "text": "1", "bytes_hex": "31"});
        assert!(source_of(&input).unwrap_err().contains("both"));
    }

    #[test]
    fn source_of_rejects_neither_text_nor_bytes_hex() {
        let input = json!({"format": "json"});
        assert!(source_of(&input).unwrap_err().contains("neither"));
    }

    #[test]
    fn source_of_rejects_a_non_string_text_field() {
        let input = json!({"text": 1});
        assert_eq!(source_of(&input).unwrap_err(), "`text` is not a string");
    }

    #[test]
    fn source_of_rejects_a_non_string_bytes_hex_field() {
        let input = json!({"bytes_hex": 1});
        assert_eq!(
            source_of(&input).unwrap_err(),
            "`bytes_hex` is not a string"
        );
    }

    #[test]
    fn source_of_text_and_bytes_hex_happy_paths() {
        assert!(matches!(
            source_of(&json!({"text": "a: 1"})).unwrap(),
            Source::Text(t) if t == "a: 1"
        ));
        assert!(matches!(
            source_of(&json!({"bytes_hex": "6100"})).unwrap(),
            Source::Bytes(b) if b == vec![0x61, 0x00]
        ));
    }

    #[test]
    fn decode_hex_rejects_an_odd_number_of_digits() {
        assert!(
            decode_hex("6")
                .unwrap_err()
                .contains("odd number of digits")
        );
    }

    #[test]
    fn decode_hex_rejects_uppercase_or_non_hex_digits() {
        assert!(
            decode_hex("6G")
                .unwrap_err()
                .contains("lowercase hexadecimal")
        );
        assert!(
            decode_hex("6F")
                .unwrap_err()
                .contains("lowercase hexadecimal")
        );
    }

    #[test]
    fn cli_format_covers_every_codec_and_rejects_unknown() {
        for (name, fmt) in [
            ("json", Fmt::Json),
            ("yaml", Fmt::Yaml),
            ("toml", Fmt::Toml),
            ("xml", Fmt::Xml),
            ("oml", Fmt::Oml),
        ] {
            assert_eq!(cli_format(name), Some(fmt));
        }
        assert_eq!(cli_format("yamlx"), None);
    }

    #[test]
    fn run_parse_rejects_bytes_hex_on_an_unknown_format() {
        let v = json!({
            "operation": "parse",
            "input": {"format": "yamlx", "bytes_hex": "6100"},
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_bytes_hex_on_a_known_format_runs_through_the_cli_entry_point() {
        let v = json!({
            "operation": "parse",
            "input": {"format": "json", "bytes_hex": "31"},
            "expect": {"ok": true, "document": {"scalar": {"kind": "integer", "value": 1}}}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_parse_bytes_hex_invalid_utf8_reports_d14_through_the_cli() {
        let v = json!({
            "operation": "parse",
            "input": {"format": "json", "bytes_hex": "80"},
            "expect": {"ok": false, "diagnostics": [{"path": "1:1", "code": "parse.invalid-encoding"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn error_diag_reports_a_schema_error_path_and_code() {
        let e = OmnistError::Schema(SchemaError::new("R", "schema.no-root", "no root"));
        assert_eq!(
            error_diag(&e),
            Some(("R".to_string(), "schema.no-root".to_string()))
        );
    }

    #[test]
    fn run_parse_schema_bytes_hex_runs_through_the_cli_entry_point() {
        let text = "record R { \"a\": string } root R\n";
        let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
        let v = json!({
            "operation": "parse_schema",
            "input": {"bytes_hex": hex},
            "expect": {"ok": true}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_parse_reports_a_malformed_source_as_a_failure_not_a_panic() {
        let v = json!({
            "operation": "parse",
            "input": {"format": "json"},
            "expect": {"ok": true}
        });
        let r = dispatch(&v);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("neither"), "{}", r.message);
    }

    #[test]
    fn run_parse_schema_reports_a_malformed_source_as_a_failure_not_a_panic() {
        let v = json!({
            "operation": "parse_schema",
            "input": {"text": "record R { a: string } root R", "bytes_hex": "31"},
            "expect": {"ok": true}
        });
        let r = dispatch(&v);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("both"), "{}", r.message);
    }

    #[test]
    fn dispatch_rejects_bytes_hex_on_an_operation_that_does_not_accept_it() {
        let v = json!({
            "operation": "validate",
            "input": {"bytes_hex": "31"},
        });
        let r = dispatch(&v);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("E-27"));
    }

    #[test]
    fn vector_count_is_273() {
        // 204 -> 249 via the submodule pin bump v0.9.1-beta -> v0.19.0-beta,
        // 249 -> 273 via v0.19.0-beta -> v0.21.0-beta (14 new bytes_hex D-14
        // vectors, 4 new OSD-15 canonical-output vectors, 5 new OML-26/27
        // vectors already counted at the v0.19.0-beta pin, 1 new infer vector).
        let vectors = iter_vectors(&suite_dir());
        assert_eq!(vectors.len(), 273);
    }

    /// Full-suite regression guard: runs every real vector through every
    /// driver (also this file's real coverage-driving test, since
    /// `main`/`main_with_dir` is process-entry-point code). The counts are
    /// freshly measured, not computed by hand.
    ///
    /// Spec v0.21.0-beta, diagnostics compared as (path, code) sets:
    /// 233 pass, 0 fail, 40 skip of 273.
    ///
    /// - the 40 skips are E-20 "not yet implemented", never a documented
    ///   divergence: 6 `document-model/limits` (no runtime-configurable
    ///   limits), 6 `formats-yaml/alias-expansion` (D-18, DIV-3), and 28
    ///   `extensions-osd-oml` (extension not implemented, omnist-rs#175).
    ///
    /// History: (170, 0, 34) at v0.9.1-beta / 204 vectors, path-only mode.
    /// At v0.19.0-beta the same code, before any change, was (197, 18, 34)
    /// path-only; switching to (path, code) mode and adopting the sweep
    /// gave (209, 0, 40) of 249. At v0.21.0-beta, before this port's own
    /// changes, the baseline was (219, 14, 40) of 273: 14 new failures from
    /// the D-14 `bytes_hex` vectors (unknown-input-field fails, per E-20) and
    /// the OSD-15 canonical-escaping vectors. Implementing E-27/D-14 (the CLI
    /// as byte-oriented entry point) and OSD-15 escaping brings it to
    /// (233, 0, 40).
    #[test]
    fn full_suite_counts_match_the_measured_baseline() {
        let (passed, failed, skipped) = run_all(&suite_dir());
        assert_eq!(
            (passed, failed, skipped),
            (233, 0, 40),
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
    fn run_parse_failure_with_no_expected_diagnostics_fails() {
        // Exact matching (E-17 rule 3): an unexpected diagnostic fails the
        // vector, so `expect.diagnostics` may not be left off a failure.
        let v = json!({"operation": "parse", "input": {"format": "oml", "text": "[[["}, "expect": {"ok": false}});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn run_parse_failure_with_the_right_path_but_the_wrong_code_fails() {
        // The false green a code-agnostic run would give: right ok, right
        // path, wrong code.
        let v = json!({
            "operation": "parse",
            "input": {"format": "oml", "text": "nan: 1\n"},
            "expect": {"ok": false, "diagnostics": [{"path": "1:4", "code": "parse.unexpected-token"}]}
        });
        let r = dispatch(&v);
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.message.contains("parse.trailing-content"),
            "{}",
            r.message
        );
    }

    #[test]
    fn run_parse_failure_with_the_right_path_and_code_passes() {
        let v = json!({
            "operation": "parse",
            "input": {"format": "oml", "text": "nan: 1\n"},
            "expect": {"ok": false, "diagnostics": [{"path": "1:4", "code": "parse.trailing-content"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_parse_non_oml_syntax_failure_with_diagnostics_matches_by_line_col_and_code() {
        // json's `error_at` produces a structured line/col and the
        // `parse.codec-syntax` code (json.rs).
        let v = json!({
            "operation": "parse",
            "input": {"format": "json", "text": "{"},
            "expect": {"ok": false, "diagnostics": [{"path": "1:2", "code": "parse.codec-syntax"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn error_diag_is_none_for_errors_with_no_structured_code() {
        // No real `read_*` function constructs a Format/Write error, and an
        // API-misuse `DocumentError` carries no code: the runner FAILS such
        // a vector (never skips it).
        let e: OmnistError = omnist::error::FormatError("x".to_string()).into();
        assert!(error_diag(&e).is_none());
        let e: OmnistError = omnist::error::DocumentError::new("$", "misuse").into();
        assert!(error_diag(&e).is_none());
    }

    #[test]
    fn parse_failure_result_with_no_structured_code_fails_not_skips() {
        let e: OmnistError = omnist::error::FormatError("x".to_string()).into();
        let r = parse_failure_result(&e, &[("$".to_string(), "document.limit.depth".to_string())]);
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn parse_failure_result_document_path_or_code_mismatch_fails() {
        let e: OmnistError =
            omnist::error::DocumentError::with_code("$", "format.dtd-forbidden", "x").into();
        let wrong_path = [("$.wrong".to_string(), "format.dtd-forbidden".to_string())];
        assert_eq!(parse_failure_result(&e, &wrong_path).status, Status::Fail);
        let wrong_code = [("$".to_string(), "format.mixed-content".to_string())];
        assert_eq!(parse_failure_result(&e, &wrong_code).status, Status::Fail);
    }

    #[test]
    fn parse_failure_result_document_path_and_code_match_passes() {
        let e: OmnistError =
            omnist::error::DocumentError::with_code("$", "format.dtd-forbidden", "x").into();
        let right = [("$".to_string(), "format.dtd-forbidden".to_string())];
        assert_eq!(parse_failure_result(&e, &right).status, Status::Pass);
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
    fn a_write_failure_with_no_structured_code_fails_the_vector() {
        // TOML cannot write a scalar-rooted Document; that WriteError has no
        // taxonomy (path, code), so the vector fails rather than passing on
        // `ok: false` alone.
        let v = json!({
            "operation": "write",
            "input": {"format": "toml", "document": {"scalar": {"kind": "integer", "value": 1}}},
            "expect": {"ok": false, "diagnostics": [{"path": "$", "code": "write.unsupported-value"}]}
        });
        let r = dispatch(&v);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("no structured"), "{}", r.message);
    }

    #[test]
    fn infer_fallbacks_that_differ_fail_the_vector() {
        let v = json!({
            "operation": "infer_with_report",
            "input": {"samples": ["x: 1\n", "x: \"a\"\n"], "allow_any": true},
            "expect": {"ok": true, "schema": "record Root {\n    \"x\": any,\n}\nroot Root\n",
                       "fallbacks": [{"location": "Root.y", "reason": "r"}]}
        });
        let r = dispatch(&v);
        assert_eq!(r.status, Status::Fail);
        assert!(r.message.contains("fallbacks differ"), "{}", r.message);
    }

    #[test]
    fn dispatch_unknown_operation_fails_it_does_not_skip() {
        let v = json!({"operation": "frobnicate"});
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    /// A skip reason is only acceptable if it is TRUE for the vector it is
    /// attached to (the TypeScript port's first pass skipped 22 vectors under
    /// a reason that did not describe them). Checks every skip in the real
    /// suite against the vector's own input/operation, and pins the three
    /// categories by name and count so a fourth cannot appear unnoticed.
    #[test]
    fn every_skip_reason_is_true_for_its_vector() {
        let (mut limits, mut alias, mut ext) = (0, 0, 0);
        for nv in iter_vectors(&suite_dir()) {
            let v = &nv.vector;
            let r = dispatch(v);
            let input = &v["input"];
            let op = v["operation"].as_str().unwrap();
            let carries_limit_key = LIMIT_KEYS.iter().any(|k| input.get(*k).is_some());
            if carries_limit_key {
                // Every declared-limit vector MUST skip -- never run against
                // the port's own default.
                assert_eq!(r.status, Status::Skip, "{}", v["name"]);
                if input.get("declared_max_alias_expansion").is_some() {
                    alias += 1;
                    assert!(
                        v["name"]
                            .as_str()
                            .unwrap()
                            .starts_with("formats-yaml/alias-expansion/")
                    );
                    assert!(r.message.contains("D-18") && r.message.contains("DIV-3"));
                    assert!(r.message.contains("omnist-rs#180"), "{}", r.message);
                } else {
                    limits += 1;
                    assert!(
                        v["name"]
                            .as_str()
                            .unwrap()
                            .starts_with("document-model/limits/")
                    );
                    let key = LIMIT_KEYS
                        .iter()
                        .find(|k| input.get(**k).is_some())
                        .unwrap();
                    assert!(r.message.contains(key), "{}", r.message);
                    assert!(r.message.contains("omnist-rs#181"), "{}", r.message);
                }
            } else if EXTENSION_OPERATIONS.contains(&op) {
                ext += 1;
                assert_eq!(r.status, Status::Skip, "{}", v["name"]);
                assert!(
                    v["name"]
                        .as_str()
                        .unwrap()
                        .starts_with("extensions-osd-oml/")
                );
                assert!(r.message.contains(op) && r.message.contains("omnist-rs#175"));
            } else {
                assert_ne!(r.status, Status::Skip, "{}: unexplained skip", v["name"]);
            }
        }
        assert_eq!((limits, alias, ext), (6, 6, 28));
    }

    #[test]
    fn dispatch_extension_operation_skips_with_a_true_reason() {
        for op in EXTENSION_OPERATIONS {
            let r = dispatch(&json!({"operation": op}));
            assert_eq!(r.status, Status::Skip);
            assert!(r.message.contains("OSD-OML extension"), "{}", r.message);
        }
    }

    #[test]
    fn main_with_dir_on_the_real_suite_returns_zero() {
        // The real vendored suite has no failing vector (skips do not count,
        // E-22), so the exit code is 0.
        assert_eq!(main_with_dir(&suite_dir()), 0);
    }

    #[test]
    fn main_with_dir_returns_one_for_any_failing_vector() {
        // A failing vector always fails the run -- there is no allowlist. The
        // recipe is a `parse` vector whose `expect.document` does not match
        // what parsing "1" actually produces.
        let tmp = std::env::temp_dir().join("vector-runner-unexpected-failure");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("basic.json"),
            r#"{"vectors": [{"name": "failing/vector", "operation": "parse", "input": {"format": "json", "text": "1"}, "expect": {"ok": true, "document": {"scalar": {"kind": "integer", "value": 2}}}}]}"#,
        )
        .unwrap();
        assert_eq!(main_with_dir(&tmp), 1);
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
    fn run_parse_success_with_matching_read_diagnostics_passes() {
        // format.attribute-dropped (Sec8.3.8, D-3): a *successful* XML
        // parse can still carry warning-severity read diagnostics.
        let v = json!({
            "operation": "parse",
            "input": {"format": "xml", "text": "<a x=\"1\"><b>hi</b></a>"},
            "expect": {
                "ok": true,
                "document": {"edges": [["a", {"edges": [["b", {"scalar": {"kind": "string", "value": "hi"}}]]}]]},
                "diagnostics": [{"path": "$.a", "code": "format.attribute-dropped"}]
            }
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_parse_success_with_mismatched_read_diagnostics_fails() {
        let v = json!({
            "operation": "parse",
            "input": {"format": "xml", "text": "<a x=\"1\"><b>hi</b></a>"},
            "expect": {
                "ok": true,
                "document": {"edges": [["a", {"edges": [["b", {"scalar": {"kind": "string", "value": "hi"}}]]}]]},
                "diagnostics": [{"path": "$.wrong"}]
            }
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
    fn run_parse_schema_invalid_expect_false_no_diagnostics_fails() {
        let v = json!({
            "operation": "parse_schema",
            "input": {"text": "not valid osd"},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
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
            "expect": {"ok": false, "diagnostics": [{"path": "$.wrong", "code": "materialize.inexact-conversion"}]}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        // A genuine failure with no `diagnostics` field: exact matching
        // (E-17 rule 3) means the unexpected diagnostic fails the vector.
        let v = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": bad_doc.clone()},
            "expect": {"ok": false}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);

        // Right path, wrong code fails; right path and code passes.
        let wrong_code = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": bad_doc.clone()},
            "expect": {"ok": false, "diagnostics": [{"path": "$.a", "code": "validate.type-mismatch"}]}
        });
        assert_eq!(dispatch(&wrong_code).status, Status::Fail);
        let right = json!({
            "operation": "materialize",
            "input": {"schema": schema, "document": bad_doc},
            "expect": {"ok": false, "diagnostics": [{"path": "$.a", "code": "materialize.inexact-conversion"}]}
        });
        assert_eq!(dispatch(&right).status, Status::Pass);
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
    fn run_write_xml_whitespace_difference_still_passes() {
        // Sec8.5.3: this port's XML writer never indents, but a vector
        // written with indentation (matching a different port's writer
        // convention) must still pass -- inter-tag whitespace isn't
        // normative.
        let v = json!({
            "operation": "write",
            "input": {
                "format": "xml",
                "document": {"edges": [["root", {"edges": [["x", {"scalar": {"kind": "string", "value": "hi"}}]]}]]}
            },
            "expect": {"ok": true, "text": "<root>\n  <x>hi</x>\n</root>\n"}
        });
        assert_eq!(dispatch(&v).status, Status::Pass);
    }

    #[test]
    fn run_write_xml_genuine_content_mismatch_still_fails() {
        // Normalizing whitespace must not mask an actual content
        // difference.
        let v = json!({
            "operation": "write",
            "input": {
                "format": "xml",
                "document": {"edges": [["root", {"edges": [["x", {"scalar": {"kind": "string", "value": "hi"}}]]}]]}
            },
            "expect": {"ok": true, "text": "<root><x>bye</x></root>"}
        });
        assert_eq!(dispatch(&v).status, Status::Fail);
    }

    #[test]
    fn normalize_xml_whitespace_collapses_inter_tag_gaps() {
        // Whitespace after any '>' is stripped, including trailing
        // whitespace at end of string -- callers already `.trim()` first
        // in practice (see `run_write`), so this is consistent with that.
        assert_eq!(
            normalize_xml_whitespace("<root>\n  <x>hi</x>\n</root>\n"),
            "<root><x>hi</x></root>"
        );
        assert_eq!(
            normalize_xml_whitespace("<root><x>hi</x></root>"),
            "<root><x>hi</x></root>"
        );
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
