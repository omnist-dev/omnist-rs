//! OML scanner: a hand-written lexer over UTF-8 source text.
//!
//! Recognizes the OML-Core token grammar plus the OML-Extended raw-string
//! (`'...'`, E2) and triple-quoted multiline-string (`"""..."""`, E3)
//! spellings -- see the parent module's doc comment for the overall
//! architecture rationale (issue #10).
//!
//! ## Temporal literals (issue #10 pitfall list)
//!
//! `date`/`time`/`datetime` literals are recognized by their *shape* here,
//! then validated by
//! [`crate::schema::is_iso_date`]/[`crate::schema::is_iso_time`]/
//! [`crate::schema::is_iso_datetime`] -- the single shared temporal
//! shape-check from issue #6, reused rather than re-implemented. A
//! recognized-but-semantically-invalid literal (`2024-02-30`) is a
//! `ParseError`, not silently accepted. [`crate::document::Scalar`] has no
//! native date/time/datetime variant (see `document.rs`'s module doc), so a
//! valid temporal literal becomes a `Scalar::Str` holding its exact source
//! spelling, matching [`crate::schema::matches_kind`]'s own convention.

use crate::error::ParseError;
use crate::formats::int_cap::{MAX_INT_DIGITS, out_of_range_message, over_cap_message};
use crate::schema::{is_iso_date, is_iso_datetime, is_iso_time};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum TokKind {
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

pub(super) struct Scanner<'a> {
    pub(super) text: &'a str,
    n: usize,
    pub(super) pos: usize,
}

impl<'a> Scanner<'a> {
    pub(super) fn new(text: &'a str) -> Self {
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

    pub(super) fn line_col(&self, pos: usize) -> (usize, usize) {
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

    pub(super) fn error_at(&self, pos: usize, msg: String) -> ParseError {
        let (line, col) = self.line_col(pos);
        ParseError::new(line, col, msg)
    }

    fn word_boundary_ok(&self, end: usize) -> bool {
        !self
            .byte_at(end)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-')
    }

    /// Advance past (and describe) the next significant token.
    pub(super) fn next(&mut self) -> Result<(TokKind, usize, usize), ParseError> {
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

pub(super) enum TemporalKind {
    Date,
    Time,
    Datetime,
}
