//! JSON codec. Ported from `~/dev/omnist/omnist/formats.py`'s
//! `read_json`/`write_json`/`check_json`.
//!
//! ## Depth guard reuse
//!
//! [`read_json`] parses JSON text into a [`crate::document::Value`], then
//! builds a [`Doc`] via [`Doc::of`] -- which calls
//! [`crate::document::check_write_depth`] internally (see `document.rs`).
//! [`write_json`]/[`check_json`] walk an already-built `Doc` (via
//! `Doc::to_grouped`/`Doc::root`), whose every node was depth-checked at
//! construction time -- there is nothing left to re-guard on the way out,
//! exactly the reasoning `Doc::to_grouped`'s own doc comment gives. So this
//! module reuses the *one* shared depth guard transitively rather than
//! adding a second (or third) copy.
//!
//! ## No native temporal type
//!
//! Python's JSON writer stringifies `datetime.date`/`datetime.time` values
//! (JSON has no native temporal type) and records a `temporal.stringified`
//! warning. This port's [`crate::document::Scalar`] has no temporal variant
//! at all (see `document.rs`'s module doc) -- a `date`/`time`/`datetime`
//! value is already a `Scalar::Str` holding its ISO spelling by the time it
//! reaches this codec, so there is nothing left to adjust or report here.
//! The only lossy JSON write left is `NaN`/`Infinity`/`-Infinity`, which
//! JSON's grammar has no token for.
//!
//! ## Integer digit cap (omnist-ts#54 / oml.rs precedent)
//!
//! Live-checked against Python (`omnist.formats.read_json`): a JSON integer
//! literal over 4300 digits raises `ParseError` (CPython's
//! `sys.set_int_max_str_digits` guard fires inside `json.loads` itself,
//! before `build_node` ever sees a value); under the cap, arbitrary
//! precision is accepted (`'9' * 4300` reads as a plain Python `int`). This
//! scanner applies the identical 4300-digit cap *before* attempting to
//! parse the literal, mirroring `oml.rs`'s `MAX_INT_DIGITS` guard exactly
//! (same constant, same "reject the digit run before conversion" shape).
//! Because this port's `Scalar::Int` is `i64` (max ~19 digits), any literal
//! over 19 digits fails as "out of range for a 64-bit integer" well before
//! the 4300-digit cap would ever fire on its own -- the same representational
//! gap already documented in `document.rs`'s module doc for OML. The cap is
//! kept anyway (dead in practice for `i64`, exactly like Python's own limit
//! for who never encounters it) purely to give a stable, specific error for
//! egregiously long digit runs rather than a generic overflow message, and
//! to keep this scanner structurally parallel to `oml.rs`'s.

use crate::WriteError;
use crate::document::{Doc, Value};
use crate::error::{OmnistError, ParseError};
use crate::formats::int_cap::{MAX_INT_DIGITS, out_of_range_message, over_cap_message};
use crate::report::{Severity, WriteReport};
use indexmap::IndexMap;

// Same guard, same constant as `oml.rs`'s -- see this module's doc
// comment. Constant and message constructors now live in
// [`crate::formats::int_cap`] (issue #49).

/// Parse JSON text into a [`Doc`].
///
/// A bare top-level array, an array nested directly inside another array,
/// or nesting past [`crate::document::MAX_DEPTH`] all surface as
/// [`crate::error::DocumentError`] (via [`Doc::of`]) rather than
/// [`ParseError`], matching Python's `read_json`, which lets `build_node`'s
/// `DocumentError` propagate uncaught alongside its own caught
/// `json.JSONDecodeError`/`ValueError` -> `ParseError` translation.
pub fn read_json(text: &str) -> Result<Doc, OmnistError> {
    let mut p = Parser::new(text);
    p.skip_ws();
    let value = p.parse_value()?;
    p.skip_ws();
    if p.pos < p.n {
        return Err(p
            .error_at(
                p.pos,
                "unexpected trailing data after JSON value".to_string(),
            )
            .into());
    }
    Ok(Doc::of(&value)?)
}

/// Project a [`Doc`] to JSON text.
///
/// `indent: None` writes compact JSON (`, `/`: ` separators, single line);
/// `indent: Some(n)` pretty-prints with `n` spaces per level, matching
/// Python's `indent=` parameter. Lenient by default: a `NaN`/`Infinity`/
/// `-Infinity` leaf is written as `null` and recorded in the report (see
/// this module's doc comment on why no temporal adjustment is needed).
/// `strict: true` raises [`WriteError`] carrying the report instead, via
/// [`crate::report::finish_write`].
pub fn write_json(
    doc: &Doc,
    indent: Option<usize>,
    strict: bool,
    report: Option<&mut WriteReport>,
) -> Result<String, WriteError> {
    let rep = check_json(doc);
    let grouped = doc.to_grouped();
    let prepared = if strict { grouped } else { prepare(grouped) };
    let mut out = String::new();
    write_value(&prepared, indent, 0, &mut out);
    crate::report::finish_write(out, rep, strict, report)
}

/// Report what writing JSON would adjust, without producing output.
pub fn check_json(doc: &Doc) -> WriteReport {
    let mut rep = WriteReport::new();
    let mut leaves = Vec::new();
    let grouped = doc.to_grouped();
    collect_leaves(&grouped, "$", &mut leaves);
    for (path, v) in leaves {
        if let Value::Float(x) = v
            && (x.is_nan() || x.is_infinite())
        {
            rep.add(
                path,
                "float.special",
                format!("{x} is not valid JSON; wrote null"),
                Severity::Error,
            );
        }
    }
    rep
}

/// Yield `(path, value)` for every scalar leaf in a grouped `Value`, mirroring
/// Python's `_leaves` generator. `Doc::to_grouped()` output is already
/// depth-checked (see this module's doc comment), so no depth guard is
/// needed here -- only path bookkeeping for same-label array indices.
fn collect_leaves<'a>(node: &'a Value, path: &str, out: &mut Vec<(String, &'a Value)>) {
    match node {
        Value::Object(map) => {
            for (label, child) in map {
                match child {
                    Value::Array(items) => {
                        for (i, item) in items.iter().enumerate() {
                            let p = if i == 0 {
                                format!("{path}.{label}")
                            } else {
                                format!("{path}.{label}[{i}]")
                            };
                            collect_leaves(item, &p, out);
                        }
                    }
                    other => {
                        collect_leaves(other, &format!("{path}.{label}"), out);
                    }
                }
            }
        }
        // A grouped `Value` never has a bare top-level/standalone `Array`
        // outside an object edge -- `to_grouped` only ever produces one
        // under an `Object` entry (see `document.rs`) -- so this arm only
        // ever matches a scalar leaf.
        other => out.push((path.to_string(), other)),
    }
}

/// Lenient-mode substitution: a NaN/Infinity leaf becomes `Null` so the
/// written text is always valid JSON. Mirrors Python's `_prepare_json`.
fn prepare(node: Value) -> Value {
    match node {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, prepare(v)))
                .collect::<IndexMap<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(prepare).collect()),
        Value::Float(x) if x.is_nan() || x.is_infinite() => Value::Null,
        other => other,
    }
}

fn write_value(v: &Value, indent: Option<usize>, level: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(x) => write_float(*x, out),
        Value::Str(s) => write_json_string(s, out),
        Value::Array(items) => write_seq(items.iter(), '[', ']', indent, level, out, write_value),
        Value::Object(map) => write_seq(
            map.iter(),
            '{',
            '}',
            indent,
            level,
            out,
            |(k, val), indent, level, out| {
                write_json_string(k, out);
                out.push_str(": ");
                write_value(val, indent, level, out);
            },
        ),
    }
}

fn write_seq<I, T>(
    items: I,
    open: char,
    close: char,
    indent: Option<usize>,
    level: usize,
    out: &mut String,
    mut write_item: impl FnMut(T, Option<usize>, usize, &mut String),
) where
    I: ExactSizeIterator<Item = T>,
{
    if items.len() == 0 {
        out.push(open);
        out.push(close);
        return;
    }
    out.push(open);
    let child_level = level + 1;
    let mut first = true;
    for item in items {
        if !first {
            out.push(',');
            if indent.is_none() {
                out.push(' ');
            }
        }
        first = false;
        if let Some(n) = indent {
            out.push('\n');
            out.push_str(&" ".repeat(n * child_level));
        }
        write_item(item, indent, child_level, out);
    }
    if let Some(n) = indent {
        out.push('\n');
        out.push_str(&" ".repeat(n * level));
    }
    out.push(close);
}

/// Matches Python's `json.dumps(..., default=_iso)` encoder for the special
/// floats it's still asked to serialize in strict mode (discarded when
/// `finish_write` raises, but still produced -- see `write_json`'s call
/// site): bare `NaN`/`Infinity`/`-Infinity` tokens, not valid JSON on their
/// own but exactly what Python's own encoder emits by default.
fn write_float(x: f64, out: &mut String) {
    if x.is_nan() {
        out.push_str("NaN");
    } else if x.is_infinite() {
        out.push_str(if x > 0.0 { "Infinity" } else { "-Infinity" });
    } else {
        // Match `json.dumps`'s float rendering of an integral value, e.g.
        // `1.0` (not `1`) -- `repr(1.0) == "1.0"` in Python. Rust's `f64`
        // `Display` never adds a decimal point on its own -- and for large
        // enough integral magnitudes (>= 1e17) it renders a bare digit run
        // with no `.`/`e`/`E` at all, e.g. `1e17.to_string() ==
        // "100000000000000000"` -- so the correct, magnitude-independent
        // test is whether the *rendered string* already contains one of
        // those markers, not whether `x` is below some fixed cutoff (see
        // `oml.rs::write_float`, the reference implementation this mirrors).
        let s = x.to_string();
        if s.contains('.') || s.contains('e') || s.contains('E') {
            out.push_str(&s);
        } else {
            out.push_str(&s);
            out.push_str(".0");
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------- Reader

struct Parser<'a> {
    text: &'a str,
    n: usize,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        // `pos`/`n` are now byte offsets into `text`, not char indices --
        // this scanner reads UTF-8 lazily (via `peek`/`char_at`, which
        // decode at most one char at a time from the current byte offset)
        // instead of materializing the whole input into a `Vec<char>`
        // upfront (issue #43). `pos` is always kept on a UTF-8 char
        // boundary, so every `text[pos..]`/`text.get(pos..)` slice below is
        // safe.
        let n = text.len();
        Parser { text, n, pos: 0 }
    }

    fn line_col(&self, pos: usize) -> (usize, usize) {
        // Byte-offset line/col computation, matching `toml.rs`'s own
        // `line_col` convention (counts `\n` *bytes*, which are always
        // single-byte in UTF-8, so this is correct regardless of any
        // multi-byte characters earlier in the text).
        let bytes = self.text.as_bytes();
        let end = pos.min(self.n);
        let mut line = 1usize;
        let mut last_nl: Option<usize> = None;
        for (i, &b) in bytes[..end].iter().enumerate() {
            if b == b'\n' {
                line += 1;
                last_nl = Some(i);
            }
        }
        let col = match last_nl {
            Some(i) => pos - i,
            None => pos + 1,
        };
        (line, col)
    }

    fn error_at(&self, pos: usize, msg: String) -> ParseError {
        let (line, col) = self.line_col(pos);
        ParseError::new(line, col, format!("invalid JSON: {msg}"))
    }

    /// Decode the char starting at byte offset `at`, if any. `at` must be a
    /// char boundary (always true for `self.pos`, and for the lookahead
    /// offsets used below, since they only ever land on boundaries produced
    /// by this same scanner).
    fn char_at(&self, at: usize) -> Option<char> {
        self.text.get(at..)?.chars().next()
    }

    fn peek(&self) -> Option<char> {
        self.char_at(self.pos)
    }

    fn skip_ws(&mut self) {
        while matches!(
            self.peek(),
            Some(' ') | Some('\t') | Some('\n') | Some('\r')
        ) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), ParseError> {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            Ok(())
        } else {
            Err(self.error_at(self.pos, format!("expected {c:?}")))
        }
    }

    /// `word` is always an ASCII literal keyword (`true`/`false`/`null`/
    /// `NaN`/`Infinity`/`-Infinity`), so comparing byte-for-byte at
    /// `self.pos + i` is exactly equivalent to comparing char-for-char, and
    /// avoids decoding a char per position.
    fn matches_word(&self, word: &str) -> bool {
        debug_assert!(
            word.is_ascii(),
            "matches_word is only used with ASCII keywords"
        );
        let bytes = self.text.as_bytes();
        word.bytes()
            .enumerate()
            .all(|(i, b)| bytes.get(self.pos + i) == Some(&b))
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            None => Err(self.error_at(self.pos, "unexpected end of input".to_string())),
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(Value::Str(self.parse_string()?)),
            Some('t') if self.matches_word("true") => {
                self.pos += 4;
                Ok(Value::Bool(true))
            }
            Some('f') if self.matches_word("false") => {
                self.pos += 5;
                Ok(Value::Bool(false))
            }
            Some('n') if self.matches_word("null") => {
                self.pos += 4;
                Ok(Value::Null)
            }
            Some('N') if self.matches_word("NaN") => {
                self.pos += 3;
                Ok(Value::Float(f64::NAN))
            }
            Some('I') if self.matches_word("Infinity") => {
                self.pos += 8;
                Ok(Value::Float(f64::INFINITY))
            }
            Some('-') if self.matches_word("-Infinity") => {
                self.pos += 9;
                Ok(Value::Float(f64::NEG_INFINITY))
            }
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(self.error_at(self.pos, format!("unexpected character {c:?}"))),
        }
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.expect('{')?;
        let mut map: IndexMap<String, Value> = IndexMap::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.error_at(self.pos, "expected string key".to_string()));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            // Last-duplicate-key-wins, first-seen position kept -- matches
            // Python `dict` semantics (`json.loads('{"a":1,"a":2}')` ==
            // `{"a": 2}`, key position from first occurrence) and
            // `IndexMap::insert`'s own behavior on re-insertion.
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.error_at(self.pos, "expected ',' or '}'".to_string())),
            }
        }
        Ok(Value::Object(map))
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            let v = self.parse_value()?;
            items.push(v);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.error_at(self.pos, "expected ',' or ']'".to_string())),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.expect('"')?;
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error_at(self.pos, "unterminated string".to_string())),
                Some('"') => {
                    self.pos += 1;
                    break;
                }
                Some('\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some('"') => {
                            s.push('"');
                            self.pos += 1;
                        }
                        Some('\\') => {
                            s.push('\\');
                            self.pos += 1;
                        }
                        Some('/') => {
                            s.push('/');
                            self.pos += 1;
                        }
                        Some('b') => {
                            s.push('\u{08}');
                            self.pos += 1;
                        }
                        Some('f') => {
                            s.push('\u{0c}');
                            self.pos += 1;
                        }
                        Some('n') => {
                            s.push('\n');
                            self.pos += 1;
                        }
                        Some('r') => {
                            s.push('\r');
                            self.pos += 1;
                        }
                        Some('t') => {
                            s.push('\t');
                            self.pos += 1;
                        }
                        Some('u') => {
                            self.pos += 1;
                            let hi = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&hi) {
                                if self.peek() == Some('\\')
                                    && self.char_at(self.pos + 1) == Some('u')
                                {
                                    self.pos += 2;
                                    let lo = self.parse_hex4()?;
                                    if (0xDC00..=0xDFFF).contains(&lo) {
                                        let c = 0x10000 + (hi - 0xD800) * 0x400 + (lo - 0xDC00);
                                        // `hi` in 0xD800..=0xDBFF and `lo` in
                                        // 0xDC00..=0xDFFF (just confirmed by
                                        // the two range checks above) always
                                        // combine to a value in
                                        // 0x10000..=0x10FFFF -- the entire
                                        // supplementary-plane range, all of
                                        // which is a valid `char` -- so this
                                        // can never fail; `.expect()`
                                        // documents the invariant instead of
                                        // an unreachable error branch (see
                                        // `oml.rs`'s identical `f64::from_str`
                                        // precedent).
                                        s.push(char::from_u32(c).expect(
                                            "a well-formed UTF-16 surrogate pair always \
                                             combines to a valid supplementary-plane char",
                                        ));
                                    } else {
                                        return Err(self.error_at(
                                            self.pos,
                                            "invalid low surrogate".to_string(),
                                        ));
                                    }
                                } else {
                                    return Err(self.error_at(
                                        self.pos,
                                        "unpaired high surrogate".to_string(),
                                    ));
                                }
                            } else if (0xDC00..=0xDFFF).contains(&hi) {
                                return Err(
                                    self.error_at(self.pos, "unpaired low surrogate".to_string())
                                );
                            } else {
                                // `hi` has already been confirmed outside
                                // both surrogate ranges (0xD800..=0xDFFF) by
                                // the two branches above, and `parse_hex4`
                                // only ever returns a 4-hex-digit value, so
                                // `hi` is in 0..=0xFFFF minus the surrogate
                                // range -- every such value is a valid BMP
                                // `char`, so this can never fail; `.expect()`
                                // documents the invariant (see the surrogate-
                                // pair branch above for the identical
                                // reasoning).
                                s.push(char::from_u32(hi).expect(
                                    "a 4-hex-digit \\u escape outside the surrogate range is \
                                     always a valid BMP char",
                                ));
                            }
                        }
                        _ => return Err(self.error_at(self.pos, "invalid escape".to_string())),
                    }
                }
                Some(c) if (c as u32) < 0x20 => {
                    return Err(self.error_at(self.pos, "control character in string".to_string()));
                }
                Some(c) => {
                    s.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        Ok(s)
    }

    fn parse_hex4(&mut self) -> Result<u32, ParseError> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let c = self.peek().ok_or_else(|| {
                self.error_at(self.pos, "unterminated unicode escape".to_string())
            })?;
            let d = c.to_digit(16).ok_or_else(|| {
                self.error_at(self.pos, "invalid hex digit in unicode escape".to_string())
            })?;
            v = v * 16 + d;
            // Hex digits are always ASCII (single byte); `to_digit(16)`
            // already rejected any non-hex-digit (including any multi-byte
            // char), so `+= 1` is exactly `+= c.len_utf8()` here.
            self.pos += 1;
        }
        Ok(v)
    }

    /// Number grammar: `-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?`. An integer
    /// literal (no fraction/exponent) is parsed as `i64`, applying the
    /// digit cap and out-of-range check documented in this module's doc
    /// comment; a literal with a fraction or exponent is a float.
    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        let int_start = self.pos;
        if self.peek() == Some('0') {
            self.pos += 1;
        } else if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        } else {
            return Err(self.error_at(self.pos, "invalid number literal".to_string()));
        }
        debug_assert!(
            self.pos > int_start,
            "the '0' and digit-run branches above both advance pos by at least 1"
        );
        // The rest of the number grammar (`.`/`e`/`E`/`+`/`-`/digits) is
        // entirely ASCII, so lookahead here compares raw bytes rather than
        // decoding a char at each position.
        let bytes = self.text.as_bytes();
        let byte_at = |p: usize| bytes.get(p).copied();
        let mut is_float = false;
        if self.peek() == Some('.') {
            let frac_start = self.pos + 1;
            let mut p = frac_start;
            while byte_at(p).is_some_and(|b| b.is_ascii_digit()) {
                p += 1;
            }
            if p > frac_start {
                is_float = true;
                self.pos = p;
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let mut p = self.pos + 1;
            if matches!(byte_at(p), Some(b'+') | Some(b'-')) {
                p += 1;
            }
            let exp_start = p;
            while byte_at(p).is_some_and(|b| b.is_ascii_digit()) {
                p += 1;
            }
            if p > exp_start {
                is_float = true;
                self.pos = p;
            }
        }
        let text: &str = &self.text[start..self.pos];
        if is_float {
            let v: f64 = text
                .parse()
                .expect("scanner only emits number-shaped text, which f64::from_str always parses");
            Ok(Value::Float(v))
        } else {
            let digits = &text[if text.starts_with('-') { 1 } else { 0 }..];
            if digits.len() > MAX_INT_DIGITS {
                return Err(self.error_at(start, over_cap_message("", digits.len())));
            }
            match text.parse::<i64>() {
                Ok(v) => Ok(Value::Int(v)),
                Err(_) => Err(self.error_at(start, out_of_range_message("", text))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Doc, Scalar, Value};
    use crate::report::Severity;

    fn obj(pairs: Vec<(&str, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    // ---------------------------------------------------------- reader

    #[test]
    fn reads_object_with_scalars() {
        let doc = read_json(r#"{"a": 1, "b": "s", "c": true, "d": null, "e": 1.5}"#).unwrap();
        let root = doc.root();
        assert_eq!(*root.get_one("a").unwrap().value().unwrap(), Scalar::Int(1));
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Str("s".to_string())
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        assert_eq!(*root.get_one("d").unwrap().value().unwrap(), Scalar::Null);
        assert_eq!(
            *root.get_one("e").unwrap().value().unwrap(),
            Scalar::Float(1.5)
        );
    }

    #[test]
    fn reads_false_literal() {
        let doc = read_json(r#"{"c": false}"#).unwrap();
        assert_eq!(
            *doc.root().get_one("c").unwrap().value().unwrap(),
            Scalar::Bool(false)
        );
    }

    #[test]
    fn reads_array_as_repeated_edges() {
        let doc = read_json(r#"{"m": [1, 2, 3]}"#).unwrap();
        let root = doc.root();
        let ms = root.get("m");
        assert_eq!(ms.len(), 3);
        assert_eq!(*ms[0].value().unwrap(), Scalar::Int(1));
        assert_eq!(*ms[2].value().unwrap(), Scalar::Int(3));
    }

    #[test]
    fn reads_empty_array_literal_as_no_edges() {
        let doc = read_json(r#"{"m": [], "n": 1}"#).unwrap();
        assert!(doc.root().get("m").is_empty());
    }

    #[test]
    fn reads_nested_object() {
        let doc = read_json(r#"{"a": {"b": {"c": 1}}}"#).unwrap();
        let root = doc.root();
        let a = root.get_one("a").unwrap();
        let b = a.get_one("b").unwrap();
        assert_eq!(*b.get_one("c").unwrap().value().unwrap(), Scalar::Int(1));
    }

    #[test]
    fn bare_top_level_array_is_a_document_error_not_a_parse_error() {
        let err = read_json("[1, 2, 3]").unwrap_err();
        assert!(matches!(err, OmnistError::Document(_)), "got {err:?}");
    }

    #[test]
    fn array_of_arrays_is_a_document_error() {
        let err = read_json(r#"{"m": [[1, 2]]}"#).unwrap_err();
        assert!(matches!(err, OmnistError::Document(_)), "got {err:?}");
    }

    #[test]
    fn invalid_json_syntax_is_a_parse_error() {
        let err = read_json("{not json}").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn trailing_data_after_a_value_is_a_parse_error() {
        let err = read_json("1 2").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn nesting_past_max_depth_is_a_document_error() {
        let mut text = String::new();
        for _ in 0..=crate::document::MAX_DEPTH {
            text.push_str(r#"{"a":"#);
        }
        text.push('1');
        for _ in 0..=crate::document::MAX_DEPTH {
            text.push('}');
        }
        let err = read_json(&text).unwrap_err();
        assert!(matches!(err, OmnistError::Document(_)), "got {err:?}");
    }

    #[test]
    fn duplicate_object_keys_last_value_wins_first_position_kept() {
        let doc = read_json(r#"{"a": 1, "b": 2, "a": 3}"#).unwrap();
        let root = doc.root();
        assert_eq!(root.labels(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(*root.get_one("a").unwrap().value().unwrap(), Scalar::Int(3));
    }

    #[test]
    fn reads_string_escapes_and_unicode_escape() {
        let doc = read_json(r#"{"s": "a\n\r\t\"\\é"}"#).unwrap();
        let v = doc.root().get_one("s").unwrap();
        assert_eq!(
            *v.value().unwrap(),
            Scalar::Str("a\n\r\t\"\\\u{e9}".to_string())
        );
    }

    #[test]
    fn reads_plain_unicode_escape() {
        const BSL: char = '\u{5c}';
        let input = format!("{{\"s\": \"{BSL}u0041\"}}");
        let doc = read_json(&input).unwrap();
        let v = doc.root().get_one("s").unwrap();
        assert_eq!(*v.value().unwrap(), Scalar::Str("A".to_string()));
    }

    #[test]
    fn reads_surrogate_pair_escape() {
        // U+1F600 (grinning face) written directly as UTF-8 in the source
        // text (not a `\u` escape) -- exercises the scanner's general
        // multi-byte-character handling.
        let doc = read_json(r#"{"s": "😀"}"#).unwrap();
        let v = doc.root().get_one("s").unwrap();
        assert_eq!(*v.value().unwrap(), Scalar::Str("\u{1F600}".to_string()));
    }

    #[test]
    fn reads_surrogate_pair_written_as_two_u_escapes() {
        // The same U+1F600 grinning face, this time spelled as its UTF-16
        // surrogate pair `😀` -- exercises the combining-formula
        // branch the plain-emoji test above never reaches.
        const BSL: char = '\u{5c}';
        let input = format!("{{\"s\": \"{BSL}ud83d{BSL}ude00\"}}");
        let doc = read_json(&input).unwrap();
        let v = doc.root().get_one("s").unwrap();
        assert_eq!(*v.value().unwrap(), Scalar::Str("\u{1F600}".to_string()));
    }

    #[test]
    fn reads_bare_nan_and_infinity_tokens() {
        // Confirmed live against Python's json.loads (allow_nan=True by
        // default): NaN/Infinity/-Infinity are accepted on read.
        let doc = read_json(r#"{"a": NaN, "b": Infinity, "c": -Infinity}"#).unwrap();
        let root = doc.root();
        assert!(
            matches!(root.get_one("a").unwrap().value().unwrap(), Scalar::Float(x) if x.is_nan())
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Float(f64::INFINITY)
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Float(f64::NEG_INFINITY)
        );
    }

    #[test]
    fn integer_literal_under_digit_cap_but_over_i64_range_is_out_of_range_error() {
        // 20 nines: over i64::MAX's 19 digits, comfortably under the
        // 4300-digit cap -- exercises the *out-of-range* branch, not the
        // digit-cap branch (see this module's doc comment on why the cap
        // is unreachable via i64 alone for such a literal).
        let text = format!(r#"{{"a": {}}}"#, "9".repeat(20));
        let err = read_json(&text).unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn integer_literal_over_digit_cap_is_rejected_before_range_check() {
        let text = format!(r#"{{"a": {}}}"#, "9".repeat(MAX_INT_DIGITS + 1));
        let err = read_json(&text).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("4300-digit")),
            "got {err:?}"
        )
    }

    #[test]
    fn integer_literal_exactly_at_digit_cap_is_out_of_range_not_digit_cap_error() {
        // At exactly 4300 digits it's still an i64 range error (this port's
        // i64-based Scalar can't hold it either way), not the digit-cap
        // message -- confirms the cap boundary is `> MAX_INT_DIGITS`.
        let text = format!(r#"{{"a": {}}}"#, "9".repeat(MAX_INT_DIGITS));
        let err = read_json(&text).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("out of range")),
            "got {err:?}"
        )
    }

    #[test]
    fn whitespace_and_negative_zero_and_exponent_numbers_read() {
        let doc = read_json("  {\n\"a\" : 1e3,\n\"b\": -0.5, \"c\": 2E-2\t}  ").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Float(1000.0)
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Float(-0.5)
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Float(0.02)
        );
    }

    #[test]
    fn error_position_after_multibyte_content_reports_correct_line() {
        // Regression for issue #43's byte-offset scanner rewrite:
        // `line_col` now counts `\n` *bytes* rather than char-vec indices --
        // confirm the reported line for an error on a later line is still
        // correct when an earlier line contains multi-byte UTF-8 content
        // (accented letters, emoji), i.e. byte-offset arithmetic doesn't
        // regress on non-ASCII input.
        let err = read_json("{\"s\": \"café \u{1F600}\"}\n@").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.line == 2),
            "got {err:?}"
        );
    }

    #[test]
    fn empty_object_and_array_read() {
        let doc = read_json(r#"{"a": {}}"#).unwrap();
        assert!(doc.root().get_one("a").unwrap().edges().unwrap().is_empty());
    }

    #[test]
    fn empty_input_is_unexpected_end_of_input_error() {
        let err = read_json("").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("unexpected end of input")),
            "got {err:?}"
        )
    }

    #[test]
    fn unrecognized_character_is_a_parse_error() {
        let err = read_json("@").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("unexpected character")),
            "got {err:?}"
        )
    }

    #[test]
    fn a_bareword_that_only_partially_matches_a_keyword_is_unexpected_character() {
        // 't' starts "true" but "tx" isn't it -- falls through every
        // keyword-literal guard to the catch-all "unexpected character".
        let err = read_json("tx").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("unexpected character")),
            "got {err:?}"
        )
    }

    #[test]
    fn error_position_reports_the_line_after_a_newline() {
        let err = read_json("{\n  \"a\": @\n}").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.line == 2),
            "got {err:?}"
        );
    }

    #[test]
    fn object_missing_colon_is_a_parse_error() {
        let err = read_json(r#"{"a" 1}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("expected ':'")),
            "got {err:?}"
        )
    }

    #[test]
    fn object_missing_key_is_a_parse_error() {
        let err = read_json(r#"{1: 2}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("expected string key")),
            "got {err:?}"
        )
    }

    #[test]
    fn object_missing_comma_or_brace_is_a_parse_error() {
        let err = read_json(r#"{"a": 1 "b": 2}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("expected ',' or '}'")),
            "got {err:?}"
        )
    }

    #[test]
    fn array_missing_comma_or_bracket_is_a_parse_error() {
        let err = read_json(r#"{"a": [1 2]}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("expected ',' or ']'")),
            "got {err:?}"
        )
    }

    #[test]
    fn unterminated_string_is_a_parse_error() {
        let err = read_json(r#"{"a": "hi}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("unterminated string")),
            "got {err:?}"
        )
    }

    #[test]
    fn control_character_in_string_is_a_parse_error() {
        let err = read_json("{\"a\": \"x\u{0007}y\"}").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("control character")),
            "got {err:?}"
        )
    }

    #[test]
    fn every_short_escape_reads_its_control_character() {
        let doc = read_json(r#"{"a": "\/\b\f"}"#).unwrap();
        let v = doc.root().get_one("a").unwrap();
        assert_eq!(
            *v.value().unwrap(),
            Scalar::Str("/\u{08}\u{0c}".to_string())
        );
    }

    #[test]
    fn invalid_escape_character_is_a_parse_error() {
        let err = read_json(r#"{"a": "\q"}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("invalid escape")),
            "got {err:?}"
        )
    }

    #[test]
    fn unpaired_high_surrogate_is_a_parse_error() {
        let err = read_json(r#"{"a": "\ud800x"}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("unpaired high surrogate")),
            "got {err:?}"
        )
    }

    #[test]
    fn high_surrogate_followed_by_non_low_surrogate_escape_is_a_parse_error() {
        // `A` ('A') is itself a well-formed escape but not a low
        // surrogate, so this exercises "high surrogate followed by
        // another `\u` escape that isn't a low surrogate" specifically
        // (distinct from the sibling test's "high surrogate with no
        // following `\u` escape at all"). Built via `format!` (rather than
        // a literal `A` inside a raw string) to sidestep this file's
        // own multi-layer string-escaping when the second `\u` needs to
        // appear literally in the JSON source text.
        const BSL: char = '\u{5c}';
        let input = format!("{{\"a\": \"{BSL}ud800{BSL}u0041\"}}");
        let err = read_json(&input).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("invalid low surrogate")),
            "got {err:?}"
        )
    }

    #[test]
    fn unpaired_low_surrogate_is_a_parse_error() {
        let err = read_json(r#"{"a": "\udc00"}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("unpaired low surrogate")),
            "got {err:?}"
        )
    }

    #[test]
    fn unterminated_unicode_escape_is_a_parse_error() {
        let err = read_json(r#"{"a": "\u12"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("unterminated unicode escape")),
            "got {err:?}"
        )
    }

    #[test]
    fn invalid_hex_digit_in_unicode_escape_is_a_parse_error() {
        let err = read_json(r#"{"a": "\u12zz"}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("invalid hex digit")),
            "got {err:?}"
        )
    }

    #[test]
    fn invalid_number_literal_is_a_parse_error() {
        let err = read_json(r#"{"a": -x}"#).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("invalid number literal")),
            "got {err:?}"
        )
    }

    // ---------------------------------------------------------- writer

    fn doc_of(v: Value) -> Doc {
        Doc::of(&v).unwrap()
    }

    #[test]
    fn round_trips_every_scalar_kind() {
        let v = obj(vec![
            ("null", Value::Null),
            ("bool", Value::Bool(true)),
            ("int", Value::Int(42)),
            ("float", Value::Float(1.5)),
            ("str", Value::Str("hi".to_string())),
        ]);
        let doc = doc_of(v);
        let text = write_json(&doc, None, false, None).unwrap();
        let back = read_json(&text).unwrap();
        assert!(doc.eq_doc(&back));
    }

    #[test]
    fn round_trips_integral_float_at_and_above_1e17_boundary_issue_46() {
        // Regression test for issue #46: an integral-valued float >= 1e17
        // used to render as a bare digit run (Rust's `f64::to_string()`
        // drops the decimal point up there), which `read_json` then
        // re-read as `Scalar::Int` -- silently changing the scalar's type
        // across a round trip.
        for x in [1.0e17, 1.0e18, -1.23e17, 9.9e16_f64] {
            let doc = doc_of(obj(vec![("a", Value::Float(x))]));
            let text = write_json(&doc, None, false, None).unwrap();
            let back = read_json(&text).unwrap();
            assert_eq!(
                *back.root().get_one("a").unwrap().value().unwrap(),
                Scalar::Float(x),
                "x={x} text={text}"
            );
        }
    }

    #[test]
    fn round_trips_temporal_like_strings_since_scalar_has_no_temporal_type() {
        // date/time/datetime values are already Scalar::Str in this port
        // (see module doc) -- confirm they round-trip as plain strings,
        // with no adjustment recorded.
        let v = obj(vec![("d", Value::Str("2024-01-15".to_string()))]);
        let doc = doc_of(v);
        let mut rep = WriteReport::new();
        let text = write_json(&doc, None, false, Some(&mut rep)).unwrap();
        assert!(rep.is_empty());
        let back = read_json(&text).unwrap();
        assert!(doc.eq_doc(&back));
    }

    #[test]
    fn writes_repeated_labels_as_a_json_array() {
        let doc = doc_of(obj(vec![(
            "m",
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
        )]));
        let text = write_json(&doc, None, false, None).unwrap();
        assert_eq!(text, r#"{"m": [1, 2]}"#);
    }

    #[test]
    fn writes_compact_with_comma_space_separators() {
        let doc = doc_of(obj(vec![("a", Value::Int(1)), ("b", Value::Int(2))]));
        let text = write_json(&doc, None, false, None).unwrap();
        assert_eq!(text, r#"{"a": 1, "b": 2}"#);
    }

    #[test]
    fn writes_indented_multiline() {
        let doc = doc_of(obj(vec![("a", Value::Int(1))]));
        let text = write_json(&doc, Some(2), false, None).unwrap();
        assert_eq!(text, "{\n  \"a\": 1\n}");
    }

    #[test]
    fn writes_empty_object_compactly() {
        let doc = doc_of(obj(vec![("o", Value::Object(IndexMap::new()))]));
        let text = write_json(&doc, Some(2), false, None).unwrap();
        assert!(text.contains("\"o\": {}"));
    }

    #[test]
    fn an_empty_array_value_produces_no_edge_at_all() {
        // `Value::Array([])` under a key expands into zero repeated edges
        // (see `document.rs`'s `child_specs`) -- the label simply doesn't
        // appear in the built `Doc`, so it can't round-trip as `[]`. This
        // is a Document-model property, not something this codec controls.
        let doc = doc_of(obj(vec![("a", Value::Array(vec![])), ("b", Value::Int(1))]));
        let text = write_json(&doc, None, false, None).unwrap();
        assert_eq!(text, r#"{"b": 1}"#);
    }

    #[test]
    fn lenient_write_substitutes_nan_and_infinity_with_null_and_reports_error_severity() {
        let doc = doc_of(obj(vec![
            ("a", Value::Float(f64::NAN)),
            ("b", Value::Float(f64::INFINITY)),
        ]));
        let mut rep = WriteReport::new();
        let text = write_json(&doc, None, false, Some(&mut rep)).unwrap();
        assert_eq!(text, r#"{"a": null, "b": null}"#);
        assert_eq!(rep.len(), 2);
        assert!(rep.errors().iter().all(|a| a.severity == Severity::Error));
        assert!(!rep.is_ok());
    }

    #[test]
    fn strict_write_raises_on_nan_and_carries_the_report() {
        let doc = doc_of(obj(vec![("a", Value::Float(f64::NAN))]));
        let err = write_json(&doc, None, true, None).unwrap_err();
        let rep = err.report().expect("strict WriteError carries a report");
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].code, "float.special");
    }

    #[test]
    fn strict_write_with_no_adjustments_succeeds() {
        let doc = doc_of(obj(vec![("a", Value::Int(1))]));
        let text = write_json(&doc, None, true, None).unwrap();
        assert_eq!(text, r#"{"a": 1}"#);
    }

    #[test]
    fn check_json_reports_without_producing_output() {
        let doc = doc_of(obj(vec![("a", Value::Float(f64::INFINITY))]));
        let rep = check_json(&doc);
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].path, "$.a");
    }

    #[test]
    fn writes_string_escapes() {
        let doc = doc_of(obj(vec![("s", Value::Str("a\n\"\\\tb".to_string()))]));
        let text = write_json(&doc, None, false, None).unwrap();
        assert_eq!(text, r#"{"s": "a\n\"\\\tb"}"#);
    }

    #[test]
    fn writes_unicode_without_escaping_non_ascii() {
        // Matches Python's `ensure_ascii=False`.
        let doc = doc_of(obj(vec![("s", Value::Str("caf\u{e9}".to_string()))]));
        let text = write_json(&doc, None, false, None).unwrap();
        assert_eq!(text, "{\"s\": \"caf\u{e9}\"}");
    }

    #[test]
    fn writes_float_with_trailing_dot_zero_for_integral_values() {
        let doc = doc_of(obj(vec![("f", Value::Float(2.0))]));
        let text = write_json(&doc, None, false, None).unwrap();
        assert_eq!(text, r#"{"f": 2.0}"#);
    }

    #[test]
    fn strict_write_of_negative_infinity_renders_the_bare_token() {
        // strict=true never substitutes -- see write_json's doc comment on
        // why the unprepared node is written (and discarded) before
        // finish_write raises.
        let doc = doc_of(obj(vec![("f", Value::Float(f64::NEG_INFINITY))]));
        let err = write_json(&doc, None, true, None).unwrap_err();
        assert_eq!(err.report().unwrap().len(), 1);
    }

    #[test]
    fn write_float_directly_covers_every_branch() {
        let mut out = String::new();
        write_float(f64::NAN, &mut out);
        assert_eq!(out, "NaN");
        out.clear();
        write_float(f64::INFINITY, &mut out);
        assert_eq!(out, "Infinity");
        out.clear();
        write_float(f64::NEG_INFINITY, &mut out);
        assert_eq!(out, "-Infinity");
        out.clear();
        write_float(1.5, &mut out);
        assert_eq!(out, "1.5");
    }

    #[test]
    fn writes_carriage_return_and_control_character_escapes() {
        let doc = doc_of(obj(vec![(
            "s",
            Value::Str("a\rb\u{08}c\u{0c}d\u{01}".to_string()),
        )]));
        let text = write_json(&doc, None, false, None).unwrap();
        const BS: char = '\u{5c}';
        let expected = format!("{{\"s\": \"a{BS}rb{BS}bc{BS}fd{BS}u0001\"}}");
        assert_eq!(text, expected);
    }

    #[test]
    fn deeply_nested_document_write_reuses_doc_construction_depth_guard() {
        // Doc::of already rejects nesting past MAX_DEPTH at construction
        // time (see this module's doc comment) -- confirms write_json never
        // even sees an over-deep Doc to begin with.
        let mut v = Value::Int(0);
        for _ in 0..=crate::document::MAX_DEPTH {
            v = obj(vec![("a", v)]);
        }
        assert!(Doc::of(&v).is_err());
    }

    #[test]
    fn round_trip_via_live_python_equivalent_scalars() {
        // Cross-checked live against `omnist.formats.write_json`/`read_json`
        // in the accompanying Python venv for the exact same literal text
        // (see the PR description) -- this test pins the Rust side of that
        // comparison.
        let doc = doc_of(obj(vec![
            ("a", Value::Int(1)),
            ("b", Value::Str("x".to_string())),
            (
                "c",
                Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            ),
        ]));
        let text = write_json(&doc, None, false, None).unwrap();
        assert_eq!(text, r#"{"a": 1, "b": "x", "c": [1, 2, 3]}"#);
    }
}
