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
//! preservation, the depth guard, all-occurrences sanitization -- is
//! hand-written, mirroring `yaml.rs`/`toml.rs`'s
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
//! `local_name` instead strips a lexical `prefix:` (up to the last `:`)
//! from a tag, which coincides with Python's behavior for the common case
//! (a declared, in-scope prefix) but does not resolve prefixes through
//! `xmlns` declarations the way real namespace-aware processing would.
//! Namespaces are outside this issue's spec (the Python docstring never
//! mentions them), so this is a deliberate, disclosed simplification, not
//! a claimed parity guarantee.
//!
//! ## Text stays untyped until materialize (omnist-rs#86)
//!
//! XML's grammar carries no type information -- `<m>1</m>` and `<m>hi</m>`
//! are syntactically identical, a bare text node. Per `docs/formats/xml.md`
//! ("Text is untyped ... every leaf arrives as a string. Typing requires a
//! schema in stage 2."), [`read_xml`] builds every leaf as `Scalar::Str`
//! unconditionally, with no int/float/bool inference at parse time --
//! confirmed against a live `~/dev/venvs/omnist` `read_xml`: `<m>1</m>`
//! reads as `Scalar::Str("1")`, never `Scalar::Int((1).into())`.
//!
//! An earlier version of this module ported a `coerce()` helper that
//! type-inferred leaf text (bool/int/float) at parse time, contradicting
//! the spec and diverging from Python's reference `read_xml` -- filed and
//! fixed as omnist-rs#86 (found via the conformance harness, vector
//! `formats-xml/basic/interleaved-elements-preserve-order`). Python fixed
//! the identical bug in its own `read_xml` as `omnist#288`
//! (`_xml_to_node` no longer infers scalar kind from text shape); this
//! module's fix mirrors that commit.
//!
//! ## Schema-guided pretyping (issue #114)
//!
//! Because XML text carries no native type information and `materialize`
//! intentionally never coerces plain strings to `integer`/`number`/`boolean`
//! scalars, [`read_xml_with_schema`] performs schema-guided pretyping of
//! `boolean`, `integer`, and `number` fields before materialization,
//! mirroring Python's `_xml_pretype`. Fields typed `any`, date/time/datetime,
//! and mismatched text stay strings for normal stage-2 validation/materialization
//! reporting.
//!
//! ## Illegal characters fail the write (no substitution)
//!
//! XML 1.0 cannot represent a raw C0 control character other than tab, LF and
//! CR (nor U+FFFE/U+FFFF), and there is no substitute spelling, so a string
//! holding one fails the write unconditionally with `write.unsupported-value`
//! at that string's path (spec section 8.3.8) -- it is never dropped or
//! replaced. `scan_xml_cursor` checks every leaf through
//! `is_xml_illegal_char` before any output exists.
//!
//! ## Depth guard reuse
//!
//! [`read_xml`] checks nesting depth itself, inline, during the
//! recursive-descent walk of `quick_xml`'s pull events (mirroring Python's
//! own `_xml_to_node`, which inlines the identical check against the
//! shared `_MAX_DEPTH` constant rather than routing through
//! `document.py`'s write-side guard) -- this is necessary because,
//! without it, an adversarially deep input could blow the native Rust call
//! stack in `parse_content`'s own recursion *before* [`Doc::from_raw`]
//! ever gets a chance to reject it via
//! `crate::document::check_write_depth`. [`Doc::from_raw`] then applies
//! its own guard a second time when building the final `Doc` -- a harmless,
//! redundant confirmation of the same limit, not a second copy of the
//! guard's logic (it calls the crate's one shared `check_write_depth`).
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
use crate::document::{Cursor, Doc, MAX_DEPTH, MAX_NODES, RawNode, Scalar};
use crate::error::{DocumentError, OmnistError, ParseError};
use crate::formats::float_fmt;
use crate::formats::textpos::line_col_bytes;
use crate::report::{Severity, WriteReport};
use crate::schema::{FieldType, Resolved, ScalarKind, Schema};
use indexmap::IndexMap;
use quick_xml::Reader;
use quick_xml::events::Event;

// ============================================================== Reader

static XML_INT_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^-?(0|[1-9]\d*)$").unwrap());

static XML_NUM_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?$").unwrap()
});

fn xml_pretype_scalar(node: RawNode, s: &crate::schema::Scalar) -> RawNode {
    let RawNode::Leaf(Scalar::Str(ref val)) = node else {
        return node;
    };
    match s.kind() {
        ScalarKind::Boolean => {
            if val == "true" {
                RawNode::Leaf(Scalar::Bool(true))
            } else if val == "false" {
                RawNode::Leaf(Scalar::Bool(false))
            } else {
                node
            }
        }
        ScalarKind::Integer => {
            if XML_INT_RE.is_match(val) {
                let digits = if let Some(stripped) = val.strip_prefix('-') {
                    stripped
                } else {
                    val.as_str()
                };
                if digits.len() <= crate::formats::int_cap::MAX_INT_DIGITS {
                    let i: num_bigint::BigInt = val
                        .parse()
                        .expect("XML_INT_RE guarantees valid integer literal");
                    return RawNode::Leaf(Scalar::Int(i));
                }
            }
            node
        }
        ScalarKind::Number => {
            if XML_NUM_RE.is_match(val) {
                let digits = if let Some(stripped) = val.strip_prefix('-') {
                    stripped
                } else {
                    val.as_str()
                };
                let int_digits = digits.split(['.', 'e', 'E']).next().unwrap_or(digits);
                if int_digits.len() <= crate::formats::int_cap::MAX_INT_DIGITS {
                    let f: f64 = val
                        .parse()
                        .expect("XML_NUM_RE guarantees valid float literal");
                    return RawNode::Leaf(Scalar::Float(f));
                }
            }
            node
        }
        _ => node,
    }
}

fn xml_pretype(node: RawNode, schema: &Schema, ty: &FieldType) -> RawNode {
    let d = schema.resolve(ty);
    match d {
        Resolved::Any => node,
        Resolved::Scalar(s) => xml_pretype_scalar(node, &s),
        Resolved::Record(rec) => {
            let RawNode::Edges(edges) = node else {
                return node;
            };
            let mut out = Vec::with_capacity(edges.len());
            for (label, child) in edges {
                let pretyped_child = if let Some(field) = rec.field(&label) {
                    xml_pretype(child, schema, &field.ty)
                } else {
                    child
                };
                out.push((label, pretyped_child));
            }
            RawNode::Edges(out)
        }
    }
}

/// A data-XML profile refusal (`docs/formats/xml.md`, "The data-XML
/// profile"): the input is well-formed XML that Omnist declines to read.
///
/// A refusal is only RECORDED when met (the first one in document order
/// wins) and raised once the whole document has proved well-formed, so
/// malformed XML is always `parse.codec-syntax` and never misreported as a
/// profile refusal (E-24).
type Refusal = Option<(&'static str, String)>;

fn note_refusal(slot: &mut Refusal, code: &'static str, message: impl Into<String>) {
    if slot.is_none() {
        *slot = Some((code, message.into()));
    }
}

fn read_xml_raw(text: &str, mut report: Option<&mut WriteReport>) -> Result<RawNode, OmnistError> {
    // D-15/D-21: one leading BOM is stripped, a second is rejected at 1:1
    // (`parse.codec-syntax`, E-24). XML 1.0 admits a leading BOM, so
    // quick_xml would otherwise discard a second one silently.
    let text = crate::bom::strip_leading_bom(text)
        .map_err(|_| ParseError::codec_syntax(1, 1, crate::bom::DOUBLED_BOM_MESSAGE))?;
    let mut refusal: Refusal = None;
    let normalized = normalize_line_endings(text);
    let mut reader = Reader::from_str(&normalized);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let root_node: RawNode = loop {
        buf.clear();
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_parse_error(&reader, &normalized, &e))?;
        match ev {
            Event::Start(e) => {
                let mut node_count = 1;
                let tag = local_name(e.name());
                let path = crate::report::child_path("$", &tag, 0);
                refuse_attribute_entities(&e, &mut refusal);
                record_elem_diagnostics(&e, &path, report.as_deref_mut());
                let content = parse_content(
                    &mut reader,
                    &normalized,
                    1,
                    &mut node_count,
                    &path,
                    &mut refusal,
                    report,
                )?;
                break RawNode::Edges(vec![(tag, content)]);
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                let path = crate::report::child_path("$", &tag, 0);
                refuse_attribute_entities(&e, &mut refusal);
                record_elem_diagnostics(&e, &path, report.as_deref_mut());
                break RawNode::Edges(vec![(tag, RawNode::Leaf(Scalar::Str(String::new())))]);
            }
            Event::Eof => {
                return Err(located_error(
                    &reader,
                    &normalized,
                    "invalid XML: no root element found",
                ));
            }
            Event::Text(t) => {
                if !t.iter().all(|b| b.is_ascii_whitespace()) {
                    return Err(located_error(
                        &reader,
                        &normalized,
                        "invalid XML: unexpected text outside root element",
                    ));
                }
            }
            Event::CData(t) => {
                if !t.iter().all(|b| b.is_ascii_whitespace()) {
                    return Err(located_error(
                        &reader,
                        &normalized,
                        "invalid XML: unexpected text outside root element",
                    ));
                }
            }
            Event::GeneralRef(_) => {
                return Err(located_error(
                    &reader,
                    &normalized,
                    "invalid XML: unexpected text outside root element",
                ));
            }
            Event::DocType(_) => note_refusal(
                &mut refusal,
                "format.dtd-forbidden",
                "a DOCTYPE declaration is outside the data-XML profile (refused on sight)",
            ),
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::End(_) => {
                // Legal prolog events: skip.
            }
        }
    };

    loop {
        buf.clear();
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_parse_error(&reader, &normalized, &e))?;
        match ev {
            Event::Eof => break,
            Event::Text(t) => {
                if !t.iter().all(|b| b.is_ascii_whitespace()) {
                    return Err(located_error(
                        &reader,
                        &normalized,
                        "invalid XML: unexpected text after root element",
                    ));
                }
            }
            Event::CData(t) => {
                if !t.iter().all(|b| b.is_ascii_whitespace()) {
                    return Err(located_error(
                        &reader,
                        &normalized,
                        "invalid XML: unexpected text after root element",
                    ));
                }
            }
            Event::GeneralRef(_) => {
                return Err(located_error(
                    &reader,
                    &normalized,
                    "invalid XML: unexpected text after root element",
                ));
            }
            Event::DocType(_) => note_refusal(
                &mut refusal,
                "format.dtd-forbidden",
                "a DOCTYPE declaration is outside the data-XML profile (refused on sight)",
            ),
            Event::Comment(_) | Event::PI(_) | Event::Decl(_) | Event::End(_) => {
                // Legal epilog events: skip.
            }
            Event::Start(_) | Event::Empty(_) => {
                return Err(located_error(
                    &reader,
                    &normalized,
                    "invalid XML: multiple root elements found",
                ));
            }
        }
    }

    // Well-formedness has been established for the whole document; only now
    // is a recorded profile refusal raised. Path `$`: the refusal is about
    // the input as a whole, not one edge (spec, xml profile vectors).
    if let Some((code, message)) = refusal {
        return Err(DocumentError::with_code("$", code, message).into());
    }
    Ok(root_node)
}

/// Parse XML text into a [`Doc`], preserving element order/interleaving
/// exactly (see this module's doc comment).
pub fn read_xml(text: &str) -> Result<Doc, OmnistError> {
    let raw = read_xml_raw(text, None)?;
    let doc = Doc::from_raw(raw)?;
    Ok(doc)
}

/// Same as [`read_xml`], but also reports `format.attribute-dropped` and
/// `format.namespace-dropped` adjustments (spec Sec8.3.8, D-3) into
/// `report` for every element that had an attribute or a namespace prefix
/// discarded, mirroring the write-side `report: Option<&mut WriteReport>`
/// pattern every writer in this crate already uses -- see
/// `crate::report`'s module doc. `report: None` behaves exactly like
/// [`read_xml`].
pub fn read_xml_report(text: &str, report: Option<&mut WriteReport>) -> Result<Doc, OmnistError> {
    let raw = read_xml_raw(text, report)?;
    let doc = Doc::from_raw(raw)?;
    Ok(doc)
}

/// Parse XML text into a [`Doc`] with schema-guided pretyping of boolean,
/// integer, and number scalar fields (spec §2.2 / issue #114).
pub fn read_xml_with_schema(text: &str, schema: &Schema) -> Result<Doc, OmnistError> {
    let raw = read_xml_raw(text, None)?;
    let pretyped = xml_pretype(raw, schema, &FieldType::Ref(schema.root().clone()));
    let doc = Doc::from_raw(pretyped)?;
    Ok(doc)
}

/// Reads the content of an already-opened element (the matching `Start`
/// event has already been consumed by the caller) up to and including its
/// `End` event, returning either an internal node ([`RawNode::Edges`], if
/// it has child elements) or a leaf ([`RawNode::Leaf`], its untyped text
/// verbatim as a [`Scalar::Str`]), mirroring Python's `_xml_to_node`.
fn parse_content(
    reader: &mut Reader<&[u8]>,
    source: &str,
    depth: usize,
    node_count: &mut usize,
    path: &str,
    refusal: &mut Refusal,
    mut report: Option<&mut WriteReport>,
) -> Result<RawNode, OmnistError> {
    if depth > MAX_DEPTH {
        return Err(DocumentError::with_code(
            "$",
            "document.limit.depth",
            format!("nesting exceeds the maximum depth ({MAX_DEPTH})"),
        )
        .into());
    }
    let mut text = String::new();
    let mut children: Vec<(String, RawNode)> = Vec::new();
    // Same-label occurrence counts, tracked incrementally (O(1) amortized
    // per child) rather than by rescanning children on every element via
    // children.iter().filter(...).count(), which is O(n) per element and
    // made a MAX_NODES-sized sibling run O(n^2).
    let mut label_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_parse_error(reader, source, &e))?;
        match ev {
            Event::Start(e) => {
                *node_count += 1;
                if *node_count > MAX_NODES {
                    return Err(DocumentError::with_code(
                        "$",
                        "document.limit.nodes",
                        format!("document exceeds the maximum node count ({MAX_NODES})"),
                    )
                    .into());
                }
                let tag = local_name(e.name());
                let index = *label_counts
                    .entry(tag.clone())
                    .and_modify(|n| *n += 1)
                    .or_insert(0);
                let child_path = crate::report::child_path(path, &tag, index);
                refuse_attribute_entities(&e, refusal);
                record_elem_diagnostics(&e, &child_path, report.as_deref_mut());
                let child = parse_content(
                    reader,
                    source,
                    depth + 1,
                    node_count,
                    &child_path,
                    refusal,
                    report.as_deref_mut(),
                )?;
                children.push((tag, child));
            }
            Event::Empty(e) => {
                *node_count += 1;
                if *node_count > MAX_NODES {
                    return Err(DocumentError::with_code(
                        "$",
                        "document.limit.nodes",
                        format!("document exceeds the maximum node count ({MAX_NODES})"),
                    )
                    .into());
                }
                let tag = local_name(e.name());
                let index = *label_counts
                    .entry(tag.clone())
                    .and_modify(|n| *n += 1)
                    .or_insert(0);
                let child_path = crate::report::child_path(path, &tag, index);
                refuse_attribute_entities(&e, refusal);
                record_elem_diagnostics(&e, &child_path, report.as_deref_mut());
                children.push((tag, RawNode::Leaf(Scalar::Str(String::new()))));
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
                if let Some(c) = resolve_general_ref(reader, source, &e, refusal)? {
                    text.push(c);
                }
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
            note_refusal(
                refusal,
                "format.mixed-content",
                "mixed content (text alongside child elements) is outside the data-XML profile",
            );
        }
        Ok(RawNode::Edges(children))
    } else {
        Ok(RawNode::Leaf(Scalar::Str(text)))
    }
}

/// Builds a [`ParseError`] located at the reader's current byte position
/// (converted to a 1-based line/column via [`line_col_bytes`]), for the error
/// conditions this module detects itself rather than receiving from
/// `quick_xml` (unexpected EOF, mixed content).
fn located_error(reader: &Reader<&[u8]>, source: &str, message: &str) -> OmnistError {
    let pos = (reader.buffer_position() as usize).min(source.len());
    let (line, col) = line_col_bytes(source, pos);
    ParseError::codec_syntax(line, col, message).into()
}

/// Records `format.attribute-dropped` and `format.namespace-dropped`
/// (spec Sec8.3.8, D-3) for one just-opened element (`Start`/`Empty`
/// event), at `path` -- the path of the element itself, matching the
/// vectors' convention (the element the attribute/prefix was lost *from*,
/// not its parent or child). A no-op when `report` is `None`, matching
/// every other `Option<&mut WriteReport>` consumer in this crate.
fn record_elem_diagnostics(
    e: &quick_xml::events::BytesStart<'_>,
    path: &str,
    report: Option<&mut WriteReport>,
) {
    let Some(rep) = report else { return };
    if e.attributes().next().is_some() {
        rep.add(
            path,
            "format.attribute-dropped",
            "an XML attribute was discarded on read",
            Severity::Warning,
        );
    }
    let name = e.name();
    let raw = std::str::from_utf8(name.as_ref()).unwrap_or_default();
    if raw.contains(':') {
        rep.add(
            path,
            "format.namespace-dropped",
            "an XML namespace prefix was discarded on read",
            Severity::Warning,
        );
    }
}

/// The data-XML profile refuses an entity reference other than the five
/// predefined ones "on sight, not on use", wherever it stands -- an
/// attribute value included, although attributes are otherwise dropped on
/// read. quick_xml reports entity references as their own events only in
/// text, so attribute values are scanned here. A numeric character reference
/// and the five predefined entities stay legal. The refusal is only
/// RECORDED (raised after well-formedness, like the others); an attribute
/// quick_xml cannot parse is ignored here because the element's own
/// well-formedness is checked by the reader.
fn refuse_attribute_entities(e: &quick_xml::events::BytesStart<'_>, refusal: &mut Refusal) {
    for attr in e.attributes().filter_map(Result::ok) {
        let value = String::from_utf8_lossy(&attr.value);
        let mut rest: &str = &value;
        while let Some(at) = rest.find('&') {
            rest = &rest[at + 1..];
            let Some(end) = rest.find(';') else { break };
            let name = &rest[..end];
            if !name.starts_with('#') && !matches!(name, "lt" | "gt" | "amp" | "apos" | "quot") {
                note_refusal(
                    refusal,
                    "format.entity-forbidden",
                    format!(
                        "entity reference '&{name};' in an attribute value is outside the \
                         data-XML profile (only the five predefined XML entities and numeric \
                         character references are read)"
                    ),
                );
                return;
            }
            rest = &rest[end + 1..];
        }
    }
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
    ParseError::codec_syntax(line, col, format!("invalid XML: {e}")).into()
}

/// Resolves an `Event::GeneralRef` (a `&...;` entity or character
/// reference `quick_xml` tokenizes as its own event, separate from
/// `Text`) to the single `char` it denotes. Handles a numeric character
/// reference (`&#65;`/`&#x41;`) via the crate's own [`quick_xml::events::BytesRef::resolve_char_ref`],
/// and the five predefined XML entities by name -- `quick_xml` has no
/// DTD support, so no other named entity can ever legitimately appear
/// (see this module's doc comment on why that's a security feature, not
/// a gap).
///
/// An entity reference outside XML's five predefined ones is a data-XML
/// profile refusal (`format.entity-forbidden`), not malformed input: it is
/// recorded in `refusal` (raised once the document proves well-formed) and
/// `Ok(None)` is returned so the scan continues.
fn resolve_general_ref(
    reader: &Reader<&[u8]>,
    source: &str,
    e: &quick_xml::events::BytesRef<'_>,
    refusal: &mut Refusal,
) -> Result<Option<char>, OmnistError> {
    if let Some(ch) = e
        .resolve_char_ref()
        .map_err(|err| xml_parse_error(reader, source, &err))?
    {
        return Ok(Some(ch));
    }
    let name = e
        .decode()
        .expect("Reader::from_str fixes the decoder to UTF-8; decode() cannot fail");
    match name.as_ref() {
        "lt" => Ok(Some('<')),
        "gt" => Ok(Some('>')),
        "amp" => Ok(Some('&')),
        "apos" => Ok(Some('\'')),
        "quot" => Ok(Some('"')),
        other => {
            note_refusal(
                refusal,
                "format.entity-forbidden",
                format!(
                    "entity reference '&{other};' is outside the data-XML profile (only the \
                     five predefined XML entities and numeric character references are read)"
                ),
            );
            Ok(None)
        }
    }
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
    let root = doc.root();
    let Ok(edges) = root.internal_edges() else {
        return Err(single_root_error());
    };
    if edges.len() > 1 {
        return Err(multiple_roots_error());
    }
    if edges.len() != 1 {
        return Err(single_root_error());
    }
    let mut rep = WriteReport::new();
    scan_xml_cursor(&root, "$", &mut rep, true)?;
    let (tag, child_id) = &edges[0];
    let child_cursor = root.seek(*child_id);
    let mut out = String::new();
    write_element(tag, &child_cursor, 0, &mut out);
    if !matches!(child_cursor.internal_edges(), Ok(e) if !e.is_empty()) && out.ends_with('\n') {
        out.pop();
    }
    crate::report::finish_write(out, rep, strict, report)
}

/// `format.multiple-roots` (spec section 8.3.8): a Document with several
/// top-level edges cannot be written as XML. Path `$`, the whole Document.
fn multiple_roots_error() -> WriteError {
    WriteError::with_diagnostic(
        "$",
        "format.multiple-roots",
        "XML needs exactly one document element; the root node has several top-level edges (a multi-rooted Document)",
    )
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
    // `fail_fast: false` -- `scan_xml_cursor` never returns `Err` on this
    // path, it only records the same conditions as `write.unsupported-value`
    // `Severity::Error` adjustments for preview purposes (`check_xml` never
    // produces output to begin with, so there is nothing to fail).
    scan_xml_cursor(&doc.root(), "$", &mut rep, false).expect("fail_fast: false never returns Err");
    rep
}

/// Marker type implementing [`crate::formats::Codec`] for XML -- adapts
/// [`read_xml`]/[`write_xml`]/[`check_xml`] to the registry's uniform
/// shape with the documented defaults (`strict: false`, no report). The
/// single-document-element root-shape error `write_xml` raises fires from
/// inside `write_xml` itself, outside `finish_write` and before any
/// scanning, exactly as before -- this impl only calls `write_xml`, it
/// doesn't reimplement it.
pub(crate) struct Xml;

impl crate::formats::Codec for Xml {
    const NAME: &'static str = "xml";

    fn read(text: &str) -> Result<Doc, OmnistError> {
        read_xml(text)
    }

    fn write(doc: &Doc) -> Result<String, OmnistError> {
        write_xml(doc, false, None).map_err(Into::into)
    }

    fn check(doc: &Doc) -> WriteReport {
        check_xml(doc)
    }
}

/// Scans a subtree for every write-time adjustment/failure XML has, mirrored
/// against `check_xml`'s preview-only need via `fail_fast`.
///
/// Two conditions -- an XML-illegal label and an empty internal node --
/// fail the write unconditionally (`write.unsupported-value`, spec
/// Sec8.3.8/Sec8.3.9 updated 2026-08-24) rather than sanitizing/substituting
/// and reporting a warning; retired the `key.sanitized`/`shape.empty_ambiguous`
/// codes (see this module's doc comment and `write_xml`'s). With
/// `fail_fast: true` (the real [`write_xml`] path), this function returns
/// `Err` the moment either condition is found, before any output is
/// produced -- so `write_element`'s own `xml_name`-sanitizing branch is now
/// unreachable in practice and has been removed; every label `write_element`
/// ever sees has already been confirmed a valid XML name here. With
/// `fail_fast: false` ([`check_xml`]'s preview-only path), both conditions
/// are instead recorded as `write.unsupported-value`/`Severity::Error`
/// adjustments and scanning continues, so `check_xml` can report every
/// occurrence in one pass rather than just the first.
fn scan_xml_cursor(
    cursor: &Cursor,
    path: &str,
    rep: &mut WriteReport,
    fail_fast: bool,
) -> Result<(), WriteError> {
    match cursor.internal_edges() {
        Ok(edges) => {
            if edges.is_empty() {
                let detail = "empty internal node (no edges) has no XML representation -- it \
                              would read back as the empty-string leaf '', indistinguishable \
                              from a genuine empty string";
                if fail_fast {
                    return Err(crate::report::unsupported_value_error(path, detail));
                }
                rep.add(path, "write.unsupported-value", detail, Severity::Error);
                return Ok(());
            }
            let mut counts: IndexMap<&str, usize> = IndexMap::new();
            for (label, child_id) in edges {
                let entry = counts.entry(label.as_str()).or_insert(0);
                let i = *entry;
                *entry += 1;
                let p = crate::report::child_path(path, label, i);
                if !is_valid_xml_name(label) {
                    let detail =
                        format!("label {label:?} is not a valid XML name and cannot be written");
                    if fail_fast {
                        return Err(crate::report::unsupported_value_error(&p, detail));
                    }
                    rep.add(
                        p.clone(),
                        "write.unsupported-value",
                        detail,
                        Severity::Error,
                    );
                }
                let child = cursor.seek(*child_id);
                scan_xml_cursor(&child, &p, rep, fail_fast)?;
            }
        }
        Err(_) => {
            let scalar = cursor.value().unwrap();
            // A string holding a character XML 1.0 cannot represent (a C0
            // control other than tab/LF/CR, U+FFFE, U+FFFF) fails the write
            // unconditionally: there is no substitute spelling (spec
            // section 8.3.8, the string-illegal-char condition).
            if let Scalar::Str(s) = scalar
                && let Some(bad) = s.chars().find(|&c| is_xml_illegal_char(c))
            {
                let detail = format!(
                    "string contains U+{:04X}, which XML 1.0 cannot represent and has no \
                     substitute spelling",
                    bad as u32
                );
                if fail_fast {
                    return Err(crate::report::unsupported_value_error(path, detail));
                }
                rep.add(path, "write.unsupported-value", detail, Severity::Error);
                return Ok(());
            }
            scan_leaf(scalar, path, rep);
        }
    }
    Ok(())
}

fn scan_leaf(scalar: &Scalar, path: &str, rep: &mut WriteReport) {
    match scalar {
        Scalar::Null => rep.add(
            path,
            "null.omitted",
            "null written as an empty element",
            Severity::Warning,
        ),
        // omnist-rs#86: read_xml no longer infers scalar kind from
        // element-text shape, so a non-string scalar written to XML (XML
        // has no native typed literals -- everything is text) now reads
        // back as a string, not its original type. Previously silent
        // (the old shape-based coercion happened to undo this on read);
        // now reported like every other type-losing write, matching
        // Python's identical fix (`omnist#288`, `value.stringified`).
        Scalar::Bool(_)
        | Scalar::Int(_)
        | Scalar::Float(_)
        | Scalar::Date(_)
        | Scalar::Time(_)
        | Scalar::Datetime(_) => rep.add(
            path,
            "value.stringified",
            "non-string scalar written as text (reads back as a string)",
            Severity::Warning,
        ),
        Scalar::Str(_) => {}
    }
    // `string.cr_normalized` retired (spec Sec8.3.8, issue #162): a
    // literal CR is no longer written raw and reported lossy -- it is
    // escaped as the numeric character reference `&#13;`, which is exempt
    // from XML's mandatory line-ending normalization on parse and
    // round-trips losslessly. See `xml_escape_text`. Nothing left to
    // report here for CR. A string holding a character XML cannot
    // represent never reaches this function: `scan_xml_cursor` fails the
    // write (or records it, for `check_xml`) before calling it.
}

/// `tag` is always already a valid XML name by the time this runs -- the
/// only two callers are `write_xml` (which fails via `scan_xml_cursor`'s
/// `fail_fast: true` pass, before this function is ever reached, on any
/// label that isn't) and this function's own recursive call on a child
/// label already scanned the same way. No sanitization happens here
/// anymore -- see `scan_xml_cursor`'s doc comment on why the write now
/// fails unconditionally on an XML-illegal label instead.
fn write_element(tag: &str, content: &Cursor, level: usize, out: &mut String) {
    let indent = "  ".repeat(level);
    out.push_str(&indent);
    out.push('<');
    out.push_str(tag);
    match content.internal_edges() {
        Ok(edges) if !edges.is_empty() => {
            out.push_str(">\n");
            for (label, child_id) in edges {
                let child = content.seek(*child_id);
                write_element(label, &child, level + 1, out);
            }
            out.push_str(&indent);
            out.push_str("</");
            out.push_str(tag);
            out.push_str(">\n");
        }
        // Unreachable via the real `write_xml` path: `scan_xml_cursor`'s
        // `fail_fast: true` pass already returned `Err` for any empty
        // internal node anywhere in the tree before `write_element` is
        // ever called (spec Sec8.3.8/Sec8.3.9, issue #161) -- see that
        // function's doc comment. White-box confirmed directly below
        // (`write_element_panics_on_empty_internal_node`), same rationale
        // as `toml.rs`'s/`yaml.rs`'s identical `unreachable!()` precedents.
        Ok(_) => unreachable!(
            "write_element is never called on an empty internal node -- scan_xml_cursor \
             already failed the write"
        ),
        Err(_) => {
            let scalar = content.value().unwrap();
            let text = xml_text(scalar);
            if text.is_empty() {
                out.push_str(" />\n");
            } else {
                out.push('>');
                out.push_str(&xml_escape_text(&text));
                out.push_str("</");
                out.push_str(tag);
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
        Scalar::Str(s) | Scalar::Date(s) | Scalar::Time(s) | Scalar::Datetime(s) => s.clone(),
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
    float_fmt::float_to_string(x, "nan", "inf", "-inf")
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
/// (`&`, `<`, `>`), plus (spec Sec8.3.8, issue #162) a literal carriage
/// return as the numeric character reference `&#13;` -- matching
/// `ElementTree.tostring`'s text-escaping for the first three (quotes are
/// left literal; they only need escaping in attribute values, which this
/// module never writes), and going beyond it for CR: XML mandates
/// line-ending normalization on parse, so a raw CR byte and a raw LF byte
/// read back identical (confirmed live) -- a numeric character reference
/// is exempt from that normalization and round-trips intact, so a string
/// containing a bare CR writes as `a&#13;b`, and a string containing a
/// CRLF sequence writes as `a&#13;\nb`, not the raw bytes.
fn xml_escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests;
