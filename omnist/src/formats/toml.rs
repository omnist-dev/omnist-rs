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
//! * **Big integers and overflowing floats** -- `toml_edit` holds an
//!   integer in 64 bits; this module reads the literals it refuses itself
//!   (see "Integers beyond 64 bits" below).
//! * **Depth guard, shape-check reuse** -- see their own sections below.
//!
//! ## No `null` (the one unrepresentable-value case TOML has)
//!
//! Spec Sec8.3.8/Sec8.3.9 (updated 2026-08-24): writing a node containing a
//! null-valued field now fails the write **unconditionally**
//! (`write.unsupported-value`, via [`crate::report::unsupported_value_error`])
//! regardless of `strict`, rather than the old "drop the field and record a
//! warning" behavior. TOML has no null token at all -- dropping the edge
//! didn't just alter what's represented at that position, it erased the
//! edge's existence entirely, with zero trace on read-back that a labeled
//! edge with that path was ever there (confirmed live; retired the
//! `null.omitted` code -- see `strip_nulls`'s doc comment). `check_toml`
//! still reports the same condition as a `write.unsupported-value`
//! `Severity::Error` adjustment for preview purposes, since it never
//! produces output to begin with.
//!
//! If stripping nulls leaves a document whose root isn't a table (object),
//! [`write_toml`] raises `WriteError` unconditionally -- **not** part of
//! the report, mirrors Python's `write_toml` raising
//! `WriteError("TOML needs a top-level table (the root must be an
//! object)")` outside of `finish_write` entirely (so it fires even when
//! `report` is supplied and even though the message never enters the
//! accumulated `WriteReport`).
//!
//! ## Integers beyond 64 bits (D-2, D-28) and overflowing floats (D-29)
//!
//! A TOML integer is arbitrary precision here, in every radix, and the digit
//! limit counts the decimal digits of its value. `toml_edit` holds an
//! integer in `i64` and refuses a longer literal with an "overflow" error
//! that carries a byte span but no value. [`read_toml`] therefore reads such
//! a literal itself (`recover_number`): sign, radix prefix, underscores and
//! all. It replaces the literal in a copy of the text by a stand-in of the
//! same width (`0`, or `0.0` for a float), parses again, and swaps the real
//! value back in by byte offset when the `toml_edit` tree is converted. The
//! copy keeps every other offset in place, so a syntax error anywhere in the
//! text is still reported at its own position and still wins over a limit.
//!
//! A decimal literal already longer than the limit in its own spelling is not
//! converted to a number: the shared [`IntGuard`] placeholder (`10^max`) goes
//! in instead, so the Document constructor refuses it with
//! `document.limit.int-digits` at the right path without the reader paying
//! the superlinear digit-string conversion. Every other literal is converted
//! exactly and checked by value (so `0xFFF` is four digits, 4095).
//!
//! A float literal that overflows `f64` (`1e999`) is refused by `toml_edit`
//! the same way; it reads as an infinity (`str::parse::<f64>` gives one).
//! This is a disclosed difference from Python only in that Python's
//! `tomllib` exempts hex/octal/binary literals from CPython's 4,300-digit
//! string-conversion guard, while this port applies D-28 to every radix.
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
use crate::error::DocumentError;
use crate::error::{OmnistError, ParseError};
use crate::formats::float_fmt;
use crate::formats::string_escape::{TOML_ESCAPES, write_quoted};
use crate::formats::textpos::line_col_bytes;
use crate::limits::{IntGuard, Limits, Resolved};
use crate::report::{Severity, WriteReport};
use indexmap::IndexMap;
use num_bigint::BigInt;
use std::collections::HashMap;
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
    read_toml_resolved(text, Resolved::DEFAULT)
}

/// [`read_toml`] under explicit [`Limits`].
///
/// Returns the [`Limits::validate`] error unchanged when the limits are out
/// of range. A limit crossed is a [`crate::error::DocumentError`]
/// (`document.limit.depth` / `document.limit.nodes` at `$`,
/// `document.limit.int-digits` at the path of the over-long integer), never
/// a [`ParseError`].
///
/// TOML text is parsed by `toml_edit`, which enforces a recursion cap of its
/// own (about 80 levels of inline-table nesting) and holds every integer in
/// 64 bits, so a `max_depth` above that cap, or a `max_int_digits` above 19,
/// does not make this reader accept anything `toml_edit` refuses.
pub fn read_toml_with(text: &str, limits: &Limits) -> Result<Doc, OmnistError> {
    read_toml_resolved(text, limits.resolve()?)
}

fn read_toml_resolved(text: &str, limits: Resolved) -> Result<Doc, OmnistError> {
    // D-23: first, on the input as received (BOM counted).
    limits.check_input_size(text.len())?;
    // D-15/D-21: one leading BOM is stripped, a second is rejected at 1:1.
    // toml_edit would reject the second on its own grammar; the pre-check
    // makes that uniform and puts the position/code where the spec says.
    let text = crate::bom::strip_leading_bom(text)
        .map_err(|_| ParseError::codec_syntax(1, 1, crate::bom::DOUBLED_BOM_MESSAGE))?;
    let mut guard = IntGuard::new(limits.max_int_digits);
    let mut lexemes: HashMap<usize, Value> = HashMap::new();
    let mut patched = text.to_string();
    // `toml_edit` holds an integer in 64 bits and refuses a number beyond that
    // (or a float that overflows), with no value to read. Each such literal is
    // read here instead (D-2, D-28, D-29), all in one scan, and stood in for by
    // a small one of the same width, so every other offset keeps its place; the
    // real value is swapped back in by offset when the tree is converted. One
    // scan, not one reparse per literal, keeps a file full of them linear.
    for (start, end) in prescan_numbers(text) {
        if let Some(value) = number_value(&text[start..end], &mut guard) {
            blank(&mut patched, start, end, &value);
            lexemes.insert(start, value);
        }
    }
    let parsed = toml_edit::Document::parse(patched.as_str())
        .map_err(|e| toml_parse_error(&patched, &e, &limits))?;
    let value = table_like_to_value(parsed.as_table(), &lexemes)?;
    Ok(Doc::of_resolved(&value, limits).map_err(|e| limits.whole_document(e))?)
}

/// Turns a `toml_edit` parse failure into this crate's [`ParseError`],
/// reporting the crate's own message at the failure's line/column.
fn toml_parse_error(text: &str, e: &toml_edit::TomlError, limits: &Resolved) -> OmnistError {
    // omnist-rs#197: `toml_edit`'s own recursion cap (about 80 levels) is
    // nesting past a bound, so it is `document.limit.depth` at `$`. Its error
    // carries a span for an inline table or array but NOT for a table header
    // (`[a.a.a. ...]`), so this check comes before anything reads the span.
    // Two spellings: "cannot recurse further; max recursion depth met" (with
    // a span) and "recursion limit" (a table header, without one).
    if ["max recursion depth", "recursion limit"]
        .iter()
        .any(|m| e.message().contains(m))
    {
        return toml_recursion_error(limits).into();
    }
    // `TomlError::span()` is optional. Every other genuine parse failure
    // seen carries one, but a missing one must never panic the reader on
    // hostile input (that was #197): fall back to the start of the input.
    let span = e.span().unwrap_or(0..0);
    let (line, col) = line_col_bytes(text, span.start);
    // A number literal `toml_edit` refuses as an overflow and this module's
    // scan did not take (a bare radix prefix such as `0x`, which has no
    // digits) keeps the message it had before 0.10.0-alpha: it names the
    // literal, not the library's generic wording.
    if e.message().contains("overflow")
        && let Some(literal) = text.get(span.clone()).filter(|l| !l.is_empty())
    {
        return ParseError::codec_syntax(
            line,
            col,
            format!(
                "invalid TOML: integer literal {literal:?} is out of range for a 64-bit integer"
            ),
        )
        .into();
    }
    ParseError::codec_syntax(line, col, format!("invalid TOML: {}", e.message())).into()
}

/// The `document.limit.depth` error for `toml_edit`'s own recursion cap. The
/// configured maximum depth may be larger (200 by default): the parser
/// library refuses first, so the message names it rather than claiming the
/// configured maximum was reached.
fn toml_recursion_error(limits: &Resolved) -> DocumentError {
    DocumentError::with_code(
        "$",
        "document.limit.depth",
        format!(
            "nesting exceeds toml_edit's own recursion limit (about 80 levels), which applies \
             below the configured maximum depth ({})",
            limits.max_depth
        ),
    )
}

fn is_number_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.')
}

/// Replaces `start..end` of `patched` by a same-width stand-in for `value`
/// (`0.0` for a float, `0` for an integer, padded with spaces).
fn blank(patched: &mut String, start: usize, end: usize, value: &Value) {
    let placeholder = if matches!(value, Value::Float(_)) {
        "0.0"
    } else {
        "0"
    };
    let mut filler = String::from(placeholder);
    filler.push_str(&" ".repeat(end - start - placeholder.len()));
    patched.replace_range(start..end, &filler);
}

/// The value of a number literal `toml_edit` cannot hold (D-2, D-28, D-29): an integer past 64
/// bits in any radix, or a float whose magnitude overflows. `None` for
/// anything else, including a malformed literal.
fn number_value(lexeme: &str, guard: &mut IntGuard) -> Option<Value> {
    let cleaned: String = lexeme.chars().filter(|&c| c != '_').collect();
    let (neg, body) = match cleaned.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, cleaned.strip_prefix('+').unwrap_or(&cleaned)),
    };
    let radix = match body.get(..2) {
        Some("0x") => 16,
        Some("0o") => 8,
        Some("0b") => 2,
        _ => 10,
    };
    if radix == 10 && body.contains(['.', 'e', 'E']) {
        return Some(Value::Float(cleaned.parse().ok()?));
    }
    let digits = if radix == 10 { body } else { &body[2..] };
    let magnitude = if radix == 10 && digits.len() > guard.max() {
        guard.placeholder()
    } else {
        BigInt::parse_bytes(digits.as_bytes(), radix)?
    };
    Some(Value::Int(if neg { -magnitude } else { magnitude }))
}

static NUMBER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(concat!(
        r"^(?:[+-]?(?:0|[1-9](?:_?[0-9])*)",
        r"|0x[0-9A-Fa-f](?:_?[0-9A-Fa-f])*",
        r"|0o[0-7](?:_?[0-7])*",
        r"|0b[01](?:_?[01])*",
        r"|[+-]?(?:0|[1-9](?:_?[0-9])*)(?:\.[0-9](?:_?[0-9])*)?(?:[eE][+-]?[0-9](?:_?[0-9])*)?)$"
    ))
    .unwrap()
});

/// The byte ranges of the value-position number literals in `text` that
/// `toml_edit` would refuse, found in one scan: a well-formed integer long
/// enough to leave 64 bits, or a well-formed float that overflows to an
/// infinity. Strings and comments are skipped, and so are keys (a token
/// before `=` or `.`, anything outside a value). A well-formed literal it
/// takes has the same value `toml_edit` would have given it.
fn prescan_numbers(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut depth, mut value) = (0, 0usize, false);
    while i < b.len() {
        match b[i] {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            q @ (b'"' | b'\'') => {
                let multi = b[i..].starts_with(&[q, q, q]);
                i += if multi { 3 } else { 1 };
                while i < b.len() {
                    if q == b'"' && b[i] == b'\\' {
                        i += 2;
                    } else if multi && b[i..].starts_with(&[q, q, q]) {
                        i += 3;
                        break;
                    } else if !multi && (b[i] == q || b[i] == b'\n') {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            b'=' if depth == 0 => value = true,
            b'[' | b'{' if value => depth += 1,
            b']' | b'}' if value => depth = depth.saturating_sub(1),
            b'\n' if depth == 0 => value = false,
            c if value && is_number_char(char::from(c)) => {
                let start = i;
                while i < b.len() && is_number_char(char::from(b[i])) {
                    i += 1;
                }
                let lexeme = &text[start..i];
                let after = text[i..].trim_start_matches([' ', '\t']);
                let is_key = after.starts_with('=');
                if !is_key && NUMBER_RE.is_match(lexeme) {
                    let prefixed = ["0x", "0o", "0b"].iter().any(|p| lexeme.starts_with(p));
                    let float = !prefixed && lexeme.contains(['.', 'e', 'E']);
                    let take = if float {
                        lexeme
                            .replace('_', "")
                            .parse::<f64>()
                            .is_ok_and(f64::is_infinite)
                    } else {
                        lexeme.len() >= 18
                    };
                    if take {
                        out.push((start, i));
                    }
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Converts a `toml_edit` table (top-level document or inline table) into a
/// [`Value::Object`], recursing into every entry.
fn table_like_to_value(
    t: &dyn TableLike,
    lexemes: &HashMap<usize, Value>,
) -> Result<Value, ParseError> {
    let mut map = IndexMap::new();
    for (k, item) in t.iter() {
        map.insert(k.to_string(), item_to_value(item, lexemes)?);
    }
    Ok(Value::Object(map))
}

fn item_to_value(item: &Item, lexemes: &HashMap<usize, Value>) -> Result<Value, ParseError> {
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
        Item::Value(v) => toml_value_to_value(v, lexemes),
        Item::Table(t) => table_like_to_value(t, lexemes),
        Item::ArrayOfTables(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for t in arr.iter() {
                out.push(table_like_to_value(t, lexemes)?);
            }
            Ok(Value::Array(out))
        }
    }
}

fn toml_value_to_value(
    v: &toml_edit::Value,
    lexemes: &HashMap<usize, Value>,
) -> Result<Value, ParseError> {
    // A number `toml_edit` refused (see `read_toml_resolved`) was read by
    // this module and stood in for; its value is found again by offset.
    if let Some(read) = v.span().and_then(|s| lexemes.get(&s.start)) {
        return Ok(read.clone());
    }
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
                out.push(toml_value_to_value(item, lexemes)?);
            }
            Ok(Value::Array(out))
        }
        toml_edit::Value::InlineTable(it) => table_like_to_value(it, lexemes),
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
    add_interleaving_diagnostic(doc, &mut rep);
    let grouped = doc.to_grouped();
    let stripped = strip_nulls(grouped, "$")?;
    let Value::Object(map) = &stripped else {
        return Err(WriteError::new(
            "TOML needs a top-level table (the root must be an object)",
        ));
    };
    check_toml_grouped(&stripped, "$", &mut rep);
    let mut out = String::new();
    write_table_body(map, &mut out);
    crate::report::finish_write(out, rep, strict, report)
}

/// `format.interleaving-lost` (spec Sec8.3.8, D-3) is a whole-document
/// diagnostic that depends on the original `Doc`'s edge order, lost by the
/// time `to_grouped` runs -- so it is detected separately via
/// `Doc::has_interleaving_loss` rather than folded into the grouped-`Value`
/// scanners below.
fn add_interleaving_diagnostic(doc: &Doc, rep: &mut WriteReport) {
    if doc.has_interleaving_loss() {
        rep.add(
            "$",
            "format.interleaving-lost",
            "cross-label interleaving could not be written; same-label edges were grouped",
            Severity::Warning,
        );
    }
}

/// Report what writing TOML would adjust, without producing output.
pub fn check_toml(doc: &Doc) -> WriteReport {
    let mut rep = WriteReport::new();
    add_interleaving_diagnostic(doc, &mut rep);
    let grouped = doc.to_grouped();
    check_toml_grouped(&grouped, "$", &mut rep);
    rep
}

fn check_toml_grouped(node: &Value, path: &str, rep: &mut WriteReport) {
    match node {
        Value::Object(map) => {
            for (label, child) in map {
                match child {
                    Value::Null => {
                        rep.add(
                            crate::report::child_path(path, label, None),
                            "write.unsupported-value",
                            "null value has no TOML representation (TOML has no null token)",
                            Severity::Error,
                        );
                    }
                    Value::Array(items) => {
                        for (i, item) in items.iter().enumerate() {
                            let p = crate::report::child_path(
                                path,
                                label,
                                crate::report::occurrence_index(i, items.len()),
                            );
                            if matches!(item, Value::Null) {
                                rep.add(
                                    p,
                                    "write.unsupported-value",
                                    "null value has no TOML representation (TOML has no null token)",
                                    Severity::Error,
                                );
                            } else {
                                check_toml_grouped(item, &p, rep);
                            }
                        }
                    }
                    other => {
                        let p = crate::report::child_path(path, label, None);
                        check_toml_grouped(other, &p, rep);
                    }
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let p = crate::report::child_path(
                    path,
                    "",
                    crate::report::occurrence_index(i, items.len()),
                );
                if matches!(item, Value::Null) {
                    rep.add(
                        p,
                        "write.unsupported-value",
                        "null value has no TOML representation (TOML has no null token)",
                        Severity::Error,
                    );
                } else {
                    check_toml_grouped(item, &p, rep);
                }
            }
        }
        // E-37 rule 4: TOML has no offset time, so a `time` with an offset is
        // written as a double-quoted string.
        Value::Time(s) if has_offset(s) => {
            rep.add(
                path,
                "format.temporal-stringified",
                "time with an offset has no native TOML literal; wrote as a quoted ISO-8601 string",
                Severity::Warning,
            );
        }
        _ => {}
    }
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

/// Fails the write unconditionally (`write.unsupported-value`, spec
/// Sec8.3.8/Sec8.3.9 updated 2026-08-24) the moment a null-valued field or
/// array item is found -- TOML has no null token at all, and the old
/// "drop the edge and warn" behavior erased the edge's existence entirely
/// with no trace on read-back (confirmed live, arguably a sharper case of
/// the same collision problem as the XML label-sanitization fix). This
/// used to be named `strip_nulls` and mirror Python's `_strip_nulls`
/// null-dropping path-numbering; renamed since it no longer drops
/// anything -- it now returns the first null path it finds as an error.
fn strip_nulls(node: Value, path: &str) -> Result<Value, WriteError> {
    match node {
        Value::Object(map) => {
            let mut out = IndexMap::new();
            for (label, child) in map {
                match child {
                    Value::Null => {
                        let p = crate::report::child_path(path, &label, None);
                        return Err(crate::report::unsupported_value_error(
                            &p,
                            "null value has no TOML representation (TOML has no null token)",
                        ));
                    }
                    Value::Array(items) => {
                        let mut kept = Vec::with_capacity(items.len());
                        let count = items.len();
                        for (i, item) in items.into_iter().enumerate() {
                            let p = crate::report::child_path(
                                path,
                                &label,
                                crate::report::occurrence_index(i, count),
                            );
                            if matches!(item, Value::Null) {
                                return Err(crate::report::unsupported_value_error(
                                    &p,
                                    "null value has no TOML representation (TOML has no null                                      token)",
                                ));
                            }
                            kept.push(strip_nulls(item, &p)?);
                        }
                        out.insert(label, Value::Array(kept));
                    }
                    other => {
                        let p = crate::report::child_path(path, &label, None);
                        out.insert(label, strip_nulls(other, &p)?);
                    }
                }
            }
            Ok(Value::Object(out))
        }
        other => Ok(other),
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
        Value::Date(s) => out.push_str(s),
        Value::Datetime(s) => out.push_str(&super::temporal_text(s, true)),
        // TOML has no "local time with an offset" literal form (only
        // *date*time can carry an offset) -- a `Time` value that happens
        // to carry one (YAML's own `TIME_RE` allows it, see issue #99's
        // `bare_time_literal_with_utc_offset_reads_as_a_genuine_temporal_leaf`)
        // has no native TOML spelling, so it's written as a quoted
        // string instead, the same fallback this module already used
        // before real provenance existed -- see `has_offset`.
        Value::Time(s) => {
            let text = super::temporal_text(s, false);
            if has_offset(s) {
                write_toml_string(&text, out);
            } else {
                out.push_str(&text);
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
    fn check_toml_reports_null_omitted_across_objects_and_arrays() {
        let doc = doc_of(obj(vec![
            ("null_field", Value::Null),
            (
                "nested",
                obj(vec![
                    ("inner_null", Value::Null),
                    ("valid", Value::Int(1.into())),
                ]),
            ),
            (
                "arr",
                Value::Array(vec![
                    Value::Null,
                    Value::Int(2.into()),
                    obj(vec![("arr_inner_null", Value::Null)]),
                ]),
            ),
        ]));
        let rep = check_toml(&doc);
        assert_eq!(rep.adjustments().len(), 4);
        assert_eq!(rep.adjustments()[0].path, "$.null_field");
        assert_eq!(rep.adjustments()[0].code, "write.unsupported-value");
    }

    #[test]
    fn check_toml_grouped_handles_array_root() {
        let arr = Value::Array(vec![Value::Null, Value::Int(42.into())]);
        let mut rep = WriteReport::new();
        check_toml_grouped(&arr, "$", &mut rep);
        assert_eq!(rep.adjustments().len(), 1);
        // The array's items sit under an empty label, which E-36 quotes.
        assert_eq!(rep.adjustments()[0].path, "$[\"\"][0]");
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
        assert_eq!(
            toml_value_to_value(&v, &HashMap::new()).unwrap(),
            Value::Str(String::new())
        );
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

    // ------------------------------------------------------------ null (write.unsupported-value)

    // Was `lenient_write_drops_null_field_and_records_adjustment` before
    // spec Sec8.3.8/Sec8.3.9 (updated 2026-08-24): a null-valued leaf now
    // fails the write unconditionally (`write.unsupported-value`) instead
    // of being dropped with a warning -- the drop erased the edge's
    // existence entirely with no trace on read-back. `strict` no longer
    // changes the outcome (this used to be the lenient case; the old
    // `strict_write_raises_on_null_even_though_severity_is_warning` test
    // below now asserts the identical failure for `strict: true`).
    #[test]
    fn write_fails_unconditionally_on_null_field_lenient() {
        let v = obj(vec![("a", Value::Int((1).into())), ("b", Value::Null)]);
        let doc = doc_of(v);
        let mut rep = WriteReport::new();
        let err = write_toml(&doc, false, Some(&mut rep)).unwrap_err();
        assert!(err.to_string().contains("write.unsupported-value"));
        assert!(err.to_string().contains("$.b"));
        assert!(rep.is_empty());
        assert!(err.report().is_none());
    }

    // Was `lenient_write_drops_null_array_item_shifting_index`: a null
    // array item now fails the write instead of being dropped and
    // shifting later items down.
    #[test]
    fn write_fails_unconditionally_on_null_array_item() {
        let v = obj(vec![(
            "c",
            Value::Array(vec![
                Value::Int((1).into()),
                Value::Null,
                Value::Int((2).into()),
            ]),
        )]);
        let doc = doc_of(v);
        let err = write_toml(&doc, false, None).unwrap_err();
        assert!(err.to_string().contains("write.unsupported-value"));
        assert!(err.to_string().contains("$.c[1]"));
    }

    #[test]
    fn null_in_nested_table_records_nested_path_in_check() {
        let v = obj(vec![(
            "nested",
            obj(vec![("x", Value::Null), ("y", Value::Int((5).into()))]),
        )]);
        let doc = doc_of(v);
        let rep = check_toml(&doc);
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].path, "$.nested.x");
        assert_eq!(rep.adjustments()[0].code, "write.unsupported-value");
    }

    // Renamed from `strict_write_raises_on_null_even_though_severity_is_warning`:
    // `strict` no longer distinguishes anything here -- both modes fail
    // identically and unconditionally, and the error carries no report
    // (the unconditional-failure path returns before any report is built).
    #[test]
    fn write_fails_unconditionally_on_null_strict() {
        let v = obj(vec![("a", Value::Int((1).into())), ("b", Value::Null)]);
        let doc = doc_of(v);
        let err = write_toml(&doc, true, None).unwrap_err();
        assert!(err.to_string().contains("$.b"));
        assert!(err.to_string().contains("write.unsupported-value"));
        assert!(err.report().is_none());
    }

    #[test]
    fn check_toml_reports_without_producing_output() {
        let v = obj(vec![("a", Value::Null)]);
        let doc = doc_of(v);
        let rep = check_toml(&doc);
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].code, "write.unsupported-value");
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
    fn a_bare_radix_prefix_names_the_literal_as_out_of_range() {
        for (text, literal) in [
            (
                "a = 0x
", "0x",
            ),
            (
                "a = 0o
", "0o",
            ),
            (
                "a = [0b]
",
                "0b",
            ),
        ] {
            let err = read_toml(text).unwrap_err();
            assert!(
                matches!(&err, OmnistError::Parse(e)
                    if e.message.contains(&format!("integer literal {literal:?} is out of range"))),
                "{text:?} gave {err:?}"
            );
        }
    }

    /// The scalar a TOML text holds at a top-level label.
    fn scalar_at(text: &str, label: &str) -> Scalar {
        read_toml(text)
            .unwrap()
            .root()
            .get_one(label)
            .unwrap()
            .value()
            .unwrap()
            .clone()
    }

    /// The integer a TOML text holds at `x`, as its decimal digits.
    fn int_at(text: &str, label: &str) -> String {
        let got = scalar_at(text, label);
        assert!(matches!(got, Scalar::Int(_)), "{got:?}");
        got.to_string()
    }

    fn assert_int_digits(err: &OmnistError, path: &str) {
        assert!(
            matches!(err, OmnistError::Document(e)
                if e.code.as_deref() == Some("document.limit.int-digits")
                    && e.path == path
                    && e.message.contains("4300-digit limit")),
            "expected document.limit.int-digits at {path}, got {err:?}"
        );
    }

    #[test]
    fn integer_at_4300_digits_reads_exactly() {
        // D-2: integers are arbitrary precision, so a literal `toml_edit`
        // cannot hold in 64 bits still reads, exactly, up to the digit limit.
        let digits = "9".repeat(4300);
        assert_eq!(int_at(&format!("x = {digits}\n"), "x"), digits);
        assert_eq!(
            int_at(&format!("x = -{digits}\n"), "x"),
            format!("-{digits}")
        );
    }

    #[test]
    fn huge_hex_literal_is_limited_by_its_decimal_digits() {
        // D-28: the limit counts the decimal digits of the VALUE. 3,600 hex
        // digits are about 4,335 decimal digits, over the default 4,300.
        let text = format!("x = 0x{}\n", "f".repeat(3600));
        assert_int_digits(&read_toml(&text).unwrap_err(), "$.x");
        // Far past it, the same.
        let text = format!("x = 0x{}\n", "f".repeat(5000));
        assert_int_digits(&read_toml(&text).unwrap_err(), "$.x");
    }

    #[test]
    fn a_long_binary_literal_within_the_decimal_limit_reads() {
        // 4,301 binary digits are about 1,295 decimal digits: within the limit.
        let text = format!("x = 0b{}\n", "1".repeat(4301));
        let expected: BigInt = (BigInt::from(1) << 4301) - 1;
        assert_eq!(scalar_at(&text, "x"), Scalar::Int(expected));
    }

    #[test]
    fn integer_over_4300_digits_is_the_digit_cap_error() {
        let text = format!("x = {}\n", "9".repeat(4301));
        assert_int_digits(&read_toml(&text).unwrap_err(), "$.x");
        let text = format!("x = -{}\n", "9".repeat(4301));
        assert_int_digits(&read_toml(&text).unwrap_err(), "$.x");
    }

    #[test]
    fn integer_beyond_64_bits_reads_exactly_in_every_radix() {
        let big = "9223372036854775808"; // i64::MAX + 1
        assert_eq!(int_at(&format!("x = {big}\n"), "x"), big);
        assert_eq!(
            int_at("x = -9223372036854775809\n", "x"),
            "-9223372036854775809"
        );
        assert_eq!(int_at("x = 9_223_372_036_854_775_808\n", "x"), big);
        assert_eq!(int_at("x = +9223372036854775808\n", "x"), big);
        assert_eq!(
            int_at("x = 0xFFFFFFFFFFFFFFFFFF\n", "x"),
            "4722366482869645213695"
        );
        assert_eq!(
            int_at("x = 0o7777777777777777777777\n", "x"),
            "73786976294838206463"
        );
        assert_eq!(
            int_at(
                "x = 0b1111111111111111111111111111111111111111111111111111111111111111111111\n",
                "x"
            )
            .to_string(),
            "1180591620717411303423"
        );
    }

    #[test]
    fn a_decimal_literal_over_a_lowered_limit_is_refused_at_its_path() {
        let limits = crate::limits::Limits::default().with_max_int_digits(20);
        let err = read_toml_with("a = 1\nb = 123456789012345678901\n", &limits).unwrap_err();
        assert!(matches!(
            err,
            OmnistError::Document(ref e)
                if e.code.as_deref() == Some("document.limit.int-digits") && e.path == "$.b"
        ));
        let ok = read_toml_with("b = 12345678901234567890\n", &limits).unwrap();
        assert!(ok.root().get_one("b").is_ok());
    }

    #[test]
    fn big_integers_are_found_by_path_inside_arrays_tables_and_inline_tables() {
        let big = "9".repeat(30);
        let text =
            format!("a = [1, {big}]\n[t]\nx = {{ y = {big} }}\n[[r]]\nz = {big}\n[[r]]\nz = 2\n");
        let doc = read_toml(&text).unwrap();
        let t = doc.root().get_one("t").unwrap();
        let x = t.get_one("x").unwrap();
        assert_eq!(
            x.get_one("y").unwrap().value(),
            Ok(&Scalar::Int(big.parse().unwrap()))
        );
        // The same literals over a lowered limit: the path is the element's.
        let limits = crate::limits::Limits::default().with_max_int_digits(20);
        for (text, path) in [
            (format!("a = [1, {big}]\n"), "$.a[1]"),
            (format!("x = {{ y = {big} }}\n"), "$.x.y"),
            (format!("[[r]]\nz = {big}\n[[r]]\nz = 2\n"), "$.r[0].z"),
            (format!("[t]\nz = {big}\n"), "$.t.z"),
        ] {
            let err = read_toml_with(&text, &limits).unwrap_err();
            assert!(
                matches!(&err, OmnistError::Document(e) if e.path == path
                    && e.code.as_deref() == Some("document.limit.int-digits")),
                "{path}: {err:?}"
            );
        }
    }

    #[test]
    fn a_syntax_error_wins_over_a_big_integer_wherever_it_is() {
        let big = "9".repeat(5000);
        // After the literal: reported at its own position.
        let err = read_toml(&format!("a = {big}\nb = = 1\n")).unwrap_err();
        assert!(matches!(err, OmnistError::Parse(ref e) if e.line == 2));
        // Before it: the same.
        let err = read_toml(&format!("b = = 1\na = {big}\n")).unwrap_err();
        assert!(matches!(err, OmnistError::Parse(ref e) if e.line == 1));
        // Several literals, one past the limit, one not.
        let text = format!("a = {big}\nb = 99999999999999999999\nc = 1\n");
        assert_int_digits(&read_toml(&text).unwrap_err(), "$.a");
    }

    #[test]
    fn an_overflowing_float_reads_as_an_infinity() {
        // D-29.
        assert_eq!(scalar_at("a = 1e999\n", "a"), Scalar::Float(f64::INFINITY));
        assert_eq!(
            scalar_at("a = -1e999\n", "a"),
            Scalar::Float(f64::NEG_INFINITY)
        );
        assert_eq!(scalar_at("a = +1e999\n", "a"), Scalar::Float(f64::INFINITY));
        assert_eq!(
            scalar_at("a = 1.5e999\n", "a"),
            Scalar::Float(f64::INFINITY)
        );
        assert_eq!(
            scalar_at("a = 1_0e9_99\n", "a"),
            Scalar::Float(f64::INFINITY)
        );
        assert_eq!(
            scalar_at("a = [1e999]\n", "a"),
            Scalar::Float(f64::INFINITY)
        );
        // And a float that merely underflows is still a finite zero.
        assert_eq!(scalar_at("a = 1e-999\n", "a"), Scalar::Float(0.0));
    }

    #[test]
    fn many_big_integers_read_in_linear_time() {
        // One reparse per refused literal made this quadratic: 8,000 of them
        // took 38 s in a release build. Now one scan finds them all.
        let n = 20_000;
        let big = "99999999999999999999";
        let start = std::time::Instant::now();
        let array = format!("a = [{}]\n", vec![big; n].join(", "));
        assert_eq!(read_toml(&array).unwrap().root().count("a"), n);
        let tables: String = (0..n)
            .map(|i| format!("t{i} = {{ x = {big} }}\n"))
            .collect();
        assert!(read_toml(&tables).is_ok());
        let floats = format!("f = [{}]\n", vec!["1e999"; n].join(", "));
        assert!(read_toml(&floats).is_ok());
        // Generous: an instrumented debug build does all three in a few seconds.
        assert!(start.elapsed().as_secs() < 60, "{:?}", start.elapsed());
    }

    #[test]
    fn the_big_number_scan_skips_keys_strings_comments_and_headers() {
        let big = "99999999999999999999";
        let text = format!(
            "# {big}\n\
             {big} = 1\n\
             k = \"{big}\" # {big}\n\
             s = '{big}'\n\
             m = \"\"\"\n{big} \\\" {big}\n\"\"\"\n\
             e = \"a\\\\\"\n\
             l = '''\n{big}\n'''\n\
             [88888888888888888888]\n\
             v = {big}\n\
             [[x]]\n\
             w = [\n  {big},\n  {{ {big} = 2, y = {big} }},\n  1e999,\n]\n"
        );
        let doc = read_toml(&text).unwrap();
        // The key is a key, still spelled out; the strings are strings.
        assert_eq!(scalar_at(&text, big), Scalar::Int(1.into()));
        assert_eq!(scalar_at(&text, "k"), Scalar::Str(big.to_string()));
        assert_eq!(scalar_at(&text, "s"), Scalar::Str(big.to_string()));
        let h = doc.root().get_one("88888888888888888888").unwrap();
        assert_eq!(h.get_one("v").unwrap().value().unwrap().to_string(), big);
        // Unbalanced closers and a stray quote never panic the scan.
        for bad in ["a = ]]]\n", "a = \"", "a = '''", "a = \"\\"] {
            assert!(read_toml(bad).is_err());
        }
    }

    #[test]
    fn an_overflowing_float_with_a_long_spelling_reads_as_an_infinity() {
        // A float literal with more characters than the digit limit used to
        // be mistaken for an over-long integer.
        let limits = crate::limits::Limits::default().with_max_int_digits(30);
        let doc = read_toml_with("k = 1.0000000000000000000000000001e999\n", &limits).unwrap();
        assert_eq!(
            *doc.root().get_one("k").unwrap().value().unwrap(),
            Scalar::Float(f64::INFINITY)
        );
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
        assert_depth_refusal(&err);
    }

    /// omnist-rs#197: `toml_edit`'s own recursion cap (about 80 levels) is
    /// reported as `document.limit.depth` at `$`, whether its error carries a
    /// span (inline tables, arrays) or not (table headers), and never
    /// panics.
    fn assert_depth_refusal(err: &OmnistError) {
        assert!(
            matches!(err, OmnistError::Document(e)
                if e.code.as_deref() == Some("document.limit.depth")
                    && e.path == "$"
                    && e.message.contains("toml_edit")),
            "expected document.limit.depth, got {err:?}"
        );
    }

    fn header_chain(n: usize, open: &str, close: &str) -> String {
        format!("{open}{}{close}\n", vec!["a"; n].join("."))
    }

    #[test]
    fn deeply_nested_table_headers_are_a_depth_error_not_a_panic() {
        // 80 levels read; 81 and beyond (200 and 201 are the depths the issue
        // measured, 1000 is far past the port's own limit) are refused.
        for (open, close) in [("[", "]"), ("[[", "]]")] {
            let ok = read_toml(&header_chain(80, open, close));
            assert!(ok.is_ok(), "80 levels of {open}: {ok:?}");
            for n in [81, 90, 199, 200, 201, 250, 1000] {
                let err = read_toml(&header_chain(n, open, close)).unwrap_err();
                assert_depth_refusal(&err);
            }
        }
        // The same through the explicit-limits reader, whatever the limits.
        let limits = crate::limits::Limits::default().with_max_depth(250);
        let err = read_toml_with(&header_chain(200, "[", "]"), &limits).unwrap_err();
        assert_depth_refusal(&err);
    }

    #[test]
    fn deeply_nested_inline_tables_arrays_and_dotted_keys_are_a_depth_error() {
        for n in [100usize, 200, 1000] {
            let inline = format!("a = {}1{}\n", "{a = ".repeat(n), "}".repeat(n));
            assert_depth_refusal(&read_toml(&inline).unwrap_err());
            let array = format!("a = {}1{}\n", "[".repeat(n), "]".repeat(n));
            assert_depth_refusal(&read_toml(&array).unwrap_err());
            let dotted = format!("{} = 1\n", vec!["a"; n].join("."));
            assert_depth_refusal(&read_toml(&dotted).unwrap_err());
        }
    }

    /// An over-long integer ahead of a header chain past the cap: the literal
    /// is blanked and the text parsed again, which then hits the recursion
    /// cap with no span. Still a coded depth error, no panic.
    #[test]
    fn an_over_cap_integer_before_a_too_deep_header_is_a_depth_error() {
        let digits = "9".repeat(5000);
        let text = format!("n = {digits}\n{}", header_chain(100, "[", "]"));
        assert_depth_refusal(&read_toml(&text).unwrap_err());
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
            let _ = item_to_value(&Item::None, &HashMap::new());
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

    // ---------------------------------------------- D-3: format.interleaving-lost
    // (issue #156, spec Sec8.3.8. Same MUST as formats-json/basic/
    // cross-label-interleaving-lost-and-reported -- TOML shares JSON's
    // `to_grouped` grouping, so the loss and the report are identical
    // in shape; see json.rs's mirrored tests.)

    fn interleaved_doc() -> Doc {
        Doc::from_raw(crate::document::RawNode::Edges(vec![
            (
                "m".to_string(),
                crate::document::RawNode::Leaf(Scalar::Str("A".to_string())),
            ),
            (
                "x".to_string(),
                crate::document::RawNode::Leaf(Scalar::Str("X".to_string())),
            ),
            (
                "m".to_string(),
                crate::document::RawNode::Leaf(Scalar::Str("B".to_string())),
            ),
        ]))
        .unwrap()
    }

    fn contiguous_repeat_doc() -> Doc {
        Doc::from_raw(crate::document::RawNode::Edges(vec![
            (
                "m".to_string(),
                crate::document::RawNode::Leaf(Scalar::Str("A".to_string())),
            ),
            (
                "m".to_string(),
                crate::document::RawNode::Leaf(Scalar::Str("B".to_string())),
            ),
            (
                "x".to_string(),
                crate::document::RawNode::Leaf(Scalar::Str("X".to_string())),
            ),
        ]))
        .unwrap()
    }

    #[test]
    fn reports_interleaving_lost_on_write() {
        let doc = interleaved_doc();
        let mut report = crate::report::WriteReport::new();
        write_toml(&doc, false, Some(&mut report)).unwrap();
        let adjustments = report.adjustments();
        assert_eq!(adjustments.len(), 1);
        assert_eq!(adjustments[0].path, "$");
        assert_eq!(adjustments[0].code, "format.interleaving-lost");
        assert_eq!(adjustments[0].severity, crate::report::Severity::Warning);
    }

    #[test]
    fn check_toml_reports_interleaving_lost() {
        let rep = check_toml(&interleaved_doc());
        assert_eq!(rep.adjustments().len(), 1);
        assert_eq!(rep.adjustments()[0].code, "format.interleaving-lost");
    }

    #[test]
    fn contiguous_repeated_label_does_not_report_interleaving_lost() {
        let doc = contiguous_repeat_doc();
        let mut report = crate::report::WriteReport::new();
        write_toml(&doc, false, Some(&mut report)).unwrap();
        assert!(report.is_empty());
        assert!(check_toml(&doc).is_empty());
    }
}
