//! OML (Omnist Markup Language) -- the native codec for the Document model.
//!
//! Ported from `~/dev/omnist/omnist/oml.py` (issue #10). OML is omnist's own
//! serialization format: every Document -- every ordered, possibly-repeated,
//! possibly-interleaved edge list, and all seven scalar kinds (`string`,
//! `integer`, `number`, `boolean`, `date`, `time`, `datetime`) plus `null` --
//! round-trips through OML exactly, with no adjustment ever needed (unlike
//! JSON/YAML/TOML/XML).
//!
//! This module implements the **OML-Core** grammar in full for both
//! [`read_oml`] and [`write_oml`], plus the **OML-Extended** raw-string
//! (`'...'`, E2) and triple-quoted multiline-string (`"""..."""`, E3)
//! spellings on read only -- [`write_oml`] only ever emits OML-Core
//! double-quoted strings, matching the Python reference.
//!
//! ## Architecture (per issue #1/#10, "architecture freedom")
//!
//! Python's reader is a single-pass scanner built around one compiled
//! "master" regex, deferring line/col computation and scalar-value
//! construction until actually needed -- a Python-performance-specific
//! design (see the module's PR #168 for the profile that motivated it), not
//! a behavioral requirement. This port uses a straightforward hand-written
//! recursive-descent scanner/parser over `Vec<char>` instead: idiomatic
//! Rust, and there's no equivalent hot-path reason to defer decoding here.
//! Observable behavior (parse results, round-trips, error content) matches
//! the Python reference; exact error wording does not need to.
//!
//! ## Node representation
//!
//! [`crate::document::RawNode`] -- not [`crate::document::Value`] -- is the
//! type this codec reads into and writes from. `Value::Object`'s `IndexMap`
//! can't hold a repeated key, so it only represents "repeated label" as a
//! *contiguous* run (an array value under one key); OML must round-trip
//! arbitrary **interleaving** of repeated labels losslessly (its whole
//! reason for existing -- "no adjustment ever needed"), which only
//! `RawNode`'s literal edge list can hold exactly.
//!
//! ## Depth guard (omnist-ts#37 / omnist-ts#70)
//!
//! [`write_oml`] takes a plain, unchecked [`crate::document::RawNode`] --
//! exactly like Python's `write_oml(node)`, which accepts any hand-built
//! canonical node, not necessarily one that passed through a depth-checked
//! builder. So the writer calls the shared
//! [`crate::document::check_write_depth`] guard itself, at every nesting
//! level, rather than assuming its input already got checked somewhere
//! upstream -- the exact bug class omnist-ts#37/#70 were: a writer (or a
//! second writer) that skipped this because *some* builder happened to
//! guard depth already.
//!
//! ## Temporal literals (issue #10 pitfall list)
//!
//! `date`/`time`/`datetime` literals are recognized by their *shape* here
//! (this module's own scanner), then validated by
//! [`crate::schema::is_iso_date`]/[`crate::schema::is_iso_time`]/
//! [`crate::schema::is_iso_datetime`] -- the single shared temporal
//! shape-check from issue #6, reused rather than re-implemented. A
//! recognized-but-semantically-invalid literal (`2024-02-30`) is a
//! `ParseError`, not silently accepted. [`crate::document::Scalar`] has no
//! native date/time/datetime variant (see `document.rs`'s module doc), so a
//! valid temporal literal becomes a `Scalar::Str` holding its exact source
//! spelling, matching [`crate::schema::matches_kind`]'s own convention.

use crate::document::{self, RawNode, Scalar, check_write_depth};
use crate::error::{ParseError, WriteError};
use crate::formats::int_cap::{MAX_INT_DIGITS, out_of_range_message, over_cap_message};
use crate::formats::string_escape::{OML_ESCAPES, write_quoted};
use crate::schema::{is_iso_date, is_iso_datetime, is_iso_time};

// Same security guard as Python's `_MAX_INT_DIGITS`: reject an integer
// literal with more than this many digits before ever attempting to
// convert it (unbounded-digit int-to-str/str-to-int conversion is
// superlinear). `document.rs` has no equivalent constant -- its `Scalar`
// uses `i64` (max 19 digits), so the guard would be permanently dead code
// there (see that module's doc comment) -- this module is the one place an
// arbitrarily-long *digit run* can actually reach the parser (as OML
// source text), so the cap lives here instead. Constant and message
// constructors now live in [`crate::formats::int_cap`] (issue #49); this
// module still owns the *finding* documented above.

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum TokKind {
    Sep,
    Eof,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    /// A quoted/raw/multiline string value, already decoded. Eligible as a
    /// field label (per the grammar's "a quoted token is always a field
    /// label" rule) as well as a scalar value.
    Str(String),
    /// A recognized-and-semantically-valid date/time/datetime literal, its
    /// exact source spelling. Deliberately **not** the same variant as
    /// `Str` -- see the module doc comment: folding these together would
    /// let `_looks_like_edge` misfire on a bare temporal value followed by
    /// a stray `:`, treating the date as a label.
    Temporal(String),
    /// `[A-Za-z_][A-Za-z0-9_-]*` -- may be `null`/`true`/`false`, a bare
    /// label, or (as a scalar) a "bare word" error.
    Ident(String),
    Int(i64),
    Float(f64),
}

struct Scanner<'a> {
    text: &'a str,
    n: usize,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        // Strip a leading UTF-8 BOM, matching the Python reference.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        // `pos`/`n` are byte offsets into `text` (kept on UTF-8 char
        // boundaries throughout), not char indices -- this scanner reads
        // UTF-8 lazily via `char_at` instead of materializing the whole
        // input into a `Vec<char>` upfront (issue #43).
        let n = text.len();
        Scanner { text, n, pos: 0 }
    }

    /// Decode the char starting at byte offset `at`, if any. `at` must be a
    /// char boundary -- always true for `self.pos` and for every lookahead
    /// offset used below, since they only ever land on boundaries this same
    /// scanner produced.
    fn char_at(&self, at: usize) -> Option<char> {
        self.text.get(at..)?.chars().next()
    }

    /// Byte at offset `at`, for ASCII-only structural lookahead (digits,
    /// punctuation, keyword matching) where decoding a full char would be
    /// unnecessary work.
    fn byte_at(&self, at: usize) -> Option<u8> {
        self.text.as_bytes().get(at).copied()
    }

    fn line_col(&self, pos: usize) -> (usize, usize) {
        // Byte-offset line/col, matching `toml.rs`'s own `line_col`
        // convention: counts `\n` *bytes* (always single-byte in UTF-8), so
        // this is correct regardless of multi-byte characters earlier in
        // the text.
        let bytes = self.text.as_bytes();
        let mut line = 1usize;
        let mut last_nl: Option<usize> = None;
        for (i, &b) in bytes[..pos].iter().enumerate() {
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
        ParseError::new(line, col, msg)
    }

    fn word_boundary_ok(&self, end: usize) -> bool {
        !self
            .byte_at(end)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-')
    }

    /// Advance past (and describe) the next significant token.
    fn next(&mut self) -> Result<(TokKind, usize, usize), ParseError> {
        loop {
            let start = self.pos;
            if self.consume_ws_or_comment_run() {
                continue;
            }
            if self.pos > start {
                // A run containing a newline/';' was consumed -- SEP.
                return Ok((TokKind::Sep, start, self.pos));
            }
            if self.pos >= self.n {
                return Ok((TokKind::Eof, self.pos, self.pos));
            }
            let c = self
                .char_at(self.pos)
                .expect("just checked self.pos < self.n above");
            return match c {
                '"' => self.scan_dquote_family(start),
                '\'' => self.scan_raw_string(start),
                '{' => self.single(TokKind::LBrace, start),
                '}' => self.single(TokKind::RBrace, start),
                '[' => self.single(TokKind::LBracket, start),
                ']' => self.single(TokKind::RBracket, start),
                ',' => self.single(TokKind::Comma, start),
                ':' => self.single(TokKind::Colon, start),
                '-' => self.scan_minus(start),
                c if c.is_ascii_digit() => self.scan_digit_start(start),
                c if c.is_alphabetic() || c == '_' => self.scan_word(start),
                other => Err(self.error_at(start, format!("stray character {other:?}"))),
            };
        }
    }

    fn single(
        &mut self,
        kind: TokKind,
        start: usize,
    ) -> Result<(TokKind, usize, usize), ParseError> {
        self.pos = start + 1;
        Ok((kind, start, self.pos))
    }

    /// Consumes one run of `[ \t]`/`#comment`/`\r\n`/`\n`/`;`. Returns
    /// `true` if the run was pure hspace/comment (no token, caller should
    /// loop again); on a newline/`;`-bearing run it advances `self.pos` and
    /// returns `false`, leaving the caller to notice `self.pos` moved and
    /// emit SEP. A lone `\r` (not immediately followed by `\n`) is *not*
    /// part of this run -- matches the Python master regex, which only has
    /// a `\r\n` alternative, never a bare `\r`.
    fn consume_ws_or_comment_run(&mut self) -> bool {
        let start = self.pos;
        let mut saw_sep = false;
        loop {
            match self.char_at(self.pos) {
                Some(' ') | Some('\t') => self.pos += 1,
                Some('#') => {
                    // Comment content may contain arbitrary (including
                    // multi-byte) characters -- advance by each char's own
                    // UTF-8 length, not a flat `+= 1`, so `self.pos` stays
                    // on a char boundary.
                    while let Some(c) = self.char_at(self.pos) {
                        if c == '\n' {
                            break;
                        }
                        self.pos += c.len_utf8();
                    }
                }
                Some('\r') if self.char_at(self.pos + 1) == Some('\n') => {
                    self.pos += 2;
                    saw_sep = true;
                }
                Some('\n') => {
                    self.pos += 1;
                    saw_sep = true;
                }
                Some(';') => {
                    self.pos += 1;
                    saw_sep = true;
                }
                _ => break,
            }
        }
        self.pos > start && !saw_sep
    }

    // -- strings ----------------------------------------------------------

    fn scan_dquote_family(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        if self.char_at(start + 1) == Some('"') && self.char_at(start + 2) == Some('"') {
            self.scan_multiline(start)
        } else {
            self.scan_dquote(start)
        }
    }

    fn scan_dquote(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        let mut i = start + 1;
        let mut out = String::new();
        loop {
            match self.char_at(i) {
                None => {
                    return Err(self.error_at(
                        start,
                        "unterminated string (missing closing \")".to_string(),
                    ));
                }
                Some('"') => {
                    i += 1;
                    self.pos = i;
                    return Ok((TokKind::Str(out), start, i));
                }
                Some('\\') => {
                    let (ch, next_i) = self.decode_escape(start, i)?;
                    out.push_str(&ch);
                    i = next_i;
                }
                Some(c) if (c as u32) < 0x20 => {
                    return Err(self.error_at(
                        start,
                        format!("control character U+{:04X} in string", c as u32),
                    ));
                }
                Some(c) => {
                    out.push(c);
                    i += c.len_utf8();
                }
            }
        }
    }

    fn scan_multiline(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        let mut i = start + 3;
        // Opening-newline elision: a single \n or \r\n right after the
        // """ delimiter is dropped, not part of the value.
        if self.char_at(i) == Some('\n') {
            i += 1;
        } else if self.char_at(i) == Some('\r') && self.char_at(i + 1) == Some('\n') {
            i += 2;
        }
        let mut out = String::new();
        loop {
            match self.char_at(i) {
                None => {
                    return Err(self.error_at(
                        start,
                        "unterminated multiline string (missing closing \"\"\")".to_string(),
                    ));
                }
                Some('"') => {
                    let mut run = 0usize;
                    let mut j = i;
                    while self.char_at(j) == Some('"') {
                        run += 1;
                        j += 1;
                    }
                    if run >= 3 {
                        // Only the first three quotes close the string;
                        // any beyond that are literal content (mirrors the
                        // Python reference's `run >= 3` closing rule).
                        for _ in 0..(run - 3) {
                            out.push('"');
                        }
                        i += run;
                        self.pos = i;
                        return Ok((TokKind::Str(out), start, i));
                    }
                    for _ in 0..run {
                        out.push('"');
                    }
                    i = j;
                }
                Some('\\') => {
                    let (ch, next_i) = self.decode_escape(start, i)?;
                    out.push_str(&ch);
                    i = next_i;
                }
                Some(c) if c == '\t' || c == '\n' || (c as u32) >= 0x20 => {
                    out.push(c);
                    i += c.len_utf8();
                }
                Some(c) => {
                    return Err(self.error_at(
                        start,
                        format!("control character U+{:04X} in multiline string", c as u32),
                    ));
                }
            }
        }
    }

    fn scan_raw_string(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        let mut i = start + 1;
        loop {
            match self.char_at(i) {
                None => {
                    return Err(self.error_at(
                        start,
                        "unterminated raw string (missing closing ')".to_string(),
                    ));
                }
                Some('\'') => {
                    let text: String = self.text[start + 1..i].to_string();
                    i += 1;
                    self.pos = i;
                    return Ok((TokKind::Str(text), start, i));
                }
                Some(c) => i += c.len_utf8(),
            }
        }
    }

    /// Decode one escape sequence at `self.char_at(i) == Some('\\')`, reporting any
    /// error at `tok_start` (the enclosing string's opening delimiter), not
    /// `i` -- matches the Python reference's error-position convention.
    fn decode_escape(&self, tok_start: usize, i: usize) -> Result<(String, usize), ParseError> {
        let Some(c) = self.char_at(i + 1) else {
            return Err(self.error_at(tok_start, "unterminated escape sequence".to_string()));
        };
        let simple = match c {
            '"' => Some('"'),
            '\\' => Some('\\'),
            '/' => Some('/'),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            _ => None,
        };
        if let Some(ch) = simple {
            return Ok((ch.to_string(), i + 2));
        }
        if c != 'u' {
            return Err(self.error_at(tok_start, format!("invalid escape \\{c}")));
        }
        let cp = self.read_hex4(tok_start, i + 2)?;
        let j = i + 6;
        if (0xD800..=0xDBFF).contains(&cp) {
            let has_low_escape = self.char_at(j) == Some('\\') && self.char_at(j + 1) == Some('u');
            let low = if has_low_escape {
                Some(self.read_hex4(tok_start, j + 2)?)
            } else {
                None
            };
            let unpaired_err = || {
                self.error_at(
                    tok_start,
                    format!(
                        "unpaired high surrogate \\u{cp:04x} (needs a following low-surrogate \
                         \\uDC00-\\uDFFF escape)"
                    ),
                )
            };
            return match low {
                Some(low) if (0xDC00..=0xDFFF).contains(&low) => {
                    let combined = 0x10000 + (cp - 0xD800) * 0x400 + (low - 0xDC00);
                    // A high surrogate (0xD800..=0xDBFF) paired with a low
                    // surrogate (0xDC00..=0xDFFF) always combines into a
                    // value in 0x10000..=0x10FFFF -- always a valid Unicode
                    // scalar value. There's no reachable failure branch to
                    // test here (see schema.rs's `mandatory_u32` for the
                    // same "provably safe by the preceding checks"
                    // pattern); `.expect` keeps this a single
                    // always-executed line instead of a lazily-evaluated
                    // error-construction branch coverage would otherwise
                    // demand a test for.
                    let ch = char::from_u32(combined)
                        .expect("surrogate pair math always yields a valid scalar value");
                    Ok((ch.to_string(), j + 6))
                }
                _ => Err(unpaired_err()),
            };
        }
        if (0xDC00..=0xDFFF).contains(&cp) {
            return Err(self.error_at(tok_start, format!("unpaired low surrogate \\u{cp:04x}")));
        }
        // cp is outside the surrogate range (both branches above already
        // returned), so it's always a valid Unicode scalar value -- see
        // the `.expect` above for the same reasoning.
        let ch = char::from_u32(cp)
            .expect("cp outside the surrogate range is always a valid scalar value");
        Ok((ch.to_string(), j))
    }

    fn read_hex4(&self, tok_start: usize, at: usize) -> Result<u32, ParseError> {
        let hex: String = (0..4)
            .map(|k| self.char_at(at + k))
            .collect::<Option<Vec<char>>>()
            .unwrap_or_default()
            .into_iter()
            .collect();
        if hex.len() != 4 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(self.error_at(
                tok_start,
                r"invalid \u escape (need 4 hex digits)".to_string(),
            ));
        }
        Ok(u32::from_str_radix(&hex, 16).expect("validated 4 hex digits"))
    }

    // -- numbers / temporal literals ---------------------------------------

    fn scan_minus(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        if self.matches_word(start, "-inf") && self.word_boundary_ok(start + 4) {
            self.pos = start + 4;
            return Ok((TokKind::Float(f64::NEG_INFINITY), start, self.pos));
        }
        if self.char_at(start + 1).is_some_and(|c| c.is_ascii_digit()) {
            return self.scan_number(start);
        }
        Err(self.error_at(start, "stray character '-'".to_string()))
    }

    fn scan_digit_start(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        if let Some(end) = self.try_datetime(start) {
            return self.finish_temporal(start, end, TemporalKind::Datetime);
        }
        if let Some(end) = self.try_date(start) {
            return self.finish_temporal(start, end, TemporalKind::Date);
        }
        if let Some(end) = self.try_time(start) {
            return self.finish_temporal(start, end, TemporalKind::Time);
        }
        self.scan_number(start)
    }

    fn finish_temporal(
        &mut self,
        start: usize,
        end: usize,
        kind: TemporalKind,
    ) -> Result<(TokKind, usize, usize), ParseError> {
        let text: String = self.text[start..end].to_string();
        let valid = match kind {
            TemporalKind::Date => is_iso_date(&text),
            TemporalKind::Time => is_iso_time(&text),
            TemporalKind::Datetime => is_iso_datetime(&text),
        };
        if !valid {
            let label = match kind {
                TemporalKind::Date => "date",
                TemporalKind::Time => "time",
                TemporalKind::Datetime => "datetime",
            };
            return Err(self.error_at(end, format!("invalid {label} {text:?}")));
        }
        self.pos = end;
        Ok((TokKind::Temporal(text), start, end))
    }

    fn digits_from(&self, pos: usize) -> usize {
        let mut p = pos;
        while self.byte_at(p).is_some_and(|b| b.is_ascii_digit()) {
            p += 1;
        }
        p
    }

    fn expect_digits(&self, pos: usize, count: usize) -> Option<usize> {
        let end = self.digits_from(pos);
        if end - pos == count { Some(end) } else { None }
    }

    fn try_date(&self, pos: usize) -> Option<usize> {
        let mut p = self.expect_digits(pos, 4)?;
        if self.byte_at(p) != Some(b'-') {
            return None;
        }
        p = self.expect_digits(p + 1, 2)?;
        if self.byte_at(p) != Some(b'-') {
            return None;
        }
        self.expect_digits(p + 1, 2)
    }

    fn try_time(&self, pos: usize) -> Option<usize> {
        let mut p = self.expect_digits(pos, 2)?;
        if self.byte_at(p) != Some(b':') {
            return None;
        }
        p = self.expect_digits(p + 1, 2)?;
        if self.byte_at(p) == Some(b':')
            && let Some(after_secs) = self.expect_digits(p + 1, 2)
        {
            p = after_secs;
            if self.byte_at(p) == Some(b'.') {
                let frac_end = self.digits_from(p + 1);
                if frac_end > p + 1 && frac_end - (p + 1) <= 6 {
                    p = frac_end;
                }
            }
        }
        if let Some(sign) = self.byte_at(p)
            && (sign == b'+' || sign == b'-')
            && let Some(after_h) = self.expect_digits(p + 1, 2)
            && self.byte_at(after_h) == Some(b':')
            && let Some(after_m) = self.expect_digits(after_h + 1, 2)
        {
            p = after_m;
        }
        Some(p)
    }

    fn try_datetime(&self, pos: usize) -> Option<usize> {
        let d_end = self.try_date(pos)?;
        if self.byte_at(d_end) != Some(b'T') {
            return None;
        }
        self.try_time(d_end + 1)
    }

    /// `pos` is a digit or `-` followed by a digit (checked by the caller):
    /// scans `INTEGER`/`NUMDEC`/`NUMEXP`.
    fn scan_number(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        let mut p = start;
        if self.byte_at(p) == Some(b'-') {
            p += 1;
        }
        let int_start = p;
        p = self.digits_from(p);
        debug_assert!(p > int_start, "caller guarantees at least one digit");
        let mut end = p;
        let mut is_float = false;
        let frac_end = if self.byte_at(p) == Some(b'.') {
            self.digits_from(p + 1)
        } else {
            p
        };
        if self.byte_at(p) == Some(b'.') && frac_end > p + 1 {
            p = frac_end;
            is_float = true;
            end = self.try_exponent(p).unwrap_or(p);
        } else if let Some(e) = self.try_exponent(p) {
            end = e;
            is_float = true;
        }
        let text: &str = &self.text[start..end];
        self.pos = end;
        if is_float {
            // `text` was built exclusively from ASCII digits, an optional
            // leading `-`, an optional `.digits` fraction, and an optional
            // `e`/`E`[+-]digits exponent (see `digits_from`/`try_exponent`
            // above). That shape always parses as an `f64` -- `f64::from_str`
            // never errors on this grammar; on overflow it yields +/-inf
            // rather than `Err`. A fallible `map_err` here is therefore dead
            // code that `cargo llvm-cov` correctly flags as unreachable, so
            // it's replaced with an `expect` documenting the invariant
            // instead of an uncoverable error branch.
            let v: f64 = text
                .parse()
                .expect("scanner only emits float-shaped digit/exponent text, which f64::from_str always parses");
            Ok((TokKind::Float(v), start, end))
        } else {
            let digits = &text[if text.starts_with('-') { 1 } else { 0 }..];
            if digits.len() > MAX_INT_DIGITS {
                return Err(self.error_at(start, over_cap_message("", digits.len())));
            }
            let v: i64 = text
                .parse()
                .map_err(|_| self.error_at(start, out_of_range_message("", text)))?;
            Ok((TokKind::Int(v), start, end))
        }
    }

    fn try_exponent(&self, pos: usize) -> Option<usize> {
        if !matches!(self.byte_at(pos), Some(b'e') | Some(b'E')) {
            return None;
        }
        let mut q = pos + 1;
        if matches!(self.byte_at(q), Some(b'+') | Some(b'-')) {
            q += 1;
        }
        let dstart = q;
        q = self.digits_from(q);
        if q > dstart { Some(q) } else { None }
    }

    /// `word` is always an ASCII literal keyword (`nan`/`inf`/`-inf`), so
    /// comparing byte-for-byte at `pos + i` is exactly equivalent to
    /// comparing char-for-char.
    fn matches_word(&self, pos: usize, word: &str) -> bool {
        debug_assert!(
            word.is_ascii(),
            "matches_word is only used with ASCII keywords"
        );
        word.bytes()
            .enumerate()
            .all(|(i, b)| self.byte_at(pos + i) == Some(b))
    }

    fn scan_word(&mut self, start: usize) -> Result<(TokKind, usize, usize), ParseError> {
        if self.matches_word(start, "nan") && self.word_boundary_ok(start + 3) {
            self.pos = start + 3;
            return Ok((TokKind::Float(f64::NAN), start, self.pos));
        }
        if self.matches_word(start, "inf") && self.word_boundary_ok(start + 3) {
            self.pos = start + 3;
            return Ok((TokKind::Float(f64::INFINITY), start, self.pos));
        }
        // The caller (`next`) only reaches here after confirming
        // `char_at(start)` is alphabetic or `_` -- but that first char isn't
        // necessarily ASCII (bare labels may start with a Unicode letter),
        // so skip past it by its own UTF-8 length, not a flat `+ 1`. Every
        // char *after* the first is restricted to ASCII
        // alphanumeric/`_`/`-` by the loop below, so `p += 1` there is safe.
        let first_len = self
            .char_at(start)
            .expect("caller already confirmed a char at `start`")
            .len_utf8();
        let mut p = start + first_len;
        while self
            .byte_at(p)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            p += 1;
        }
        let text: String = self.text[start..p].to_string();
        self.pos = p;
        Ok((TokKind::Ident(text), start, p))
    }
}

enum TemporalKind {
    Date,
    Time,
    Datetime,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

const RESERVED: [&str; 3] = ["null", "true", "false"];

struct Parser<'a> {
    sc: Scanner<'a>,
    kind: TokKind,
    start: usize,
    end: usize,
}

impl<'a> Parser<'a> {
    fn new(mut sc: Scanner<'a>) -> Result<Self, ParseError> {
        let (kind, start, end) = sc.next()?;
        Ok(Parser {
            sc,
            kind,
            start,
            end,
        })
    }

    fn advance(&mut self) -> Result<(TokKind, usize, usize), ParseError> {
        let (nk, ns, ne) = self.sc.next()?;
        let cur = (std::mem::replace(&mut self.kind, nk), self.start, self.end);
        self.start = ns;
        self.end = ne;
        Ok(cur)
    }

    fn skip_sep(&mut self) -> Result<(), ParseError> {
        while matches!(self.kind, TokKind::Sep) {
            self.advance()?;
        }
        Ok(())
    }

    fn tok_display(kind: &TokKind, text: &str) -> String {
        match kind {
            TokKind::Str(s) => format!("{s:?}"),
            _ => format!("{text:?}"),
        }
    }

    fn parse_document(&mut self) -> Result<RawNode, ParseError> {
        self.skip_sep()?;
        let node = if matches!(self.kind, TokKind::Eof) {
            RawNode::Edges(vec![])
        } else if matches!(self.kind, TokKind::LBrace) {
            self.parse_brace_value(0)?
        } else if self.looks_like_edge() {
            RawNode::Edges(self.parse_node_edges(0)?)
        } else {
            RawNode::Leaf(self.parse_scalar()?)
        };
        self.skip_sep()?;
        if !matches!(self.kind, TokKind::Eof) {
            let text: String = self.sc.text[self.start..self.end].to_string();
            return Err(self.sc.error_at(
                self.start,
                format!(
                    "unexpected trailing content after the document body (token {})",
                    Self::tok_display(&self.kind, &text)
                ),
            ));
        }
        Ok(node)
    }

    fn looks_like_edge(&mut self) -> bool {
        match &self.kind {
            TokKind::Str(_) => self.peek_is_colon(),
            TokKind::Ident(text) => {
                if RESERVED.contains(&text.as_str()) {
                    false
                } else {
                    self.peek_is_colon()
                }
            }
            _ => false,
        }
    }

    fn peek_is_colon(&mut self) -> bool {
        let saved_pos = self.sc.pos;
        let result = self.sc.next();
        self.sc.pos = saved_pos;
        matches!(result, Ok((TokKind::Colon, _, _)))
    }

    fn parse_node_edges(&mut self, depth: usize) -> Result<Vec<(String, RawNode)>, ParseError> {
        let mut edges = Vec::new();
        self.skip_sep()?;
        while !matches!(self.kind, TokKind::RBrace | TokKind::Eof) {
            let label = self.parse_label()?;
            let (colon_kind, colon_start, colon_end) = self.advance()?;
            if !matches!(colon_kind, TokKind::Colon) {
                let text: String = self.sc.text[colon_start..colon_end].to_string();
                return Err(self.sc.error_at(
                    colon_start,
                    format!(
                        "expected ':' after label {label:?}, got {}",
                        Self::tok_display(&colon_kind, &text)
                    ),
                ));
            }
            if matches!(self.kind, TokKind::LBracket) {
                for element in self.parse_array(depth)? {
                    edges.push((label.clone(), element));
                }
            } else {
                edges.push((label, self.parse_value(depth)?));
            }
            if matches!(self.kind, TokKind::RBrace | TokKind::Eof) {
                break;
            }
            if !matches!(self.kind, TokKind::Sep) {
                let text: String = self.sc.text[self.start..self.end].to_string();
                return Err(self.sc.error_at(
                    self.start,
                    format!(
                        "expected a separator (newline or ';') or '}}', got {}",
                        Self::tok_display(&self.kind, &text)
                    ),
                ));
            }
            self.skip_sep()?;
        }
        Ok(edges)
    }

    fn parse_label(&mut self) -> Result<String, ParseError> {
        let (kind, start, end) = self.advance()?;
        match kind {
            TokKind::Str(s) => Ok(s),
            TokKind::Ident(text) => {
                if RESERVED.contains(&text.as_str()) {
                    Err(self.sc.error_at(
                        start,
                        format!(
                            "{text:?} is a reserved word and cannot be a bare label; quote it: \
                             \"{text}\""
                        ),
                    ))
                } else {
                    Ok(text)
                }
            }
            other => {
                let text: String = self.sc.text[start..end].to_string();
                Err(self.sc.error_at(
                    start,
                    format!("expected a label, got {}", Self::tok_display(&other, &text)),
                ))
            }
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<RawNode, ParseError> {
        if matches!(self.kind, TokKind::LBrace) {
            self.parse_brace_value(depth)
        } else {
            Ok(RawNode::Leaf(self.parse_scalar()?))
        }
    }

    fn parse_brace_value(&mut self, depth: usize) -> Result<RawNode, ParseError> {
        if depth + 1 > document::MAX_DEPTH {
            return Err(ParseError::new(
                0,
                0,
                format!(
                    "nesting exceeds the maximum depth ({})",
                    document::MAX_DEPTH
                ),
            ));
        }
        self.advance()?; // consume '{'
        self.skip_sep()?;
        let edges = self.parse_node_edges(depth + 1)?;
        self.skip_sep()?;
        let (close_kind, close_start, close_end) = self.advance()?;
        if !matches!(close_kind, TokKind::RBrace) {
            let text: String = self.sc.text[close_start..close_end].to_string();
            return Err(self.sc.error_at(
                close_start,
                format!(
                    "expected '}}', got {}",
                    Self::tok_display(&close_kind, &text)
                ),
            ));
        }
        Ok(RawNode::Edges(edges))
    }

    fn parse_array(&mut self, depth: usize) -> Result<Vec<RawNode>, ParseError> {
        let open_start = self.start;
        self.advance()?; // consume '['
        self.skip_sep()?;
        if matches!(self.kind, TokKind::RBracket) {
            return Err(self
                .sc
                .error_at(open_start, "empty array is not allowed".to_string()));
        }
        let mut elements = Vec::new();
        loop {
            if matches!(self.kind, TokKind::LBracket) {
                return Err(self.sc.error_at(
                    self.start,
                    "nested array is not allowed (arrays may only contain scalars, null, or \
                     brace subtrees)"
                        .to_string(),
                ));
            }
            elements.push(self.parse_value(depth)?);
            self.skip_sep()?;
            if matches!(self.kind, TokKind::Comma) {
                self.advance()?;
                self.skip_sep()?;
                if matches!(self.kind, TokKind::RBracket) {
                    break;
                }
                continue;
            }
            break;
        }
        let (close_kind, close_start, close_end) = self.advance()?;
        if !matches!(close_kind, TokKind::RBracket) {
            let text: String = self.sc.text[close_start..close_end].to_string();
            return Err(self.sc.error_at(
                close_start,
                format!(
                    "expected ',' or ']' in array, got {}",
                    Self::tok_display(&close_kind, &text)
                ),
            ));
        }
        Ok(elements)
    }

    fn parse_scalar(&mut self) -> Result<Scalar, ParseError> {
        let (kind, start, end) = self.advance()?;
        match kind {
            TokKind::Str(s) | TokKind::Temporal(s) => Ok(Scalar::Str(s)),
            TokKind::Int(i) => Ok(Scalar::Int(i)),
            TokKind::Float(f) => Ok(Scalar::Float(f)),
            TokKind::Ident(text) => match text.as_str() {
                "null" => Ok(Scalar::Null),
                "true" => Ok(Scalar::Bool(true)),
                "false" => Ok(Scalar::Bool(false)),
                _ => Err(self.sc.error_at(
                    start,
                    format!("bare word {text:?} is not a valid value here; strings must be quoted"),
                )),
            },
            other => {
                let text: String = self.sc.text[start..end].to_string();
                Err(self.sc.error_at(
                    start,
                    format!("expected a value, got {}", Self::tok_display(&other, &text)),
                ))
            }
        }
    }
}

/// Parse OML source into a canonical [`RawNode`] (edge-list or leaf).
///
/// Supports the full OML-Core grammar, plus OML-Extended raw-string (`'...'`)
/// and triple-quoted multiline-string (`"""..."""`) spellings -- see the
/// module doc comment.
pub fn read_oml(text: &str) -> Result<RawNode, ParseError> {
    let sc = Scanner::new(text);
    let mut parser = Parser::new(sc)?;
    parser.parse_document()
}

// ---------------------------------------------------------------------------
// Writer (OML-Core only)
// ---------------------------------------------------------------------------

/// Render a canonical [`RawNode`] as OML-Core source, pretty-printed with
/// `indent` spaces per nesting level.
///
/// OML is lossless for every Document: there's never an adjustment to
/// report, so there's no `strict=`/report machinery -- writing always
/// succeeds, unless the input itself nests deeper than
/// [`crate::document::MAX_DEPTH`] (see the module doc comment on the depth
/// guard).
pub fn write_oml(node: &RawNode, indent: usize) -> Result<String, WriteError> {
    match node {
        RawNode::Leaf(s) => Ok(write_scalar(s)),
        RawNode::Edges(edges) => write_edges(edges, 0, indent, 0),
    }
}

/// Single-line ("compact") rendering: edges joined by `"; "`, no
/// newlines/padding. Mirrors Python's `write_oml(..., indent=None)`. Both
/// forms round-trip through [`read_oml`].
pub fn write_oml_compact(node: &RawNode) -> Result<String, WriteError> {
    match node {
        RawNode::Leaf(s) => Ok(write_scalar(s)),
        RawNode::Edges(edges) => write_edges_compact(edges, 0),
    }
}

/// Report what writing OML would adjust, without producing output. Added
/// for issue #31 (the format registry): OML is lossless for every
/// `Document` (see this module's doc comment), so there is never anything
/// to report -- mirrors Python's `check_oml`, which is exactly `return
/// WriteReport()`. Every other builtin format has a `check_*` function
/// already; this is the OML counterpart, needed so the `"oml"` registry
/// entry has a `check` callable like the other four.
pub fn check_oml(_doc: &crate::document::Doc) -> crate::report::WriteReport {
    crate::report::WriteReport::new()
}

/// `node_depth` is *this edges list's own* depth, matching
/// `document.rs`'s `push_raw`/`build_node` convention exactly: the guard is
/// checked for every node (container *and* leaf) at its own depth, with a
/// child one level deeper than its parent container -- not just at
/// container boundaries. This is what makes the boundary case line up with
/// `document.rs`'s own tests (a leaf at exactly `MAX_DEPTH` is accepted;
/// one past it is rejected), and is the literal fix for the omnist-ts#37/
/// #70 bug class: every depth-costing step is guarded, not only "one
/// nesting level" as a proxy for it.
fn write_edges(
    edges: &[(String, RawNode)],
    depth: usize,
    indent: usize,
    node_depth: usize,
) -> Result<String, WriteError> {
    check_write_depth(node_depth, "$")?;
    let pad = " ".repeat(indent * depth);
    let mut lines = Vec::with_capacity(edges.len());
    for (label, child) in edges {
        let lab = write_label(label);
        match child {
            RawNode::Edges(inner) if inner.is_empty() => {
                check_write_depth(node_depth + 1, "$")?;
                lines.push(format!("{pad}{lab}: {{}}"));
            }
            RawNode::Edges(inner) => {
                let body = write_edges(inner, depth + 1, indent, node_depth + 1)?;
                lines.push(format!("{pad}{lab}: {{\n{body}\n{pad}}}"));
            }
            RawNode::Leaf(s) => {
                check_write_depth(node_depth + 1, "$")?;
                lines.push(format!("{pad}{lab}: {}", write_scalar(s)));
            }
        }
    }
    Ok(lines.join("\n"))
}

fn write_edges_compact(
    edges: &[(String, RawNode)],
    node_depth: usize,
) -> Result<String, WriteError> {
    check_write_depth(node_depth, "$")?;
    let mut parts = Vec::with_capacity(edges.len());
    for (label, child) in edges {
        let lab = write_label(label);
        match child {
            RawNode::Edges(inner) if inner.is_empty() => {
                check_write_depth(node_depth + 1, "$")?;
                parts.push(format!("{lab}: {{}}"));
            }
            RawNode::Edges(inner) => {
                let body = write_edges_compact(inner, node_depth + 1)?;
                parts.push(format!("{lab}: {{ {body} }}"));
            }
            RawNode::Leaf(s) => {
                check_write_depth(node_depth + 1, "$")?;
                parts.push(format!("{lab}: {}", write_scalar(s)));
            }
        }
    }
    Ok(parts.join("; "))
}

fn is_bare_label(label: &str) -> bool {
    let mut chars = label.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    // \Z (Rust: full match, no trailing-newline-before-end laxity) --
    // matches every remaining char strictly, unlike a `$`-style anchor that
    // would also accept a trailing "\n" (that subtlety is why the Python
    // reference uses `\Z`, not `$`, for this pattern -- see oml.py).
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && !RESERVED.contains(&label)
        && label != "nan"
        && label != "inf"
}

fn write_label(label: &str) -> String {
    if is_bare_label(label) {
        label.to_string()
    } else {
        write_string(label)
    }
}

fn write_scalar(v: &Scalar) -> String {
    match v {
        Scalar::Null => "null".to_string(),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Int(i) => i.to_string(),
        Scalar::Float(f) => write_float(*f),
        Scalar::Str(s) => write_str_scalar(s),
    }
}

/// A `Scalar::Str` shaped like a valid date/time/datetime (per the same
/// [`is_iso_date`]/[`is_iso_time`]/[`is_iso_datetime`] shape-check the
/// reader uses) writes bare, matching the reader's own convention that a
/// temporal literal *is* a `Scalar::Str` holding its exact spelling (see the
/// module doc comment -- `document::Scalar` has no separate temporal
/// variant). This is the omnist-ts#52 fix: `read_oml("a: 12:00")` produces
/// `Scalar::Str("12:00")`, and writing that value back must reproduce the
/// bare `12:00` spelling, not `"12:00"` (which would silently turn a time
/// into a plain string on the next read).
fn write_str_scalar(s: &str) -> String {
    if is_iso_datetime(s) || is_iso_date(s) || is_iso_time(s) {
        s.to_string()
    } else {
        write_string(s)
    }
}

/// Renders `v` so it always re-tokenizes as `NUMDEC`/`NEGINF`/`NANLIT`/
/// `POSINF` on read, never `INTEGER` -- an integer-valued float (`1.0`)
/// must keep a decimal point on write (Rust's `Display` for `f64` omits the
/// trailing `.0`, unlike Python's `repr()`), or the OML round-trip would
/// silently reclassify it as `Scalar::Int` on read-back.
fn write_float(v: f64) -> String {
    crate::formats::float_fmt::float_to_string(v, "nan", "inf", "-inf")
}

/// Escapes every occurrence of a special character -- a per-char loop, not
/// a find/replace pass, so this can never under-sanitize by only touching
/// the *first* match (the general "regex-in-a-writer" risk flagged by
/// issue #10's omnist-ts#36-equivalent note).
fn write_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    write_quoted(s, &OML_ESCAPES, &mut out);
    out
}

#[cfg(test)]
mod tests;
