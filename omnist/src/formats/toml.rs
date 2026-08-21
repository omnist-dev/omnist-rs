//! TOML codec. Ported from `~/dev/omnist/omnist/formats.py`'s
//! `read_toml`/`write_toml`/`check_toml`.
//!
//! ## Crate choice
//!
//! Like `yaml.rs` (issue #18), this module delegates raw tokenization to a
//! well-tested crate -- [`toml_edit`] -- rather than hand-rolling TOML's
//! grammar (tables, dotted keys, inline tables, array-of-tables headers,
//! four temporal literal forms, three integer radixes). `toml_edit` is used
//! **read-side only**: [`read_toml`] walks its parsed `DocumentMut` into
//! this crate's canonical [`crate::document::Value`]. The **writer is
//! hand-written** (mirroring `json.rs`/`yaml.rs`'s writers), producing TOML
//! text directly from a [`crate::document::Value`] rather than round-
//! tripping back through `toml_edit`'s own formatter -- see "Writer output
//! shape" below for why.
//!
//! Everything omnist-specific stays hand-written Rust, not delegated to the
//! crate:
//!
//! * **Null-adjustment** (`strip_nulls`) -- TOML has no `null` at all;
//!   dropping a null-valued field/array-item and recording it via
//!   [`crate::report::WriteReport`] is this module's own logic (see
//!   "No null" below).
//! * **Temporal canonicalization** (`format_datetime`) -- turning
//!   `toml_edit`'s parsed `Date`/`Time`/`Datetime` structs into this port's
//!   canonical ISO-string spelling (see "Native temporal types" below).
//! * **Integer digit cap** -- `toml_edit` itself rejects any integer
//!   literal that doesn't fit in `i64` with a generic "integer number
//!   overflowed" parse error that discards the offending literal's text;
//!   this module recovers the raw digit run from the error's byte span and
//!   re-derives the same 4300-digit-cap-vs-genuine-overflow distinction
//!   `json.rs`/`yaml.rs` already make (see "Integer digit cap" below for
//!   why this needed hand-written recovery rather than being free from the
//!   crate).
//! * **Depth guard, shape-check reuse** -- see their own sections below.
//!
//! ## No `null` (the one lossy TOML adjustment)
//!
//! Live-confirmed against `tomli_w.dumps` (this project's Python reference
//! TOML writer, via `~/dev/omnist/omnist/formats.py`'s `write_toml`):
//! writing a node containing a null-valued field **drops the field
//! entirely** (not a sentinel, not an empty string) -- `{'a': 1, 'b': None}`
//! writes as `a = 1\n`, no trace of `b`. A null *inside an array* drops
//! just that element (shifting later elements down, not leaving a hole) --
//! `{'c': [1, None, 2]}` writes as `c = [\n    1,\n    2,\n]\n`. Each drop
//! is recorded as a `null.omitted`/`Severity::Warning` adjustment (matching
//! Python's `rep.add(p, "null.omitted", "null value dropped (TOML has no
//! null)", "warning")` exactly, path-for-path), and -- confirmed live --
//! `strict=True` raises even though the severity is only `Warning`
//! (`WriteReport.__bool__`/`finish_write` only ignores severity for the
//! *is_ok* check, not for whether `strict` raises at all: `finish_write`
//! raises on *any* adjustment in strict mode, matching this crate's own
//! [`crate::report::finish_write`]).
//!
//! If stripping nulls leaves a document whose root isn't a table (object),
//! [`write_toml`] raises `WriteError` unconditionally -- **not** part of
//! the report, mirrors Python's `write_toml` raising
//! `WriteError("TOML needs a top-level table (the root must be an
//! object)")` outside of `finish_write` entirely (so it fires even when
//! `report` is supplied and even though the message never enters the
//! accumulated `WriteReport`).
//!
//! ## Integer digit cap (omnist-ts#54 / json.rs / yaml.rs precedent) --
//! **not** a natural 64-bit range check
//!
//! The issue's own framing floated "TOML integers are spec'd as 64-bit, so
//! this may be a natural range check rather than the 4300-digit-cap
//! mechanism used elsewhere" -- **live-checked against `tomllib` (Python's
//! stdlib TOML reader, which `omnist.formats.read_toml` wraps directly) and
//! found not to be the case**: `tomllib.loads("x = " + "9"*4300)` parses
//! successfully to a full-precision Python `int` (no 64-bit truncation, no
//! error) -- while `tomllib.loads("x = " + "9"*4301)` raises `ValueError:
//! Exceeds the limit (4300 digits) for integer string conversion`, the
//! *identical* CPython `int(str)`-conversion guard `read_json`'s comment
//! documents (`sys.set_int_max_str_digits`), not a TOML-spec-mandated
//! 64-bit bounds check at all -- **for decimal literals only**. Hex/octal/
//! binary literals are a genuine exception, not a false one: live-confirmed
//! `tomllib.loads("x = 0x" + "f" * 5000)` (and even 10000 `f`s) parses
//! successfully with **no error at all**, matching CPython's own documented
//! carve-out (`sys.set_int_max_str_digits` explicitly exempts power-of-two
//! bases) -- an earlier draft of this comment claimed hex/octal/binary hit
//! the identical digit-limit `ValueError`, which was wrong and has been
//! corrected. So Python's TOML integer handling is `json.rs`'s/`yaml.rs`'s
//! existing 4300-digit-cap pattern for decimal literals, and *uncapped* for
//! hex/octal/binary.
//!
//! This port does **not** replicate that decimal/non-decimal split: every
//! radix goes through the same `toml_overflow_error` recovery and the same
//! 4300-digit cap, because `toml_edit` itself enforces a strict `i64` range
//! at parse time regardless of radix (see below) -- there is no path in this
//! implementation for an oversized hex/octal/binary literal to reach
//! `Scalar::Int` uncapped the way Python's arbitrary-precision `int` does.
//! This is a **disclosed divergence from Python**, not parity: kept
//! deliberately rather than special-cased away, because (a) TOML 1.0 itself
//! specifies 64-bit signed integers and `toml_edit` enforces 64-bit bounds
//! at parse time, so an "uncapped hex" path would still fail for anything
//! over `i64::MAX`, and (b) capping the digit run uniformly preserves the
//! same superlinear-conversion DoS protection `json.rs`/`yaml.rs` apply,
//! without carving out a radix-specific exemption.
//!
//! Where this module's implementation had to diverge from `json.rs`'s
//! straight-line reuse: **`toml_edit` itself enforces a strict 64-bit
//! range** at parse time (`"9"*20` / `i64::MAX + 1` both fail with a
//! generic "integer number overflowed" `TomlError` that does not expose the
//! original digit run), which is *stricter* than Python's real behavior for
//! anything between 20 and 4300 digits. To keep this port's observable
//! integer-literal error behavior matching Python's (not `toml_edit`'s
//! internal, incidentally-stricter parse limit), `toml_overflow_error`
//! recovers the raw literal text from the failed parse's byte span
//! (`TomlError::span`) and re-derives the digit count itself, producing the
//! same two-tier message `json.rs`/`yaml.rs` give ("exceeds the 4300-digit
//! cap" vs "out of range for a 64-bit integer") instead of surfacing
//! `toml_edit`'s own message directly.
//!
//! ## Native temporal types (the opposite direction from JSON's problem)
//!
//! Unlike JSON (no temporal type at all) and like YAML (a native but looser
//! timestamp grammar), TOML has **four** first-class temporal literal forms
//! (local date, local time, local datetime, offset datetime) that are
//! *stricter*-shaped than YAML's -- `toml_edit`'s own parser already fully
//! validates calendar/clock fields (leap years, per-month day counts, valid
//! hour/minute/offset ranges: live-confirmed `2024-02-30`, `2024-13-01`,
//! `25:00:00`, `00:60:00`, and a `+25:00` offset are all **parse
//! errors**, not accepted-then-rejected-later), so [`read_toml`] does not
//! need to re-validate calendar/clock fields the way `yaml.rs`'s
//! `normalize_timestamp` must (YAML's own crate does no such validation).
//!
//! [`crate::document::Scalar`] has real `Date`/`Time`/`Datetime` variants
//! (issue #105) -- `toml_value_to_value` reads `toml_edit`'s own already
//! fully-validated `Datetime` struct's `date`/`time` presence to construct
//! the right one directly (real provenance, not a shape guess), and
//! `format_datetime` renders the canonical ISO spelling either way:
//! zero-padded, `T`-joined, a bare `Z` offset normalized to `+00:00`
//! (matching `yaml.rs`'s identical normalization -- this port's canonical
//! temporal strings never contain a literal `Z`, only a numeric offset,
//! which is what `crate::schema::is_iso_datetime`'s regex expects).
//! Fractional seconds beyond microsecond precision are **truncated, not
//! rounded**, to six digits -- live-confirmed against `tomllib`:
//! `00:32:00.9999999` (7 nines) reads as `datetime.time(0, 32, 0, 999999)`
//! (truncated, not rounded to `1000000` and carried), matching this
//! module's `nanosecond / 1000` integer-truncating conversion exactly.
//!
//! **UTC-offset preservation** (the omnist-ts#51-pattern check this issue
//! calls for): an offset datetime's numeric offset is preserved exactly in
//! the canonical string (`-07:00` stays `-07:00`across read+write), so this
//! module does not repeat the OML writer's offset-erasure bug -- confirmed
//! by this module's `round_trips_offset_datetime_preserving_negative_offset`
//! and `_positive_offset` tests.
//!
//! On the **write** side, a genuine `Scalar::Date`/`Datetime` writes as a
//! *native* TOML temporal literal (unquoted) unconditionally -- no
//! shape-check, since the variant itself is the provenance signal (issue
//! #105, the same fix issue #99 already applied to OML). An ordinary
//! `Scalar::Str` **always writes quoted**, however date-shaped its text --
//! this now matches Python's `write_toml` exactly (previously diverged:
//! Python's document model retains a real `datetime.date`/`time`/
//! `datetime` object end-to-end, so a plain `str` that merely looks like a
//! date -- live-confirmed: `tomli_w.dumps({'a': '1979-05-27'})` -- always
//! wrote as the quoted string `a = "1979-05-27"`, never a native literal;
//! this port's pre-#105 `Scalar` had no way to make that distinction, so
//! it wrote bare unconditionally whenever the text merely *looked*
//! temporal-shaped -- a real bug, confirmed live and fixed, not a
//! permitted variation). A `Scalar::Time` carrying a UTC offset is the one
//! remaining case with no native TOML spelling at all (TOML's *local
//! time* literal has no offset field) -- see `write_scalar`'s own
//! `has_offset` fallback.
//!
//! ## Depth guard reuse
//!
//! [`read_toml`] parses TOML text into a [`crate::document::Value`], then
//! builds a [`Doc`] via [`Doc::of`] -- which calls
//! `crate::document::check_write_depth` internally (see `document.rs`).
//! [`write_toml`]/[`check_toml`]/`strip_nulls` walk an already-built
//! `Doc` (via `Doc::to_grouped`), whose every node was depth-checked at
//! construction time -- exactly `json.rs`'s reasoning (not `yaml.rs`'s,
//! which pre-processes raw structures *before* `Doc::of` ever runs) --
//! there is nothing left to re-guard on the way out, so `strip_nulls` does
//! not take or check a depth parameter at all.
//!
//! `toml_edit` additionally enforces **its own, separate recursion cap**
//! while parsing -- empirically found (see this module's tests) to reject
//! TOML text nested roughly 81 levels deep (inline tables), well below this
//! crate's own 200-level `MAX_DEPTH`. This means a *read*-side test can
//! only ever observe `toml_edit`'s own `ParseError` firing first, never
//! this crate's `DocumentError` -- the depth-guard-reuse obligation is
//! instead demonstrated the way `json.rs`/`yaml.rs` already do, by building
//! an over-deep [`Value`] directly and confirming [`Doc::of`] rejects it
//! (see `deeply_nested_document_write_reuses_doc_construction_depth_guard`).
//!
//! ## Writer output shape (architecture freedom, per issue #1)
//!
//! This module always emits nested tables and table-arrays as **inline**
//! TOML (`{ k = v }` / `[ v, v ]`), never `[section]`/`[[section]]` headers.
//! This is a deliberate divergence from `tomli_w`'s (and most hand-written
//! TOML's) header-based style, chosen because it is unambiguous, needs no
//! header-nesting state machine, and is fully spec-valid TOML -- the "one
//! constraint" from issue #1 is observable *behavior* (what a round trip
//! produces), not byte-for-byte resemblance to `tomli_w`'s pretty-printing
//! choices, and inline tables/arrays parse back to an identical `Doc`
//! either way.

use crate::WriteError;
use crate::document::{Doc, Value};
use crate::error::{OmnistError, ParseError};
use crate::formats::float_fmt;
use crate::formats::int_cap::{MAX_INT_DIGITS, out_of_range_message, over_cap_message};
use crate::formats::string_escape::{TOML_ESCAPES, write_quoted};
use crate::formats::textpos::line_col_bytes;
use crate::report::{Severity, WriteReport};
use indexmap::IndexMap;
use toml_edit::{Item, TableLike};

// Same guard, same constant as `json.rs`'s/`yaml.rs`'s -- see this
// module's doc comment. Constant and message constructors now live in
// [`crate::formats::int_cap`] (issue #49).

// ============================================================== Reader

/// Parse TOML text into a [`Doc`].
///
/// TOML documents are always tables at the top level (there is no bare-
/// scalar-document form in the grammar), so unlike `read_json`/`read_yaml`
/// this never needs to special-case a non-object root on the way in.
/// Nesting past [`crate::document::MAX_DEPTH`] surfaces as
/// [`crate::error::DocumentError`] via [`Doc::of`], matching the other
/// format readers.
pub fn read_toml(text: &str) -> Result<Doc, OmnistError> {
    let parsed: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| toml_parse_error(text, &e))?;
    let value = table_like_to_value(parsed.as_table())?;
    Ok(Doc::of(&value)?)
}

/// Turns a `toml_edit` parse failure into this crate's [`ParseError`].
/// Detects the crate's generic integer-overflow message specially (see
/// this module's doc comment on the integer digit cap) and otherwise
/// reports the crate's own message at the failure's line/column.
fn toml_parse_error(text: &str, e: &toml_edit::TomlError) -> ParseError {
    // `toml_edit::TomlError::span()` is documented as optional, but
    // empirically (see this module's tests) every genuine parse failure --
    // an empty/unquoted key, an unclosed array/string, a missing `=`, an
    // integer overflow -- carries a real span; there is no reachable case
    // from parsing text (as opposed to this crate's own mutation API,
    // which this module never uses) that omits one.
    let span = e
        .span()
        .expect("toml_edit's TomlError always carries a span for a genuine text-parse failure");
    if e.message().contains("overflow") {
        return toml_overflow_error(text, span);
    }
    let (line, col) = line_col_bytes(text, span.start);
    ParseError::new(line, col, format!("invalid TOML: {}", e.message()))
}

/// Recovers the raw digit run from an integer literal `toml_edit` refused
/// to parse (its own error discards the literal's text), and re-derives
/// `json.rs`/`yaml.rs`'s exact two-tier message: over the 4300-digit cap
/// gets the security-motivated cap message; under the cap (but still not
/// representable in `i64`, `toml_edit`'s actual failure condition) gets the
/// "out of range for a 64-bit integer" message -- see this module's doc
/// comment for why this recovery is needed at all, and for why this
/// applies uniformly across radixes even though Python's own tomllib
/// leaves hex/octal/binary literals uncapped (a disclosed divergence,
/// not a parity claim).
fn toml_overflow_error(text: &str, span: std::ops::Range<usize>) -> ParseError {
    let (line, col) = line_col_bytes(text, span.start);
    let raw = &text[span];
    let digits: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let digit_count = digits
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .len();
    if digit_count > MAX_INT_DIGITS {
        return ParseError::new(line, col, over_cap_message("invalid TOML: ", digit_count));
    }
    ParseError::new(line, col, out_of_range_message("invalid TOML: ", raw))
}

/// Converts a `toml_edit` table (top-level document or inline table) into a
/// [`Value::Object`], recursing into every entry.
fn table_like_to_value(t: &dyn TableLike) -> Result<Value, ParseError> {
    let mut map = IndexMap::new();
    for (k, item) in t.iter() {
        map.insert(k.to_string(), item_to_value(item)?);
    }
    Ok(Value::Object(map))
}

fn item_to_value(item: &Item) -> Result<Value, ParseError> {
    match item {
        // `Item::None` is only ever produced by `toml_edit`'s *mutation*
        // API (`Entry`/`Index::or_insert(Item::None)`, confirmed by reading
        // the crate's source) -- never by parsing text, which is the only
        // way `read_toml` ever constructs an `Item`. White-box-tested
        // directly (see this module's tests) rather than left an
        // unreachable branch with no proof.
        Item::None => unreachable!(
            "Item::None is only produced by toml_edit's mutation API, never by parsing text"
        ),
        Item::Value(v) => toml_value_to_value(v),
        Item::Table(t) => table_like_to_value(t),
        Item::ArrayOfTables(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for t in arr.iter() {
                out.push(table_like_to_value(t)?);
            }
            Ok(Value::Array(out))
        }
    }
}

fn toml_value_to_value(v: &toml_edit::Value) -> Result<Value, ParseError> {
    match v {
        toml_edit::Value::String(s) => Ok(Value::Str(s.value().clone())),
        // `toml_edit`'s own `Integer` is `i64`-backed (the TOML 1.0 format
        // spec itself specifies 64-bit signed integers) -- a >19-digit
        // literal in TOML *source text* is rejected by `toml_edit`'s own
        // parser before this function ever runs, a genuine external
        // format-level constraint distinct from omnist's own Scalar
        // representation (issue #104; see docs/formats/toml.md).
        toml_edit::Value::Integer(i) => Ok(Value::Int((*i.value()).into())),
        toml_edit::Value::Float(f) => Ok(Value::Float(*f.value())),
        toml_edit::Value::Boolean(b) => Ok(Value::Bool(*b.value())),
        // `toml_edit` already validates and types TOML's four native
        // temporal forms itself (calendar/clock fields, `2024-02-30`
        // etc., are rejected in its own parser) -- `dt.value()`'s
        // `date`/`time` presence tells us exactly which of the three
        // kinds this is, real provenance rather than a shape guess
        // (issue #105; previously collapsed straight to `Value::Str`,
        // discarding real type information `toml_edit` had already
        // computed).
        toml_edit::Value::Datetime(dt) => {
            let canonical = format_datetime(dt.value());
            let inner = dt.value();
            Ok(match (inner.date.is_some(), inner.time.is_some()) {
                (true, true) => Value::Datetime(canonical),
                (true, false) => Value::Date(canonical),
                (false, true) => Value::Time(canonical),
                // `toml_edit`'s own grammar always sets at least one of
                // `date`/`time` -- a defensive, non-panicking fallback
                // rather than `unreachable!()`, since this is an
                // assumption about a dependency's invariant, not this
                // crate's own.
                (false, false) => Value::Str(canonical),
            })
        }
        toml_edit::Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr.iter() {
                out.push(toml_value_to_value(item)?);
            }
            Ok(Value::Array(out))
        }
        toml_edit::Value::InlineTable(it) => table_like_to_value(it),
    }
}

/// Canonicalizes a parsed `toml_edit` [`toml_edit::Datetime`] into this
/// port's ISO-string spelling -- see this module's doc comment on native
/// temporal types.
fn format_datetime(dt: &toml_edit::Datetime) -> String {
    let mut out = String::new();
    if let Some(d) = &dt.date {
        out.push_str(&format!("{:04}-{:02}-{:02}", d.year, d.month, d.day));
    }
    if let Some(t) = &dt.time {
        if dt.date.is_some() {
            out.push('T');
        }
        out.push_str(&format!(
            "{:02}:{:02}:{:02}",
            t.hour,
            t.minute,
            t.second.unwrap_or(0)
        ));
        if let Some(ns) = t.nanosecond {
            let micros = ns / 1000;
            if micros > 0 {
                out.push('.');
                out.push_str(&format!("{micros:06}"));
            }
        }
    }
    if let Some(off) = &dt.offset {
        match off {
            toml_edit::Offset::Z => out.push_str("+00:00"),
            toml_edit::Offset::Custom { minutes } => {
                let sign = if *minutes < 0 { '-' } else { '+' };
                let m = minutes.unsigned_abs();
                out.push_str(&format!("{sign}{:02}:{:02}", m / 60, m % 60));
            }
        }
    }
    out
}

// ============================================================== Writer

/// Project a [`Doc`] to TOML text. See this module's doc comment for the
/// null-adjustment, integer-cap, temporal, and output-shape decisions.
pub fn write_toml(
    doc: &Doc,
    strict: bool,
    report: Option<&mut WriteReport>,
) -> Result<String, WriteError> {
    let mut rep = WriteReport::new();
    let grouped = doc.to_grouped();
    let stripped = strip_nulls(grouped, "$", &mut rep);
    let Value::Object(map) = &stripped else {
        return Err(WriteError::new(
            "TOML needs a top-level table (the root must be an object)",
        ));
    };
    let mut out = String::new();
    write_table_body(map, &mut out);
    crate::report::finish_write(out, rep, strict, report)
}

/// Report what writing TOML would adjust, without producing output.
pub fn check_toml(doc: &Doc) -> WriteReport {
    let mut rep = WriteReport::new();
    let grouped = doc.to_grouped();
    let _ = strip_nulls(grouped, "$", &mut rep);
    rep
}

/// Marker type implementing [`crate::formats::Codec`] for TOML -- adapts
/// [`read_toml`]/[`write_toml`]/[`check_toml`] to the registry's uniform
/// shape with the documented defaults (`strict: false`, no report). The
/// root-shape error `write_toml` raises for a non-object root fires from
/// inside `write_toml` itself, outside `finish_write`, exactly as before --
/// this impl only calls `write_toml`, it doesn't reimplement it.
pub(crate) struct Toml;

impl crate::formats::Codec for Toml {
    const NAME: &'static str = "toml";

    fn read(text: &str) -> Result<Doc, OmnistError> {
        read_toml(text)
    }

    fn write(doc: &Doc) -> Result<String, OmnistError> {
        write_toml(doc, false, None).map_err(Into::into)
    }

    fn check(doc: &Doc) -> WriteReport {
        check_toml(doc)
    }
}

/// Drops every null-valued field/array-item, recording a `null.omitted`
/// warning for each (TOML has no null at all) -- see this module's doc
/// comment. Mirrors Python's `_strip_nulls` path-numbering exactly:
/// same-label array items are indexed `path.label[i]` for `i > 0`.
fn strip_nulls(node: Value, path: &str, rep: &mut WriteReport) -> Value {
    match node {
        Value::Object(map) => {
            let mut out = IndexMap::new();
            for (label, child) in map {
                match child {
                    Value::Null => {
                        rep.add(
                            crate::report::child_path(path, &label, 0),
                            "null.omitted",
                            "null value dropped (TOML has no null)",
                            Severity::Warning,
                        );
                    }
                    Value::Array(items) => {
                        let mut kept = Vec::with_capacity(items.len());
                        for (i, item) in items.into_iter().enumerate() {
                            let p = crate::report::child_path(path, &label, i);
                            if matches!(item, Value::Null) {
                                rep.add(
                                    p,
                                    "null.omitted",
                                    "null value dropped (TOML has no null)",
                                    Severity::Warning,
                                );
                            } else {
                                kept.push(strip_nulls(item, &p, rep));
                            }
                        }
                        out.insert(label, Value::Array(kept));
                    }
                    other => {
                        let p = crate::report::child_path(path, &label, 0);
                        out.insert(label, strip_nulls(other, &p, rep));
                    }
                }
            }
            Value::Object(out)
        }
        other => other,
    }
}

fn write_table_body(map: &IndexMap<String, Value>, out: &mut String) {
    for (k, v) in map {
        write_key(k, out);
        out.push_str(" = ");
        write_inline_value(v, out);
        out.push('\n');
    }
}

fn write_inline_value(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{ ");
            let mut first = true;
            for (k, child) in map {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                write_key(k, out);
                out.push_str(" = ");
                write_inline_value(child, out);
            }
            out.push_str(" }");
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            let mut first = true;
            for item in items {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                write_inline_value(item, out);
            }
            out.push(']');
        }
        scalar => write_scalar(scalar, out),
    }
}

fn write_scalar(v: &Value, out: &mut String) {
    match v {
        Value::Null => unreachable!("null values are stripped before writing (strip_nulls)"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(x) => write_float(*x, out),
        // Always quoted -- no shape-guessing (issue #105, the same fix
        // issue #99 already applied to OML). A genuinely temporal-kinded
        // value is a `Date`/`Time`/`Datetime` variant, not a
        // shape-matched `Str`; see the arms below.
        Value::Str(s) => write_toml_string(s, out),
        Value::Date(s) | Value::Datetime(s) => out.push_str(s),
        // TOML has no "local time with an offset" literal form (only
        // *date*time can carry an offset) -- a `Time` value that happens
        // to carry one (YAML's own `TIME_RE` allows it, see issue #99's
        // `bare_time_literal_with_utc_offset_reads_as_a_genuine_temporal_leaf`)
        // has no native TOML spelling, so it's written as a quoted
        // string instead, the same fallback this module already used
        // before real provenance existed -- see `has_offset`.
        Value::Time(s) => {
            if has_offset(s) {
                write_toml_string(s, out);
            } else {
                out.push_str(s);
            }
        }
        Value::Object(_) | Value::Array(_) => {
            unreachable!("write_scalar is only ever called on a leaf")
        }
    }
}

/// See `formats::float_fmt` (issue #47) for the shared render-then-inspect
/// core (issue #46's fix); this is just TOML's spelling table.
fn write_float(x: f64, out: &mut String) {
    float_fmt::write_float(x, "nan", "inf", "-inf", out);
}

/// Whether an [`is_iso_time`]-shaped string also carries a `+HH:MM`/
/// `-HH:MM` offset -- a bare TOML local-time literal has no offset, so a
/// string shaped like "time with an offset" (an unusual value that isn't a
/// real TOML literal at all) is written as a quoted string instead.
fn has_offset(s: &str) -> bool {
    s.contains('+') || s.contains('-')
}

fn write_key(k: &str, out: &mut String) {
    if is_bare_key(k) {
        out.push_str(k);
    } else {
        write_toml_string(k, out);
    }
}

fn is_bare_key(k: &str) -> bool {
    !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn write_toml_string(s: &str, out: &mut String) {
    write_quoted(s, &TOML_ESCAPES, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Scalar;

    fn doc_of(v: Value) -> Doc {
        Doc::of(&v).unwrap()
    }

    fn obj(pairs: Vec<(&str, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn toml_value_to_value_defensive_fallback_on_a_bare_offset_datetime() {
        // `toml_edit`'s own grammar never emits a `Datetime` with neither
        // `date` nor `time` set through real TOML source text, but its
        // `Datetime` struct's fields are public, so this hand-built case
        // is directly constructible -- real, tested coverage of the
        // defensive `Value::Str` fallback (issue #105), not speculative
        // dead code.
        let dt = toml_edit::Datetime {
            date: None,
            time: None,
            offset: None,
        };
        let v = toml_edit::Value::Datetime(toml_edit::Formatted::new(dt));
        assert_eq!(toml_value_to_value(&v).unwrap(), Value::Str(String::new()));
    }

    // ---------------------------------------------------------- reader: scalars

    #[test]
    fn reads_every_native_scalar_kind() {
        let doc = read_toml("a = 1\nb = \"s\"\nc = true\nd = 1.5\ne = false\n").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Int((1).into())
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Str("s".to_string())
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        assert_eq!(
            *root.get_one("d").unwrap().value().unwrap(),
            Scalar::Float(1.5)
        );
        assert_eq!(
            *root.get_one("e").unwrap().value().unwrap(),
            Scalar::Bool(false)
        );
    }

    #[test]
    fn reads_nested_tables_and_arrays() {
        let doc = read_toml("arr = [1, 2, 3]\n[nested]\nx = 1\n").unwrap();
        let root = doc.root();
        let items: Vec<_> = root.get("arr");
        assert_eq!(items.len(), 3);
        let nested = root.get_one("nested").unwrap();
        assert_eq!(
            *nested.get_one("x").unwrap().value().unwrap(),
            Scalar::Int((1).into())
        );
    }

    #[test]
    fn reads_array_of_tables() {
        let doc = read_toml("[[items]]\nx = 1\n[[items]]\nx = 2\n").unwrap();
        let root = doc.root();
        let items: Vec<_> = root.get("items");
        assert_eq!(items.len(), 2);
        assert_eq!(
            *items[0].get_one("x").unwrap().value().unwrap(),
            Scalar::Int((1).into())
        );
        assert_eq!(
            *items[1].get_one("x").unwrap().value().unwrap(),
            Scalar::Int((2).into())
        );
    }

    #[test]
    fn invalid_toml_is_a_parse_error() {
        let err = read_toml("a = ").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)));
    }

    // ------------------------------------------------------- temporal literals

    #[test]
    fn reads_local_date() {
        let doc = read_toml("d = 1979-05-27\n").unwrap();
        assert_eq!(
            *doc.root().get_one("d").unwrap().value().unwrap(),
            Scalar::Date("1979-05-27".to_string())
        );
    }

    #[test]
    fn reads_local_time_with_fraction() {
        let doc = read_toml("t = 00:32:00.999999\n").unwrap();
        assert_eq!(
            *doc.root().get_one("t").unwrap().value().unwrap(),
            Scalar::Time("00:32:00.999999".to_string())
        );
    }

    #[test]
    fn reads_local_time_without_fraction() {
        let doc = read_toml("t = 07:32:00\n").unwrap();
        assert_eq!(
            *doc.root().get_one("t").unwrap().value().unwrap(),
            Scalar::Time("07:32:00".to_string())
        );
    }

    #[test]
    fn truncates_fraction_beyond_microseconds_matching_python() {
        // Live-confirmed: tomllib truncates (not rounds) a 7-digit fraction
        // to microseconds -- 00:32:00.9999999 -> time(0, 32, 0, 999999).
        let doc = read_toml("t = 00:32:00.9999999\n").unwrap();
        assert_eq!(
            *doc.root().get_one("t").unwrap().value().unwrap(),
            Scalar::Time("00:32:00.999999".to_string())
        );
    }

    #[test]
    fn sub_microsecond_fraction_truncates_to_no_fraction() {
        // Live-confirmed: tomllib gives time(7, 32) (no fraction) for this.
        let doc = read_toml("t = 07:32:00.000000001\n").unwrap();
        assert_eq!(
            *doc.root().get_one("t").unwrap().value().unwrap(),
            Scalar::Time("07:32:00".to_string())
        );
    }

    #[test]
    fn reads_local_datetime() {
        let doc = read_toml("dt = 1979-05-27T07:32:00\n").unwrap();
        assert_eq!(
            *doc.root().get_one("dt").unwrap().value().unwrap(),
            Scalar::Datetime("1979-05-27T07:32:00".to_string())
        );
    }

    #[test]
    fn reads_offset_datetime_z_normalizes_to_numeric_offset() {
        let doc = read_toml("dt = 1979-05-27T07:32:00Z\n").unwrap();
        assert_eq!(
            *doc.root().get_one("dt").unwrap().value().unwrap(),
            Scalar::Datetime("1979-05-27T07:32:00+00:00".to_string())
        );
    }

    #[test]
    fn round_trips_offset_datetime_preserving_negative_offset() {
        let doc = read_toml("dt = 1979-05-27T07:32:00-07:00\n").unwrap();
        assert_eq!(
            *doc.root().get_one("dt").unwrap().value().unwrap(),
            Scalar::Datetime("1979-05-27T07:32:00-07:00".to_string())
        );
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "dt = 1979-05-27T07:32:00-07:00\n");
        let doc2 = read_toml(&text).unwrap();
        assert_eq!(
            *doc2.root().get_one("dt").unwrap().value().unwrap(),
            Scalar::Datetime("1979-05-27T07:32:00-07:00".to_string())
        );
    }

    #[test]
    fn round_trips_offset_datetime_preserving_positive_offset_and_fraction() {
        let src = "dt = 1979-05-27T07:32:00.999999+07:00\n";
        let doc = read_toml(src).unwrap();
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, src);
    }

    #[test]
    fn space_separated_datetime_reads_as_t_joined_canonical_string() {
        let doc = read_toml("dt = 1979-05-27 07:32:00-07:00\n").unwrap();
        assert_eq!(
            *doc.root().get_one("dt").unwrap().value().unwrap(),
            Scalar::Datetime("1979-05-27T07:32:00-07:00".to_string())
        );
    }

    // ------------------------------------------------------------ round trips

    #[test]
    fn round_trips_every_native_scalar_kind() {
        let v = obj(vec![
            ("a", Value::Int((42).into())),
            ("b", Value::Str("hello".to_string())),
            ("c", Value::Bool(true)),
            ("d", Value::Float(1.5)),
            ("e", Value::Bool(false)),
        ]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        let doc2 = read_toml(&text).unwrap();
        let root = doc2.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Int((42).into())
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Str("hello".to_string())
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        assert_eq!(
            *root.get_one("d").unwrap().value().unwrap(),
            Scalar::Float(1.5)
        );
        assert_eq!(
            *root.get_one("e").unwrap().value().unwrap(),
            Scalar::Bool(false)
        );
    }

    #[test]
    fn round_trips_integral_float_at_and_above_1e17_boundary_issue_46() {
        // Regression test for issue #46 (see json.rs's twin test for the
        // full explanation): an integral-valued float >= 1e17 used to
        // render as a bare digit run and re-read as `Scalar::Int`.
        for x in [1.0e17, 1.0e18, -1.23e17, 9.9e16_f64] {
            let doc = doc_of(obj(vec![("a", Value::Float(x))]));
            let text = write_toml(&doc, false, None).unwrap();
            let back = read_toml(&text).unwrap();
            assert_eq!(
                *back.root().get_one("a").unwrap().value().unwrap(),
                Scalar::Float(x),
                "x={x} text={text}"
            );
        }
    }

    #[test]
    fn round_trips_local_date_time_and_datetime() {
        // Genuinely temporal-kinded values (issue #105) write as TOML's
        // native literals and read back as the same real variant -- see
        // `plain_string_that_looks_like_a_date_stays_quoted` below for the
        // companion case (a plain string merely shaped like one of these).
        let v = obj(vec![
            ("d", Value::Date("1979-05-27".to_string())),
            ("t", Value::Time("07:32:00".to_string())),
            ("dt", Value::Datetime("1979-05-27T07:32:00".to_string())),
        ]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert!(text.contains("d = 1979-05-27\n"));
        assert!(text.contains("t = 07:32:00\n"));
        assert!(text.contains("dt = 1979-05-27T07:32:00\n"));
        let doc2 = read_toml(&text).unwrap();
        let root = doc2.root();
        assert_eq!(
            *root.get_one("d").unwrap().value().unwrap(),
            Scalar::Date("1979-05-27".to_string())
        );
        assert_eq!(
            *root.get_one("t").unwrap().value().unwrap(),
            Scalar::Time("07:32:00".to_string())
        );
        assert_eq!(
            *root.get_one("dt").unwrap().value().unwrap(),
            Scalar::Datetime("1979-05-27T07:32:00".to_string())
        );
    }

    #[test]
    fn a_genuine_time_value_carrying_an_offset_writes_as_a_quoted_string() {
        // TOML has no "local time with an offset" literal (only *date*time
        // can carry one) -- a real `Value::Time` that happens to carry a
        // UTC offset (OML's own time grammar allows this) has no native
        // TOML spelling, so `write_scalar` falls back to a quoted string
        // (see `has_offset` and the `Value::Time` write arm).
        let v = obj(vec![("t", Value::Time("07:32:00+02:00".to_string()))]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert!(text.contains("t = \"07:32:00+02:00\"\n"));
    }

    #[test]
    fn round_trips_nested_table_and_array() {
        let v = obj(vec![
            (
                "nested",
                obj(vec![
                    ("x", Value::Int((1).into())),
                    ("y", Value::Str("z".to_string())),
                ]),
            ),
            (
                "arr",
                Value::Array(vec![
                    Value::Int((1).into()),
                    Value::Int((2).into()),
                    Value::Int((3).into()),
                ]),
            ),
        ]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        let doc2 = read_toml(&text).unwrap();
        let root = doc2.root();
        let nested = root.get_one("nested").unwrap();
        assert_eq!(
            *nested.get_one("x").unwrap().value().unwrap(),
            Scalar::Int((1).into())
        );
        assert_eq!(
            *nested.get_one("y").unwrap().value().unwrap(),
            Scalar::Str("z".to_string())
        );
        assert_eq!(root.get("arr").len(), 3);
    }

    #[test]
    fn round_trips_array_of_tables() {
        let v = obj(vec![(
            "items",
            Value::Array(vec![
                obj(vec![("x", Value::Int((1).into()))]),
                obj(vec![("x", Value::Int((2).into()))]),
            ]),
        )]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        let doc2 = read_toml(&text).unwrap();
        let items: Vec<_> = doc2.root().get("items");
        assert_eq!(items.len(), 2);
        assert_eq!(
            *items[0].get_one("x").unwrap().value().unwrap(),
            Scalar::Int((1).into())
        );
        assert_eq!(
            *items[1].get_one("x").unwrap().value().unwrap(),
            Scalar::Int((2).into())
        );
    }

    #[test]
    fn round_trips_nan_and_infinity_natively() {
        let v = obj(vec![
            ("a", Value::Float(f64::NAN)),
            ("b", Value::Float(f64::INFINITY)),
            ("c", Value::Float(f64::NEG_INFINITY)),
        ]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert!(text.contains("a = nan\n"));
        assert!(text.contains("b = inf\n"));
        assert!(text.contains("c = -inf\n"));
        let doc2 = read_toml(&text).unwrap();
        let root = doc2.root();
        assert!(matches!(
            root.get_one("a").unwrap().value().unwrap(),
            Scalar::Float(x) if x.is_nan()
        ));
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Float(f64::INFINITY)
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Float(f64::NEG_INFINITY)
        );
        // no adjustment needed -- TOML holds special floats natively.
        assert!(check_toml(&doc).is_empty());
    }

    // ------------------------------------------------------------ null adjustment

    #[test]
    fn lenient_write_drops_null_field_and_records_adjustment() {
        let v = obj(vec![("a", Value::Int((1).into())), ("b", Value::Null)]);
        let doc = doc_of(v);
        let mut rep = WriteReport::new();
        let text = write_toml(&doc, false, Some(&mut rep)).unwrap();
        assert!(!text.contains('b'));
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].path, "$.b");
        assert_eq!(rep.adjustments()[0].code, "null.omitted");
        assert!(rep.is_ok(), "null.omitted is only a Warning");
    }

    #[test]
    fn lenient_write_drops_null_array_item_shifting_index() {
        let v = obj(vec![(
            "c",
            Value::Array(vec![
                Value::Int((1).into()),
                Value::Null,
                Value::Int((2).into()),
            ]),
        )]);
        let doc = doc_of(v);
        let mut rep = WriteReport::new();
        let text = write_toml(&doc, false, Some(&mut rep)).unwrap();
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].path, "$.c[1]");
        let doc2 = read_toml(&text).unwrap();
        assert_eq!(doc2.root().get("c").len(), 2);
    }

    #[test]
    fn null_in_nested_table_records_nested_path() {
        let v = obj(vec![(
            "nested",
            obj(vec![("x", Value::Null), ("y", Value::Int((5).into()))]),
        )]);
        let doc = doc_of(v);
        let rep = check_toml(&doc);
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].path, "$.nested.x");
    }

    #[test]
    fn strict_write_raises_on_null_even_though_severity_is_warning() {
        let v = obj(vec![("a", Value::Int((1).into())), ("b", Value::Null)]);
        let doc = doc_of(v);
        let err = write_toml(&doc, true, None).unwrap_err();
        assert!(err.to_string().contains("$.b"));
        assert_eq!(err.report().unwrap().len(), 1);
    }

    #[test]
    fn check_toml_reports_without_producing_output() {
        let v = obj(vec![("a", Value::Null)]);
        let doc = doc_of(v);
        let rep = check_toml(&doc);
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].code, "null.omitted");
    }

    // ------------------------------------------------------------ top-level shape

    #[test]
    fn non_table_root_is_a_write_error() {
        let doc = doc_of(Value::Int((5).into()));
        let err = write_toml(&doc, false, None).unwrap_err();
        assert!(err.to_string().contains("top-level table"));
        assert!(err.report().is_none());
    }

    #[test]
    fn non_table_root_is_a_write_error_even_with_a_report_supplied() {
        let doc = doc_of(Value::Int((5).into()));
        let mut rep = WriteReport::new();
        let err = write_toml(&doc, false, Some(&mut rep)).unwrap_err();
        assert!(err.to_string().contains("top-level table"));
        assert!(rep.is_empty());
    }

    #[test]
    fn empty_object_root_writes_empty_text() {
        let doc = doc_of(Value::Object(IndexMap::new()));
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "");
    }

    // ------------------------------------------------------------ integer cap

    #[test]
    fn integer_at_4300_digits_reads_but_overflows_i64() {
        // Live-confirmed: tomllib itself accepts a 4300-digit literal (no
        // digit-cap error) but our Scalar::Int is i64 -- so this port's
        // read still fails, just with the "out of range" message, not the
        // "digit cap exceeded" one, matching json.rs's own precedent for
        // the same representational gap.
        let text = format!("x = {}\n", "9".repeat(4300));
        let err = read_toml(&text).unwrap_err();
        assert!(matches!(err, OmnistError::Parse(ref e) if e.message.contains("out of range")));
    }

    #[test]
    fn huge_hex_literal_is_capped_unlike_pythons_uncapped_tomllib() {
        // Live-confirmed divergence: tomllib.loads("x = 0x" + "f"*5000)
        // parses successfully in Python with no error at all (CPython's
        // digit cap explicitly exempts power-of-two bases). This port does
        // not replicate that exemption -- toml_edit enforces a strict i64
        // range at parse time regardless of radix, so this hits the same
        // digit-cap recovery path as a decimal literal and is rejected. See
        // this module's doc comment for why that's a disclosed, deliberate
        // divergence rather than a parity claim.
        let text = format!("x = 0x{}\n", "f".repeat(5000));
        let err = read_toml(&text).unwrap_err();
        assert!(matches!(
            err,
            OmnistError::Parse(ref e) if e.message.contains("exceeding the 4300-digit limit")
        ));
    }

    #[test]
    fn integer_over_4300_digits_is_the_digit_cap_error() {
        let text = format!("x = {}\n", "9".repeat(4301));
        let err = read_toml(&text).unwrap_err();
        assert!(matches!(
            err,
            OmnistError::Parse(ref e) if e.message.contains("exceeding the 4300-digit limit")
        ));
    }

    #[test]
    fn integer_literal_under_digit_cap_but_over_i64_range_is_out_of_range_error() {
        // Live-confirmed: tomllib.loads("x = 9223372036854775808") parses
        // fine in Python (arbitrary-precision int, no error at all -- this
        // is well under the 4300-digit cap). This port's Scalar::Int is
        // i64-only, so the same literal is rejected here as out of range --
        // a disclosed representational limit of this Rust port, not parity
        // with Python's real behavior, matching json.rs's own precedent for
        // the same representational gap.
        let text = "x = 9223372036854775808\n"; // i64::MAX + 1, 19 digits
        let err = read_toml(text).unwrap_err();
        assert!(matches!(
            err,
            OmnistError::Parse(ref e) if e.message.contains("out of range for a 64-bit integer")
        ));
    }

    #[test]
    fn integer_at_i64_max_and_min_round_trip() {
        let text = format!("a = {}\nb = {}\n", i64::MAX, i64::MIN);
        let doc = read_toml(&text).unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Int((i64::MAX).into())
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Int((i64::MIN).into())
        );
    }

    // ------------------------------------------------------------ depth guard

    #[test]
    fn toml_edit_s_own_recursion_cap_fires_before_our_200_depth_guard_on_read() {
        // Empirically found (see this module's doc comment): toml_edit has
        // its own internal recursion cap around 80-81 levels of nested
        // inline tables -- well below this crate's own MAX_DEPTH (200) --
        // so deeply-nested TOML *text* trips the crate's own ParseError
        // long before our own DocumentError guard could ever see it. This
        // is a real, distinct protection layer from this crate's own depth
        // guard, not the same one -- see the next test for the guard this
        // module actually reuses.
        let mut text = String::from("x = ");
        for _ in 0..250 {
            text.push_str("{ a = ");
        }
        text.push('1');
        for _ in 0..250 {
            text.push_str(" }");
        }
        text.push('\n');
        let err = read_toml(&text).unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)));
    }

    #[test]
    fn deeply_nested_document_write_reuses_doc_construction_depth_guard() {
        // Doc::of already rejects nesting past MAX_DEPTH at construction
        // time (see this module's doc comment) -- confirms write_toml/
        // check_toml never even see an over-deep Doc to begin with, exactly
        // json.rs's/yaml.rs's own precedent test for this same guard.
        let mut v = Value::Int((0).into());
        for _ in 0..=crate::document::MAX_DEPTH {
            v = obj(vec![("a", v)]);
        }
        assert!(Doc::of(&v).is_err());
    }

    // ------------------------------------------------------------ keys

    #[test]
    fn writes_quoted_key_for_non_bare_label() {
        let v = obj(vec![("has space", Value::Int((1).into()))]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "\"has space\" = 1\n");
        let doc2 = read_toml(&text).unwrap();
        assert_eq!(
            *doc2.root().get_one("has space").unwrap().value().unwrap(),
            Scalar::Int((1).into())
        );
    }

    #[test]
    fn string_with_control_char_and_quote_escapes_on_write() {
        let v = obj(vec![(
            "a",
            Value::Str("line\nbreak \"q\" \t tab".to_string()),
        )]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        let doc2 = read_toml(&text).unwrap();
        assert_eq!(
            *doc2.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("line\nbreak \"q\" \t tab".to_string())
        );
    }

    #[test]
    fn time_shaped_string_with_offset_is_not_a_real_toml_time_and_stays_quoted() {
        // "07:32:00+01:00" matches is_iso_time's shape (offset is optional
        // in that regex) but isn't a real TOML local-time literal (which
        // has no offset) -- so this module writes it quoted, not as a
        // (invalid) bare local-time-with-offset token.
        let v = obj(vec![("a", Value::Str("07:32:00+01:00".to_string()))]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "a = \"07:32:00+01:00\"\n");
    }

    #[test]
    fn plain_string_that_looks_like_a_date_stays_quoted() {
        // Issue #105 (the same fix issue #99 already applied to OML): a
        // plain string that merely *looks* date-shaped must stay quoted
        // on write -- writing it bare would silently promote it to a
        // genuine TOML native date literal on the next read (a different
        // Document). Previously diverged from Python here (which always
        // kept it a quoted string, since Python's document model
        // distinguishes a real `datetime.date` from a `str` at runtime);
        // now matches.
        let v = obj(vec![("a", Value::Str("1979-05-27".to_string()))]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "a = \"1979-05-27\"\n");
        let doc2 = read_toml(&text).unwrap();
        assert_eq!(
            *doc2.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("1979-05-27".to_string())
        );
    }

    #[test]
    fn float_integral_value_still_gets_a_decimal_point() {
        let v = obj(vec![("a", Value::Float(1.0))]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "a = 1.0\n");
    }

    #[test]
    fn float_non_integral_value_writes_default_repr() {
        let v = obj(vec![("a", Value::Float(1.25))]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "a = 1.25\n");
    }

    // ------------------------------------------------------- coverage: white-box

    #[test]
    fn write_scalar_panics_on_null() {
        // `write_scalar` is only ever called on a leaf that has already
        // been through `strip_nulls` via the public `write_toml` path --
        // white-box confirming that documented invariant directly, same
        // rationale as yaml.rs's identical precedent test.
        let result = std::panic::catch_unwind(|| {
            let mut out = String::new();
            write_scalar(&Value::Null, &mut out);
        });
        assert!(result.is_err());
    }

    #[test]
    fn write_scalar_panics_on_a_non_leaf_value() {
        let result = std::panic::catch_unwind(|| {
            let mut out = String::new();
            write_scalar(&Value::Object(IndexMap::new()), &mut out);
        });
        assert!(result.is_err());
    }

    #[test]
    fn item_to_value_panics_on_item_none() {
        // Item::None is only produced by toml_edit's mutation API (see this
        // module's doc comment) -- read_toml never constructs one, so this
        // white-box-tests the documented invariant directly.
        let result = std::panic::catch_unwind(|| {
            let _ = item_to_value(&Item::None);
        });
        assert!(result.is_err());
    }

    #[test]
    fn nested_empty_table_writes_as_inline_empty_table() {
        let v = obj(vec![("t", Value::Object(IndexMap::new()))]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(text, "t = {}\n");
    }

    #[test]
    fn write_inline_value_on_a_bare_empty_array_writes_the_empty_token() {
        // A zero-item array can never come from a real Doc -- the Document
        // model represents "array" as repeated same-label edges, so an
        // empty array has *zero* edges and the field disappears entirely
        // at `Doc::of` construction time (never round-trips back as an
        // empty-array `Value`). White-box exercising `write_inline_value`'s
        // empty-array arm directly, same rationale and pattern as yaml.rs's
        // `write_node_on_a_bare_empty_array_writes_the_flow_empty_token`.
        let mut out = String::new();
        write_inline_value(&Value::Array(vec![]), &mut out);
        assert_eq!(out, "[]");
    }

    #[test]
    fn line_col_reports_line_two_for_an_error_after_a_newline() {
        // Forces line_col's newline-counting branch and its `Some(i)`
        // column-offset arm, neither reachable from any single-line error.
        let err = read_toml("a = 1\nb = \n").unwrap_err();
        assert!(
            matches!(err, OmnistError::Parse(ref e) if e.line == 2),
            "got {err:?}"
        );
    }

    #[test]
    fn write_toml_string_escapes_every_control_char_form() {
        let v = obj(vec![(
            "a",
            Value::Str("back\\slash cr\r back\u{08}space form\u{0c}feed ctl\u{01}".to_string()),
        )]);
        let doc = doc_of(v);
        let text = write_toml(&doc, false, None).unwrap();
        assert_eq!(
            text,
            "a = \"back\\\\slash cr\\r back\\bspace form\\ffeed ctl\\u0001\"\n"
        );
        let doc2 = read_toml(&text).unwrap();
        assert_eq!(
            *doc2.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("back\\slash cr\r back\u{08}space form\u{0c}feed ctl\u{01}".to_string())
        );
    }
}
