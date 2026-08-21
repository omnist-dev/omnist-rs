//! OSD (Omnist Schema Definition) -- the text language for the [`crate::schema`]
//! model. Ported from `~/dev/omnist/omnist/osd.py`.
//!
//! Grammar (informal):
//!
//! ```text
//! schema      := record* 'root' NAME
//! record      := 'record' NAME '{' field (',' field)* ','? '}'
//! field       := STRING cardinality? ':' type
//! cardinality := '[' INT? (',' INT?)? ']'   -- [m,n] [m,] [,n] [n]; absent = [1,1]
//! type        := SCALARNAME '?'? | NAME     -- one scalar, or one Ref
//! ```
//!
//! Quoting rule: a `"quoted"` token is always a field label (data string);
//! an unquoted identifier is always a schema name (scalar keyword or Ref).
//! There is no value-domain composition (no `|`, enum, literal fields, or
//! `union`).
//!
//! ## The `any` keyword
//!
//! Python's `osd.py` recognizes `"any"` as a reserved type keyword and
//! parses it to `ANY` (`RESERVED_TYPE_NAMES = SCALAR_NAMES | {"any"}`). This
//! module mirrors that: `any` in a type position parses to
//! [`crate::schema::FieldType::Any`], and -- exactly like Python -- `any` is
//! still a reserved name that cannot be used as a record name (matching the
//! grammar). Since `Any` already includes `null`, a trailing `?` after `any`
//! (`"a": any?`) is a [`SchemaError`] ("redundant"), not silently accepted.

use indexmap::IndexMap;
use regex::Regex;
use std::sync::LazyLock;

use crate::error::SchemaError;
use crate::schema::{Field, Record, Ref, Scalar, ScalarKind, Schema};

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
          (?P<ws>\s+)
        | (?P<comment>\#[^\n]*)
        | (?P<string>"(?:\\.|[^"\\])*")
        | (?P<number>-?\d+\.\d+|-?\d+)
        | (?P<name>[A-Za-z_][A-Za-z0-9_]*)
        | (?P<punct>[{}\[\]:,?])
        "#,
    )
    .unwrap()
});

/// A token kind. `Ws`/`Comment` never make it into the token stream produced
/// by [`tokenize`] -- they're skipped there, exactly like Python's
/// `_tokenize`, which is why they still exist as variants (the regex names
/// them) but are unreachable outside this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind {
    String,
    Number,
    Name,
    Punct,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tok {
    kind: TokKind,
    text: String,
    pos: usize,
}

/// Tokenize OSD source, mirroring Python's `_tokenize`: whitespace and
/// `#`-comments are dropped; everything else becomes a [`Tok`], with a
/// trailing `Eof` sentinel. Returns a [`SchemaError`] naming the offending
/// character and byte offset on the first unrecognized character.
fn tokenize(text: &str) -> Result<Vec<Tok>, SchemaError> {
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let Some(m) = TOKEN_RE.captures(&text[i..]) else {
            // `i` is always either `0` or a previous iteration's
            // `whole.len()` -- a match length reported by the `regex`
            // crate against `&str` input -- so it always lands on a char
            // boundary. `text[i..]` can't panic here, and `.chars().next()`
            // always yields a char (the loop guard `i < text.len()` rules
            // out an empty remainder).
            let ch = text[i..].chars().next().unwrap();
            return Err(SchemaError::new(format!(
                "unexpected character {ch:?} at {i}"
            )));
        };
        // The regex has no anchor, so a match not starting at 0 means the
        // characters at `i` didn't match any alternative.
        let whole = m.get(0).unwrap();
        if whole.start() != 0 {
            // Same char-boundary invariant as above: `i` hasn't changed
            // since the last successful match (or loop start), so it's
            // still a valid boundary and the slice/next() can't fail.
            let ch = text[i..].chars().next().unwrap();
            return Err(SchemaError::new(format!(
                "unexpected character {ch:?} at {i}"
            )));
        }
        let start = i;
        i += whole.len();
        if m.name("ws").is_some() || m.name("comment").is_some() {
            continue;
        }
        let (kind, matched) = if let Some(g) = m.name("string") {
            (TokKind::String, g.as_str())
        } else if let Some(g) = m.name("number") {
            (TokKind::Number, g.as_str())
        } else if let Some(g) = m.name("name") {
            (TokKind::Name, g.as_str())
        } else {
            (TokKind::Punct, m.name("punct").unwrap().as_str())
        };
        toks.push(Tok {
            kind,
            text: matched.to_string(),
            pos: start,
        });
    }
    toks.push(Tok {
        kind: TokKind::Eof,
        text: String::new(),
        pos: text.len(),
    });
    Ok(toks)
}

/// Un-escape a quoted string token's raw text (including its surrounding
/// `"`s) via `\X -> X`, mirroring Python's `_unquote` (`re.sub(r'\\(.)',
/// r'\1', s[1:-1])`).
fn unquote(s: &str) -> String {
    let inner = &s[1..s.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// A field's type as produced by the parser, before being wired into
/// [`Field::new`].
enum ParsedType {
    Scalar(Scalar),
    Ref(Ref),
    Any,
}

impl From<ParsedType> for crate::schema::FieldType {
    fn from(t: ParsedType) -> Self {
        match t {
            ParsedType::Scalar(s) => s.into(),
            ParsedType::Ref(r) => r.into(),
            ParsedType::Any => crate::schema::FieldType::Any,
        }
    }
}

struct Parser {
    toks: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn new(toks: Vec<Tok>) -> Self {
        Parser { toks, i: 0 }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.i]
    }

    fn next_tok(&mut self) -> Tok {
        let t = self.toks[self.i].clone();
        self.i += 1;
        t
    }

    fn expect_punct(&mut self, text: &str) -> Result<Tok, SchemaError> {
        let t = self.next_tok();
        if t.kind != TokKind::Punct || t.text != text {
            return Err(SchemaError::new(format!(
                "expected {text:?} at {}, got {:?}",
                t.pos, t.text
            )));
        }
        Ok(t)
    }

    fn expect_name(&mut self) -> Result<Tok, SchemaError> {
        let t = self.next_tok();
        if t.kind != TokKind::Name {
            return Err(SchemaError::new(format!(
                "expected a name at {}, got {:?}",
                t.pos, t.text
            )));
        }
        Ok(t)
    }

    fn parse_schema(&mut self) -> Result<Schema, SchemaError> {
        let mut env: IndexMap<String, Record> = IndexMap::new();
        let mut root: Option<String> = None;
        while self.peek().kind != TokKind::Eof {
            let t = self.peek().clone();
            if t.kind == TokKind::Name && t.text == "record" {
                let (name, rec, name_pos) = self.parse_record()?;
                self.define(&mut env, name, rec, name_pos)?;
            } else if t.kind == TokKind::Name && t.text == "root" {
                self.next_tok();
                root = Some(self.expect_name()?.text);
            } else {
                return Err(SchemaError::new(format!(
                    "expected 'record' or 'root' at {}, got {:?}",
                    t.pos, t.text
                )));
            }
        }
        let Some(root) = root else {
            return Err(SchemaError::new("a schema must declare a root"));
        };
        Schema::new(Ref::new(root), env)
    }

    fn define(
        &self,
        env: &mut IndexMap<String, Record>,
        name: String,
        rec: Record,
        name_pos: usize,
    ) -> Result<(), SchemaError> {
        if name == "any" {
            return Err(SchemaError::new(format!(
                "'any' is a reserved type name and cannot be used as a record name at {name_pos}"
            )));
        }
        if ScalarKind::ALL.iter().any(|k| k.as_str() == name) {
            return Err(SchemaError::new(format!(
                "{name:?} is a reserved scalar name; a record cannot be defined with \
                 this name, or it could never be referenced (a bare name in a type \
                 position always means the builtin scalar)"
            )));
        }
        if env.contains_key(&name) {
            return Err(SchemaError::new(format!("duplicate definition {name:?}")));
        }
        env.insert(name, rec);
        Ok(())
    }

    /// Parses a `record NAME { ... }` block. The caller (`parse_schema`'s
    /// main loop) only invokes this after peeking a `Name` token whose text
    /// is exactly `"record"`, so consuming that token here can never fail --
    /// there is no reachable error path for a mismatched keyword, unlike
    /// `expect_name`/`expect_punct` which validate tokens the caller hasn't
    /// already checked.
    fn parse_record(&mut self) -> Result<(String, Record, usize), SchemaError> {
        self.next_tok(); // guaranteed to be the `record` keyword, see above.
        let name_tok = self.expect_name()?;
        self.expect_punct("{")?;
        let mut fields = Vec::new();
        while self.peek().text != "}" {
            fields.push(self.parse_field()?);
            if self.peek().text == "," {
                self.next_tok();
            } else {
                break;
            }
        }
        self.expect_punct("}")?;
        let rec = Record::new(fields)?;
        Ok((name_tok.text.clone(), rec, name_tok.pos))
    }

    fn parse_field(&mut self) -> Result<Field, SchemaError> {
        let label_tok = self.next_tok();
        if label_tok.kind != TokKind::String {
            return Err(SchemaError::new(format!(
                "expected a quoted field name at {}, got {:?}",
                label_tok.pos, label_tok.text
            )));
        }
        let label = unquote(&label_tok.text);
        let (min, max) = if self.peek().text == "[" {
            self.parse_cardinality()?
        } else {
            (1, Some(1))
        };
        self.expect_punct(":")?;
        let ty = self.parse_type()?;
        Field::new(label, ty, min, max)
    }

    fn parse_cardinality(&mut self) -> Result<(usize, Option<usize>), SchemaError> {
        self.expect_punct("[")?;
        let mut first: Option<usize> = None;
        if self.peek().kind == TokKind::Number {
            first = Some(self.parse_cardinality_int()?);
        }
        let (lo, hi) = if self.peek().text == "," {
            self.next_tok();
            let mut second: Option<usize> = None;
            if self.peek().kind == TokKind::Number {
                second = Some(self.parse_cardinality_int()?);
            }
            (first.unwrap_or(0), second)
        } else {
            let Some(first) = first else {
                return Err(SchemaError::new(format!(
                    "empty cardinality at {}",
                    self.peek().pos
                )));
            };
            (first, Some(first))
        };
        self.expect_punct("]")?;
        Ok((lo, hi))
    }

    fn parse_cardinality_int(&mut self) -> Result<usize, SchemaError> {
        let t = self.next_tok();
        if t.text.contains('.') {
            return Err(SchemaError::new(format!(
                "cardinality must be a whole number, got {:?} at {}",
                t.text, t.pos
            )));
        }
        t.text.parse::<usize>().map_err(|_| {
            SchemaError::new(format!(
                "cardinality must be a non-negative whole number, got {:?} at {}",
                t.text, t.pos
            ))
        })
    }

    fn parse_type(&mut self) -> Result<ParsedType, SchemaError> {
        let t = self.next_tok();
        if t.kind != TokKind::Name {
            return Err(SchemaError::new(format!(
                "expected a scalar name or a reference at {}, got {:?} (enums and \
                 literal-valued fields are not supported -- a field's type is \
                 always one scalar or a reference to a named record)",
                t.pos, t.text
            )));
        }
        if t.text == "any" {
            if self.peek().text == "?" {
                let q = self.next_tok();
                return Err(SchemaError::new(format!(
                    "'any' already includes null; 'any?' is redundant at {}",
                    q.pos
                )));
            }
            return Ok(ParsedType::Any);
        }
        let mut nullable = false;
        if self.peek().text == "?" {
            self.next_tok();
            nullable = true;
        }
        if ScalarKind::ALL.iter().any(|k| k.as_str() == t.text) {
            return Ok(ParsedType::Scalar(Scalar::named(&t.text, nullable)?));
        }
        if nullable {
            return Err(SchemaError::new(format!(
                "'?' cannot apply to the reference {:?}; use cardinality [0,1] for \
                 an optional field",
                t.text
            )));
        }
        Ok(ParsedType::Ref(Ref::new(t.text)))
    }
}

/// Parse OSD text into a [`Schema`].
pub fn parse_schema(text: &str) -> Result<Schema, SchemaError> {
    let toks = tokenize(text)?;
    Parser::new(toks).parse_schema()
}

// ---------------------------------------------------------------------------
// Serialize a Schema back to OSD text
// ---------------------------------------------------------------------------

/// Serialize a [`Schema`] back to OSD text. Ported from Python's
/// `osd.to_osd`/`_record`/`_field`/`_card`/`_type`.
///
/// `indent: None` renders a single-line, machine-oriented form (record
/// definitions and the trailing `root` statement joined by spaces, fields
/// joined by `, `, no trailing comma) instead of the default
/// pretty-printed, indented form -- mirroring `write_oml`/`write_json`'s
/// own `indent: None` convention. A `Some(n)` sets the pretty-mode indent
/// width in spaces. Both forms round-trip through [`parse_schema`].
pub fn to_osd(schema: &Schema, indent: Option<usize>) -> String {
    let mut parts: Vec<String> = schema
        .env()
        .iter()
        .map(|(name, rec)| osd_record(name, rec, indent))
        .collect();
    parts.push(format!("root {}", schema.root().name));
    if indent.is_none() {
        return format!("{}\n", parts.join(" "));
    }
    format!("{}\n", parts.join("\n"))
}

fn osd_record(name: &str, rec: &Record, indent: Option<usize>) -> String {
    if indent.is_none() {
        let fields: Vec<String> = rec.fields().iter().map(osd_field).collect();
        return format!("record {name} {{ {} }}", fields.join(", "));
    }
    let pad = " ".repeat(indent.unwrap_or(4));
    let mut out = vec![format!("record {name} {{")];
    for f in rec.fields() {
        out.push(format!("{pad}{},", osd_field(f)));
    }
    out.push("}".to_string());
    out.join("\n")
}

fn quote_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn osd_field(f: &Field) -> String {
    let card = if (f.min, f.max) == (1, Some(1)) {
        String::new()
    } else {
        format!(" {}", osd_cardinality(f.min, f.max))
    };
    format!("{}{card}: {}", quote_label(&f.label), osd_type(&f.ty))
}

fn osd_cardinality(lo: usize, hi: Option<usize>) -> String {
    match hi {
        Some(hi) if hi == lo => format!("[{lo}]"),
        Some(hi) => format!("[{lo},{hi}]"),
        None => format!("[{lo},]"),
    }
}

fn osd_type(t: &crate::schema::FieldType) -> String {
    match t {
        crate::schema::FieldType::Ref(r) => r.name.clone(),
        crate::schema::FieldType::Scalar(s) => {
            format!(
                "{}{}",
                s.kind().as_str(),
                if s.is_nullable() { "?" } else { "" }
            )
        }
        crate::schema::FieldType::Any => "any".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldType;

    // -- Tokenizer -----------------------------------------------------------

    #[test]
    fn tokenizer_skips_whitespace_and_comments() {
        let toks = tokenize("  # a comment\n  root  # trailing\nX").unwrap();
        let kinds: Vec<&str> = toks
            .iter()
            .map(|t| {
                if t.text.is_empty() {
                    "eof"
                } else {
                    t.text.as_str()
                }
            })
            .collect();
        assert_eq!(kinds, vec!["root", "X", "eof"]);
    }

    #[test]
    fn tokenizer_handles_string_escapes() {
        let toks = tokenize(r#""a \"quoted\" b\\c""#).unwrap();
        assert_eq!(toks[0].kind, TokKind::String);
        assert_eq!(unquote(&toks[0].text), "a \"quoted\" b\\c");
    }

    #[test]
    fn tokenizer_handles_numbers_int_and_decimal() {
        let toks = tokenize("3 -4 2.5 -1.25").unwrap();
        let texts: Vec<&str> = toks[..4].iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["3", "-4", "2.5", "-1.25"]);
        assert!(toks[..4].iter().all(|t| t.kind == TokKind::Number));
    }

    #[test]
    fn tokenizer_handles_punctuation_and_names() {
        let toks = tokenize("{}[]:,?record_1").unwrap();
        let texts: Vec<&str> = toks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["{", "}", "[", "]", ":", ",", "?", "record_1", ""]
        );
    }

    #[test]
    fn tokenizer_rejects_unexpected_character() {
        let err = tokenize("record X { \"a\": string } root X\n@").unwrap_err();
        assert!(err.to_string().contains("unexpected character"));
        assert!(err.to_string().contains("'@'"));
    }

    #[test]
    fn tokenizer_rejects_unexpected_character_even_when_a_later_match_exists() {
        // The regex search is unanchored over the remaining suffix -- an
        // invalid leading character must still be reported at its own
        // position, not skipped over in favor of a later match (e.g. the
        // `x` in "@x" must not cause the tokenizer to silently resume at
        // position 1).
        let err = tokenize("@x").unwrap_err();
        assert!(err.to_string().contains("unexpected character"));
        assert!(err.to_string().contains("at 0"));
    }

    // -- Parser: minimal valid schema ----------------------------------------

    #[test]
    fn parses_minimal_schema_one_record_and_root() {
        let schema = parse_schema(r#"record X { "a": string } root X"#).unwrap();
        assert_eq!(schema.root().name, "X");
        let rec = schema.env().get("X").unwrap();
        let f = rec.field("a").unwrap();
        assert_eq!(f.min, 1);
        assert_eq!(f.max, Some(1));
        assert_eq!(f.ty, FieldType::Scalar(crate::schema::STRING));
    }

    // -- Parser: cardinality variants -----------------------------------------

    #[test]
    fn cardinality_variants() {
        let schema = parse_schema(
            r#"record X {
                "a" [2]: string,
                "b" [1,3]: string,
                "c" [2,]: string,
                "d" [,5]: string,
                "e": string,
            }
            root X"#,
        )
        .unwrap();
        let rec = schema.env().get("X").unwrap();
        assert_eq!(
            (rec.field("a").unwrap().min, rec.field("a").unwrap().max),
            (2, Some(2))
        );
        assert_eq!(
            (rec.field("b").unwrap().min, rec.field("b").unwrap().max),
            (1, Some(3))
        );
        assert_eq!(
            (rec.field("c").unwrap().min, rec.field("c").unwrap().max),
            (2, None)
        );
        assert_eq!(
            (rec.field("d").unwrap().min, rec.field("d").unwrap().max),
            (0, Some(5))
        );
        assert_eq!(
            (rec.field("e").unwrap().min, rec.field("e").unwrap().max),
            (1, Some(1))
        );
    }

    #[test]
    fn cardinality_empty_brackets_is_an_error() {
        let err = parse_schema(r#"record X { "a" []: string } root X"#).unwrap_err();
        assert!(err.to_string().contains("empty cardinality"));
    }

    #[test]
    fn cardinality_decimal_is_an_error() {
        let err = parse_schema(r#"record X { "a" [2.5]: string } root X"#).unwrap_err();
        assert!(err.to_string().contains("whole number"));
    }

    // -- Parser: scalar with/without `?`, Ref --------------------------------

    #[test]
    fn scalar_type_with_and_without_nullable() {
        let schema = parse_schema(r#"record X { "a": integer, "b": integer? } root X"#).unwrap();
        let rec = schema.env().get("X").unwrap();
        assert_eq!(
            rec.field("a").unwrap().ty,
            FieldType::Scalar(crate::schema::INTEGER)
        );
        assert_eq!(
            rec.field("b").unwrap().ty,
            FieldType::Scalar(crate::schema::nullable(crate::schema::INTEGER))
        );
    }

    #[test]
    fn ref_type_resolves_across_records() {
        let schema = parse_schema(
            r#"record Child { "v": string }
               record Parent { "c": Child }
               root Parent"#,
        )
        .unwrap();
        let rec = schema.env().get("Parent").unwrap();
        assert_eq!(
            rec.field("c").unwrap().ty,
            FieldType::Ref(Ref::new("Child"))
        );
    }

    #[test]
    fn ref_type_rejects_nullable_marker() {
        let err = parse_schema(
            r#"record Child { "v": string }
               record Parent { "c": Child? }
               root Parent"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("cannot apply to the reference"));
    }

    // -- Parser: unknown Ref target / duplicate field label (reuses #6) -----

    #[test]
    fn unknown_ref_target_is_caught() {
        let err = parse_schema(r#"record X { "a": Missing } root X"#).unwrap_err();
        assert!(err.to_string().contains("unknown type"));
        assert!(err.to_string().contains("Missing"));
    }

    #[test]
    fn duplicate_field_label_is_caught() {
        let err = parse_schema(r#"record X { "a": string, "a": integer } root X"#).unwrap_err();
        assert!(err.to_string().contains("duplicate field label"));
    }

    #[test]
    fn duplicate_record_definition_is_caught() {
        let err = parse_schema(r#"record X { "a": string } record X { "b": string } root X"#)
            .unwrap_err();
        assert!(err.to_string().contains("duplicate definition"));
    }

    #[test]
    fn record_cannot_be_named_a_reserved_scalar_name() {
        let err = parse_schema(r#"record string { "a": string } root string"#).unwrap_err();
        assert!(err.to_string().contains("reserved scalar name"));
    }

    // -- Parser: malformed input at the right position -----------------------

    #[test]
    fn malformed_top_level_keyword_reports_position() {
        let err = parse_schema("bogus X").unwrap_err();
        assert!(err.to_string().contains("expected 'record' or 'root'"));
        assert!(err.to_string().contains(" at 0"));
    }

    #[test]
    fn missing_root_is_an_error() {
        let err = parse_schema(r#"record X { "a": string }"#).unwrap_err();
        assert!(err.to_string().contains("must declare a root"));
    }

    #[test]
    fn root_name_must_be_a_name_token() {
        let err = parse_schema(r#"record X { "a": string } root 5"#).unwrap_err();
        assert!(err.to_string().contains("expected a name"));
    }

    #[test]
    fn cardinality_rejects_a_negative_number() {
        let err = parse_schema(r#"record X { "a" [-1]: string } root X"#).unwrap_err();
        assert!(err.to_string().contains("non-negative whole number"));
    }

    #[test]
    fn missing_field_colon_reports_position() {
        let err = parse_schema(r#"record X { "a" string } root X"#).unwrap_err();
        assert!(err.to_string().contains("expected \":\""));
    }

    #[test]
    fn field_label_must_be_quoted() {
        let err = parse_schema(r#"record X { a: string } root X"#).unwrap_err();
        assert!(err.to_string().contains("expected a quoted field name"));
    }

    #[test]
    fn type_position_rejects_non_name_token() {
        let err = parse_schema(r#"record X { "a": 5 } root X"#).unwrap_err();
        assert!(
            err.to_string()
                .contains("expected a scalar name or a reference")
        );
    }

    // -- The `any` keyword: real support --------------------------------------

    #[test]
    fn any_as_field_type_parses_to_the_any_field_type() {
        let schema = parse_schema(r#"record X { "a": any } root X"#).unwrap();
        let rec = schema.env().get("X").unwrap();
        assert_eq!(rec.field("a").unwrap().ty, FieldType::Any);
    }

    #[test]
    fn any_with_nullable_marker_is_redundant_error() {
        let err = parse_schema(r#"record X { "a": any? } root X"#).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("already includes null"));
        assert!(msg.contains("redundant"));
    }

    #[test]
    fn any_round_trips_through_to_osd() {
        let src = r#"record X { "a": any } root X"#;
        let schema = parse_schema(src).unwrap();
        let rendered = to_osd(&schema, None);
        assert_eq!(rendered, "record X { \"a\": any } root X\n");
        let reparsed = parse_schema(&rendered).unwrap();
        assert_eq!(reparsed, schema);
    }

    #[test]
    fn any_as_record_name_is_rejected_as_reserved() {
        let err = parse_schema(r#"record any { "a": string } root any"#).unwrap_err();
        assert!(err.to_string().contains("reserved type name"));
        assert!(err.to_string().contains("cannot be used as a record name"));
    }

    // -- Comments interleaved with real grammar ------------------------------

    #[test]
    fn comments_are_ignored_between_tokens() {
        let schema = parse_schema(
            "# leading comment\nrecord X { # field list\n  \"a\": string # trailing\n} root X # done",
        )
        .unwrap();
        assert_eq!(schema.root().name, "X");
    }

    // -- to_osd ----------------------------------------------------------------

    #[test]
    fn to_osd_pretty_round_trips_through_parse_schema() {
        let src = r#"
            record X {
                "a": string,
                "b" [0,1]: integer?,
                "c" [2,]: X,
            }
            root X
        "#;
        let schema = parse_schema(src).unwrap();
        let rendered = to_osd(&schema, Some(4));
        assert_eq!(
            rendered,
            "record X {\n    \"a\": string,\n    \"b\" [0,1]: integer?,\n    \
             \"c\" [2,]: X,\n}\nroot X\n"
        );
        let reparsed = parse_schema(&rendered).unwrap();
        assert_eq!(reparsed, schema);
    }

    #[test]
    fn to_osd_compact_round_trips_through_parse_schema() {
        let src = r#"record X { "a": string, "b" [0,1]: integer? } root X"#;
        let schema = parse_schema(src).unwrap();
        let rendered = to_osd(&schema, None);
        assert_eq!(
            rendered,
            "record X { \"a\": string, \"b\" [0,1]: integer? } root X\n"
        );
        let reparsed = parse_schema(&rendered).unwrap();
        assert_eq!(reparsed, schema);
    }

    #[test]
    fn to_osd_renders_exact_cardinality_without_brackets_comma() {
        let src = r#"record X { "a" [2]: string } root X"#;
        let schema = parse_schema(src).unwrap();
        assert_eq!(
            to_osd(&schema, None),
            "record X { \"a\" [2]: string } root X\n"
        );
    }

    #[test]
    fn to_osd_renders_ref_type_bare() {
        let src = r#"record Leaf { "v": string } record X { "child": Leaf } root X"#;
        let schema = parse_schema(src).unwrap();
        let rendered = to_osd(&schema, None);
        assert!(rendered.contains("\"child\": Leaf"));
    }

    #[test]
    fn to_osd_empty_record_field_list_renders_braces() {
        let schema = Schema::new(
            Ref::new("X"),
            IndexMap::from([("X".to_string(), Record::new(vec![]).unwrap())]),
        )
        .unwrap();
        assert_eq!(to_osd(&schema, None), "record X {  } root X\n");
        assert_eq!(to_osd(&schema, Some(2)), "record X {\n}\nroot X\n");
    }

    #[test]
    fn test_osd_writer_quote_backslash_escaping() {
        use crate::schema::{Field, INTEGER, Record, Ref, Schema};
        use indexmap::IndexMap;

        let test_labels = ["a\"b", "a\\b", "a\"b\\c", r#"quote"and\backslash"together"#];
        for label in test_labels {
            let field = Field::required(label, INTEGER).unwrap();
            let record = Record::new(vec![field]).unwrap();
            let mut env = IndexMap::new();
            env.insert("Root".to_string(), record);
            let schema = Schema::new(Ref::new("Root"), env).unwrap();

            // Test pretty OSD
            let osd_pretty = to_osd(&schema, Some(4));
            let parsed_pretty = parse_schema(&osd_pretty).expect("pretty OSD should re-parse");
            let rec_pretty = parsed_pretty.env().get("Root").unwrap();
            assert_eq!(rec_pretty.fields()[0].label, label);

            // Test compact OSD
            let osd_compact = to_osd(&schema, None);
            let parsed_compact = parse_schema(&osd_compact).expect("compact OSD should re-parse");
            let rec_compact = parsed_compact.env().get("Root").unwrap();
            assert_eq!(rec_compact.fields()[0].label, label);
        }
    }
}
