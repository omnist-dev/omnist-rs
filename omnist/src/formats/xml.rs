//! XML codec. Ported from `~/dev/omnist/omnist/formats.py`'s
//! `read_xml`/`write_xml`/`check_xml`.
//!
//! ## Structural difference from `json.rs`/`yaml.rs`/`toml.rs`
//!
//! Per the Python module's own top docstring: "XML uses repeated elements
//! directly, so it preserves interleaving on read and needs a single
//! document element on write." Unlike the other three codecs, which all
//! project through [`crate::document::Doc::to_grouped`] (the JSON-shaped
//! "same-label edges become an array" representation), this module goes
//! through [`crate::document::Doc::from_raw`]/[`Doc::to_raw`] and
//! [`crate::document::RawNode`] instead -- the same interleaving-preserving
//! path `crate::oml` uses, and for the same reason: XML element order can
//! interleave distinct labels arbitrarily (`<b/><c/><b/>`), which a
//! `Value::Object`'s `IndexMap` cannot represent (see `RawNode`'s own doc
//! comment in `document.rs`). Read builds a `RawNode` directly from the
//! parsed element tree; write walks `Doc::to_raw()` directly, never
//! `to_grouped()`.
//!
//! ## Crate choice: `quick-xml`, advisory-checked
//!
//! [`quick_xml`] is used for read-side tokenization only (default features,
//! no `serde`); the omnist-specific layer -- interleaving/repetition
//! preservation, the depth guard, scalar coercion, all-occurrences
//! sanitization -- is hand-written, mirroring `yaml.rs`/`toml.rs`'s
//! crate-for-tokenization-plus-hand-written-logic pattern.
//!
//! Checked per the omnist-ts#38 concern (an unfixable `fast-xml-parser`
//! GHSA advisory the TS port could not shed): `cargo audit` against this
//! crate's full dependency tree (29 crates, including `quick-xml` 0.41.0)
//! on 2026-07-26 against the RustSec advisory database (1169 advisories
//! loaded) found **zero** matches -- `quick-xml` has no open advisory at
//! this version, unlike TS's situation. Additionally, `quick-xml` has no
//! DTD/external-entity expansion support at all (only the five predefined
//! XML entities `&lt; &gt; &amp; &apos; &quot;` are recognized; an
//! undefined entity reference is a parse error, not silently resolved) --
//! so, unlike Python's `read_xml` (which specifically requires
//! `defusedxml` instead of the stdlib `xml.etree.ElementTree`, precisely to
//! shut off XXE/entity-expansion attacks), this port has no equivalent
//! opt-in needed: the crate is XXE-safe by construction, not by
//! configuration.
//!
//! ## Namespace handling: a disclosed simplification
//!
//! Python's `read_xml` uses `ElementTree`, which resolves a namespaced tag
//! into Clark notation (`{uri}local`) and `_local()` strips the `{uri}`
//! prefix to get the bare local name. `quick_xml` (used here in
//! non-namespace-aware mode) does not perform namespace URI resolution;
//! [`local_name`] instead strips a lexical `prefix:` (up to the last `:`)
//! from a tag, which coincides with Python's behavior for the common case
//! (a declared, in-scope prefix) but does not resolve prefixes through
//! `xmlns` declarations the way real namespace-aware processing would.
//! Namespaces are outside this issue's spec (the Python docstring never
//! mentions them), so this is a deliberate, disclosed simplification, not
//! a claimed parity guarantee.
//!
//! ## Scalar coercion, confirmed against live Python (omnist-ts#53 lesson)
//!
//! `omnist-ts#53` found TS's XML scalar coercion narrower than Python's,
//! undocumented. This module's [`coerce`] was checked against a live
//! `~/dev/venvs/omnist` interpreter's `omnist.formats._coerce`, not
//! assumed. Confirmed rules (see this module's tests for the exact
//! input/output pairs exercised):
//!
//! * Empty string reads as the empty string `""` -- never coerced.
//! * A **case-insensitive, whole-string** match against `"true"`/`"false"`
//!   reads as a bool -- **without trimming** first: live-confirmed
//!   `_coerce(" true ")` stays the *string* `" true "` (surrounding
//!   whitespace defeats the bool match, since Python compares
//!   `text.lower()` against the literal text, not a trimmed copy).
//! * Otherwise, Python tries `int(text)`, then `float(text)`, keeping the
//!   **original, untrimmed** text if neither succeeds. Both of Python's
//!   conversions themselves trim surrounding whitespace and accept a
//!   single underscore between two digits as a digit-group separator
//!   (`"1_0"` -> `10`, live-confirmed) -- this module's [`try_parse_int`]/
//!   [`try_parse_float`] replicate exactly that (trim, then validate that
//!   every `_` sits strictly between two ASCII digits before stripping
//!   them and delegating to Rust's own `i64`/`f64` parsers).
//! * `float(text)` also accepts the words `inf`/`infinity`/`nan` (any
//!   ASCII case, optionally signed) -- live-confirmed `_coerce('Infinity')
//!   -> inf`, `_coerce('nan') -> nan`. Rust's `f64::from_str` accepts the
//!   same vocabulary (see this module's `parses_inf_and_nan_words` test),
//!   so no special-casing was needed beyond the shared trim/underscore
//!   step.
//! * **Disclosed representational gap** (same shape as `json.rs`'s/
//!   `toml.rs`'s i64-only `Scalar::Int`): live-confirmed
//!   `_coerce('9'*19)` is a Python `int` and `_coerce('9'*20)` is *also*
//!   still a Python `int` (arbitrary precision, no cap until 4300 digits,
//!   where CPython's `int(str)` digit-limit guard makes `int()` raise and
//!   `_coerce` falls back to `float()`, which overflows silently to
//!   `inf` for `'9'*4301` -- live-confirmed). This port's `Scalar::Int` is
//!   `i64`-backed (19-digit range), so anything from 20 digits up through
//!   4300 digits that Python holds as an exact `int` instead falls
//!   through to this module's own `try_parse_float`, which is *always*
//!   attempted after `try_parse_int` fails (unlike `json.rs`/`toml.rs`,
//!   which reject an over-`i64` integer *literal* outright as a
//!   `ParseError`, because JSON/TOML's grammar lexically commits to
//!   "this token is an integer" before parsing it). XML's `_coerce` has no
//!   such lexical commitment -- it is textual coercion, tries int then
//!   float unconditionally -- so falling back to a `Scalar::Float`
//!   instead of erroring is the more faithful match to Python's actual
//!   (int-then-float-fallback) control flow, not a new divergence
//!   invented for this port.
//! * **Not replicated**: Python's `int()`/`float()` accept any Unicode
//!   decimal digit (e.g. U+0663 ARABIC-INDIC DIGIT THREE), live-confirmed
//!   `_coerce('٣') == 3`. This module's parsers are ASCII-digit-only
//!   (`char::is_ascii_digit`) -- a disclosed, narrower-than-Python
//!   simplification in the same spirit as `omnist-ts#53`'s own
//!   already-accepted narrowing, not silently assumed.
//!
//! ## All-occurrences sanitization (omnist-ts#36 regression)
//!
//! `omnist-ts#36`: `writeXml`'s `xmlSanitize` used a **non-global** regex,
//! so only the *first* XML-illegal character in a string was replaced,
//! emitting malformed XML for any string with more than one. This module's
//! [`xml_sanitize`] does not use a substitution regex at all -- it maps
//! every `char` of the input through [`is_xml_illegal_char`] individually
//! (`str::chars().map(...).collect()`), so there is no "first occurrence
//! only" bug class available in the first place. See
//! `sanitizes_every_illegal_character_not_just_the_first` for the
//! regression test with multiple illegal characters in one string.
//!
//! ## Depth guard reuse
//!
//! [`read_xml`] checks nesting depth itself, inline, during the
//! recursive-descent walk of `quick_xml`'s pull events (mirroring Python's
//! own `_xml_to_node`, which inlines the identical check against the
//! shared `_MAX_DEPTH` constant rather than routing through
//! `document.py`'s write-side guard) -- this is necessary because,
//! without it, an adversarially deep input could blow the native Rust call
//! stack in [`parse_content`]'s own recursion *before* [`Doc::from_raw`]
//! ever gets a chance to reject it via
//! [`crate::document::check_write_depth`]. [`Doc::from_raw`] then applies
//! its own guard a second time when building the final `Doc` -- a harmless,
//! redundant confirmation of the same limit, not a second copy of the
//! guard's logic (it calls the crate's one shared [`check_write_depth`]).
//! [`write_xml`]/[`check_xml`] do not re-guard on the way out, matching
//! `json.rs`'s reasoning: they walk an already-`Doc`-validated
//! [`RawNode`], so there is nothing left to guard.
//!
//! ## Single document element (write-side)
//!
//! [`write_xml`] requires `doc.to_raw()` to be a [`RawNode::Edges`] with
//! **exactly one** edge -- any other shape (a bare leaf-rooted `Doc`, zero
//! edges, or more than one top-level edge) is a [`crate::error::WriteError`],
//! matching Python's `write_xml` check
//! (`if not (isinstance(node, list) and len(node) == 1): raise WriteError(...)`)
//! exactly, including that it fires *outside* [`crate::report::finish_write`]
//! -- unconditionally, even under `strict=false`, and with no
//! [`crate::report::WriteReport`] attached (mirrors `toml.rs`'s identical
//! "non-table root" precedent).

use crate::WriteError;
use crate::document::{Doc, MAX_DEPTH, RawNode, Scalar};
use crate::error::{DocumentError, OmnistError, ParseError};
use crate::formats::textpos::line_col_bytes;
use crate::report::{Severity, WriteReport};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashMap;

// ============================================================== Reader

/// Parse XML text into a [`Doc`], preserving element order/interleaving
/// exactly (see this module's doc comment).
pub fn read_xml(text: &str) -> Result<Doc, OmnistError> {
    let normalized = normalize_line_endings(text);
    let mut reader = Reader::from_str(&normalized);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_parse_error(&reader, &normalized, &e))?;
        match ev {
            Event::Start(e) => {
                let tag = local_name(e.name());
                let content = parse_content(&mut reader, &normalized, 1)?;
                let doc = Doc::from_raw(RawNode::Edges(vec![(tag, content)]))?;
                return Ok(doc);
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                let doc = Doc::from_raw(RawNode::Edges(vec![(tag, RawNode::Leaf(coerce("")))]))?;
                return Ok(doc);
            }
            Event::Eof => {
                return Err(located_error(
                    &reader,
                    &normalized,
                    "invalid XML: no root element found",
                ));
            }
            // Decl/Comment/PI/DocType/whitespace-only prolog text: skip.
            _ => continue,
        }
    }
}

/// Reads the content of an already-opened element (the matching `Start`
/// event has already been consumed by the caller) up to and including its
/// `End` event, returning either an internal node ([`RawNode::Edges`], if
/// it has child elements) or a leaf ([`RawNode::Leaf`], coerced from its
/// text), mirroring Python's `_xml_to_node`.
fn parse_content(
    reader: &mut Reader<&[u8]>,
    source: &str,
    depth: usize,
) -> Result<RawNode, OmnistError> {
    if depth > MAX_DEPTH {
        return Err(DocumentError::new(
            "$",
            format!("nesting exceeds the maximum depth ({MAX_DEPTH})"),
        )
        .into());
    }
    let mut text = String::new();
    let mut children: Vec<(String, RawNode)> = Vec::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_parse_error(reader, source, &e))?;
        match ev {
            Event::Start(e) => {
                let tag = local_name(e.name());
                let child = parse_content(reader, source, depth + 1)?;
                children.push((tag, child));
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                children.push((tag, RawNode::Leaf(coerce(""))));
            }
            Event::End(_) => break,
            Event::Text(e) => {
                // `quick_xml` splits entity/character references out into
                // their own `GeneralRef` events (see the arm below) --
                // a `Text` event's content never itself contains an
                // unresolved `&...;` sequence, so only charset decoding
                // (never entity unescaping) is needed here. `decode()`
                // cannot fail for a `Reader::from_str`-backed reader (the
                // crate's own source notes the decoder is fixed to UTF-8
                // automatically in that case -- there is no declared-
                // encoding-vs-actual-bytes mismatch possible when the
                // input was already a Rust `&str`), so this is an
                // `.expect()`, not a propagated error path.
                let decoded = e
                    .decode()
                    .expect("Reader::from_str fixes the decoder to UTF-8; decode() cannot fail");
                text.push_str(&decoded);
            }
            Event::GeneralRef(e) => {
                text.push(resolve_general_ref(reader, source, &e)?);
            }
            Event::CData(e) => {
                text.push_str(&String::from_utf8_lossy(e.as_ref()));
            }
            Event::Eof => {
                return Err(located_error(
                    reader,
                    source,
                    "invalid XML: unexpected end of document",
                ));
            }
            // Comments/PIs inside an element body: skip.
            _ => {}
        }
    }
    if !children.is_empty() {
        if !text.trim().is_empty() {
            return Err(located_error(
                reader,
                source,
                "invalid XML: mixed content (text alongside child elements) is outside the \
                 data-XML profile",
            ));
        }
        Ok(RawNode::Edges(children))
    } else {
        Ok(RawNode::Leaf(coerce(&text)))
    }
}

/// Builds a [`ParseError`] located at the reader's current byte position
/// (converted to a 1-based line/column via [`line_col_bytes`]), for the error
/// conditions this module detects itself rather than receiving from
/// `quick_xml` (unexpected EOF, mixed content).
fn located_error(reader: &Reader<&[u8]>, source: &str, message: &str) -> OmnistError {
    let pos = (reader.buffer_position() as usize).min(source.len());
    let (line, col) = line_col_bytes(source, pos);
    ParseError::new(line, col, message).into()
}

/// The local (unprefixed) part of a tag name -- see this module's doc
/// comment on namespace handling.
fn local_name(name: quick_xml::name::QName) -> String {
    let raw = std::str::from_utf8(name.as_ref()).unwrap_or_default();
    match raw.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => raw.to_string(),
    }
}

/// XML mandates line-ending normalization on parse (any of `"\r\n"`,
/// a lone `"\r"`) to a bare `"\n"` -- live-confirmed against
/// `defusedxml.ElementTree` (see this module's tests).
fn normalize_line_endings(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn xml_parse_error(reader: &Reader<&[u8]>, source: &str, e: &quick_xml::Error) -> OmnistError {
    let pos = (reader.buffer_position() as usize).min(source.len());
    let (line, col) = line_col_bytes(source, pos);
    ParseError::new(line, col, format!("invalid XML: {e}")).into()
}

/// Resolves an `Event::GeneralRef` (a `&...;` entity or character
/// reference `quick_xml` tokenizes as its own event, separate from
/// `Text`) to the single `char` it denotes. Handles a numeric character
/// reference (`&#65;`/`&#x41;`) via the crate's own [`quick_xml::events::BytesRef::resolve_char_ref`],
/// and the five predefined XML entities by name -- `quick_xml` has no
/// DTD support, so no other named entity can ever legitimately appear
/// (see this module's doc comment on why that's a security feature, not
/// a gap).
fn resolve_general_ref(
    reader: &Reader<&[u8]>,
    source: &str,
    e: &quick_xml::events::BytesRef<'_>,
) -> Result<char, OmnistError> {
    if let Some(ch) = e
        .resolve_char_ref()
        .map_err(|err| xml_parse_error(reader, source, &err))?
    {
        return Ok(ch);
    }
    let name = e
        .decode()
        .expect("Reader::from_str fixes the decoder to UTF-8; decode() cannot fail");
    match name.as_ref() {
        "lt" => Ok('<'),
        "gt" => Ok('>'),
        "amp" => Ok('&'),
        "apos" => Ok('\''),
        "quot" => Ok('"'),
        other => {
            let pos = (reader.buffer_position() as usize).min(source.len());
            let (line, col) = line_col_bytes(source, pos);
            Err(ParseError::new(
                line,
                col,
                format!(
                    "invalid XML: unrecognized entity reference '&{other};' (only the five \
                     predefined XML entities are supported; quick_xml has no DTD support)"
                ),
            )
            .into())
        }
    }
}

/// Turns element text into a [`Scalar`] -- see this module's doc comment
/// for the confirmed-against-live-Python coercion rules.
fn coerce(text: &str) -> Scalar {
    if text.is_empty() {
        return Scalar::Str(String::new());
    }
    let low = text.to_lowercase();
    if low == "true" {
        return Scalar::Bool(true);
    }
    if low == "false" {
        return Scalar::Bool(false);
    }
    if let Some(i) = try_parse_int(text) {
        return Scalar::Int(i);
    }
    if let Some(f) = try_parse_float(text) {
        return Scalar::Float(f);
    }
    Scalar::Str(text.to_string())
}

/// `None` if `s` contains an underscore that isn't strictly between two
/// ASCII digits (Python's digit-group-separator rule for `int()`/
/// `float()`); otherwise `Some` of `s` with every underscore removed.
fn strip_underscores_if_valid(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            let prev_ok = i > 0 && chars[i - 1].is_ascii_digit();
            let next_ok = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
            if !(prev_ok && next_ok) {
                return None;
            }
        }
    }
    Some(chars.into_iter().filter(|&c| c != '_').collect())
}

/// Python's `int(text)`: trims whitespace, allows a `+`/`-` sign and
/// underscore digit-group separators, ASCII digits only (see this
/// module's doc comment on the Unicode-digit divergence).
fn try_parse_int(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let cleaned = strip_underscores_if_valid(t)?;
    cleaned.parse::<i64>().ok()
}

/// Python's `float(text)`: trims whitespace, allows a sign, underscore
/// digit-group separators, and the `inf`/`infinity`/`nan` words (any
/// case) -- see this module's doc comment.
fn try_parse_float(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let cleaned = strip_underscores_if_valid(t)?;
    cleaned.parse::<f64>().ok()
}

// ============================================================== Writer

/// Project a [`Doc`] to XML text. See this module's doc comment for the
/// single-document-element requirement, sanitization, and depth-guard
/// decisions.
pub fn write_xml(
    doc: &Doc,
    strict: bool,
    report: Option<&mut WriteReport>,
) -> Result<String, WriteError> {
    let raw = doc.to_raw();
    let RawNode::Edges(edges) = &raw else {
        return Err(single_root_error());
    };
    if edges.len() != 1 {
        return Err(single_root_error());
    }
    let mut rep = WriteReport::new();
    scan_xml_into(&raw, "$", &mut rep);
    let (tag, content) = &edges[0];
    let mut out = String::new();
    write_element(tag, content, 0, &mut out);
    if !matches!(content, RawNode::Edges(e) if !e.is_empty()) && out.ends_with('\n') {
        out.pop();
    }
    crate::report::finish_write(out, rep, strict, report)
}

fn single_root_error() -> WriteError {
    WriteError::new(
        "XML needs exactly one document element; the root node must have a single top-level \
         edge (a single-rooted Document)",
    )
}

/// Report what writing XML would adjust, without producing output. Unlike
/// [`write_xml`], this does not enforce the single-document-element shape
/// (mirrors Python's `check_xml`, which is just `_scan_xml(node, "$", rep)`
/// with no root-shape guard of its own).
pub fn check_xml(doc: &Doc) -> WriteReport {
    let mut rep = WriteReport::new();
    scan_xml_into(&doc.to_raw(), "$", &mut rep);
    rep
}

fn scan_xml_into(node: &RawNode, path: &str, rep: &mut WriteReport) {
    match node {
        RawNode::Edges(edges) => {
            if edges.is_empty() {
                rep.add(
                    path,
                    "shape.empty_ambiguous",
                    "empty internal node (no edges) written as <tag /> and reads back as the \
                     empty-string leaf '', not []",
                    Severity::Warning,
                );
                return;
            }
            let mut counts: HashMap<String, usize> = HashMap::new();
            for (label, child) in edges {
                let i = *counts.get(label).unwrap_or(&0);
                counts.insert(label.clone(), i + 1);
                let p = if i == 0 {
                    format!("{path}.{label}")
                } else {
                    format!("{path}.{label}[{i}]")
                };
                if !is_valid_xml_name(label) {
                    rep.add(
                        p.clone(),
                        "key.sanitized",
                        format!("label {label:?} isn't a valid XML name; written sanitized"),
                        Severity::Warning,
                    );
                }
                scan_xml_into(child, &p, rep);
            }
        }
        RawNode::Leaf(scalar) => scan_leaf(scalar, path, rep),
    }
}

fn scan_leaf(scalar: &Scalar, path: &str, rep: &mut WriteReport) {
    match scalar {
        Scalar::Null => rep.add(
            path,
            "null.omitted",
            "null written as an empty element",
            Severity::Warning,
        ),
        Scalar::Str(v) => {
            if !matches!(coerce(v), Scalar::Str(_)) {
                rep.add(
                    path,
                    "string.ambiguous",
                    format!("string {v:?} looks like another type and reads back as that type"),
                    Severity::Warning,
                );
            }
        }
        Scalar::Bool(_) | Scalar::Int(_) | Scalar::Float(_) => {}
    }
    if let Scalar::Str(v) = scalar {
        if v.chars().any(is_xml_illegal_char) {
            rep.add(
                path,
                "string.illegal_xml_char",
                "string contains a character XML 1.0 cannot represent (e.g. a C0 control other \
                 than tab/LF/CR); it is replaced with U+FFFD on write so the output stays \
                 well-formed",
                Severity::Error,
            );
        }
        if v.contains('\r') {
            rep.add(
                path,
                "string.cr_normalized",
                "string contains a carriage return ('\\r'); XML mandates line-ending \
                 normalization on parse, so '\\r' (and '\\r\\n') read back as '\\n'",
                Severity::Warning,
            );
        }
    }
}

fn write_element(tag: &str, content: &RawNode, level: usize, out: &mut String) {
    let indent = "  ".repeat(level);
    out.push_str(&indent);
    out.push('<');
    out.push_str(&xml_name(tag));
    match content {
        RawNode::Edges(edges) if !edges.is_empty() => {
            out.push_str(">\n");
            for (label, child) in edges {
                write_element(label, child, level + 1, out);
            }
            out.push_str(&indent);
            out.push_str("</");
            out.push_str(&xml_name(tag));
            out.push_str(">\n");
        }
        RawNode::Edges(_) => out.push_str(" />\n"),
        RawNode::Leaf(scalar) => {
            let text = xml_sanitize(&xml_text(scalar));
            if text.is_empty() {
                out.push_str(" />\n");
            } else {
                out.push('>');
                out.push_str(&xml_escape_text(&text));
                out.push_str("</");
                out.push_str(&xml_name(tag));
                out.push_str(">\n");
            }
        }
    }
}

/// A valid XML 1.0 `Name`, simplified to the ASCII-friendly subset Python's
/// own `_XML_NAME` regex accepts (`^[A-Za-z_][A-Za-z0-9_.\-]*$`).
fn is_valid_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// A label as a legal XML element name -- sanitized if it isn't already
/// one, matching Python's `_xml_name`.
fn xml_name(name: &str) -> String {
    if is_valid_xml_name(name) {
        return name.to_string();
    }
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() || !is_valid_xml_name(&safe) {
        format!("_{safe}")
    } else {
        safe
    }
}

fn xml_text(scalar: &Scalar) -> String {
    match scalar {
        Scalar::Null => String::new(),
        Scalar::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Scalar::Int(i) => i.to_string(),
        Scalar::Float(x) => write_float_text(*x),
        Scalar::Str(s) => s.clone(),
    }
}

/// Same float-formatting convention as `toml.rs`'s `write_float`: `nan`/
/// `inf`/`-inf` lowercase for special values, an explicit `.0` appended
/// whenever the default `Display` rendering doesn't already contain a
/// `.`/`e`/`E` marker (matching Python's `str(float)`, which always emits
/// one of those markers; Rust's bare `{}` formatter does not, and for
/// integral values >= 1e17 it doesn't even include a decimal point -- see
/// issue #46).
fn write_float_text(x: f64) -> String {
    if x.is_nan() {
        "nan".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "inf" } else { "-inf" }.to_string()
    } else {
        // See `json.rs::write_float` for why this checks the rendered
        // string for `.`/`e`/`E` rather than comparing `x` against a fixed
        // magnitude cutoff (issue #46: Rust's `f64::to_string()` drops the
        // decimal point for integral values >= 1e17).
        let s = x.to_string();
        if s.contains('.') || s.contains('e') || s.contains('E') {
            s
        } else {
            format!("{s}.0")
        }
    }
}

/// Replaces every character XML 1.0 cannot represent with U+FFFD --
/// see this module's doc comment on the omnist-ts#36 all-occurrences fix.
fn xml_sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if is_xml_illegal_char(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

/// XML 1.0's character-data legality rule (tab/LF/CR plus U+0020-U+D7FF,
/// U+E000-U+FFFD, U+10000-U+10FFFF are legal; everything else, including
/// the C0 controls other than tab/LF/CR and the BMP noncharacters
/// U+FFFE/U+FFFF, is not) -- a Rust `char` can never be a UTF-16 surrogate,
/// so that illegal range from Python's version is unreachable here and
/// intentionally omitted.
fn is_xml_illegal_char(c: char) -> bool {
    let cp = c as u32;
    (0x00..=0x08).contains(&cp)
        || (0x0B..=0x0C).contains(&cp)
        || (0x0E..=0x1F).contains(&cp)
        || (0xFFFE..=0xFFFF).contains(&cp)
}

/// Escapes the three characters XML text content requires escaped
/// (`&`, `<`, `>`) -- matching `ElementTree.tostring`'s text-escaping
/// (quotes are left literal; they only need escaping in attribute values,
/// which this module never writes).
fn xml_escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests;
