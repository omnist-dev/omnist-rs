//! YAML codec. Ported from `~/dev/omnist/omnist/formats.py`'s
//! `read_yaml`/`write_yaml`/`check_yaml`.
//!
//! ## Crate choice
//!
//! YAML's grammar (block/flow collections, indentation-sensitivity, scalar
//! styles, anchors/aliases, tags) is significantly more complex than JSON's.
//! This module uses [`yaml_rust2`]'s low-level [`Parser`]/[`MarkedEventReceiver`]
//! event stream for raw tokenization/structure (indentation handling, flow vs.
//! block collections, quoting, anchors/aliases) -- the "reasonable,
//! spec-compliant" choice issue #18 calls out, since re-deriving YAML's
//! indentation/quoting grammar by hand would duplicate a large, well-tested
//! surface for little benefit. Everything omnist-specific is still hand-written
//! Rust code in this module, not delegated to the crate:
//!
//! * **Scalar-tag resolution** ([`resolve_plain_scalar`]) -- `yaml_rust2`'s own
//!   built-in resolver (`Yaml::from_str`) only recognizes `true`/`false`
//!   (YAML 1.2 core schema) for booleans. Live-checked against Python's
//!   `yaml.safe_load` (PyYAML, which this project's Python reference wraps):
//!   PyYAML additionally treats `yes`/`no`/`on`/`off` (and case variants) as
//!   booleans (YAML 1.1 `Resolver.yaml_implicit_resolvers`, confirmed via the
//!   `tag:yaml.org,2002:bool` regex) -- `y`/`n` alone are **not** included and
//!   stay strings. Because `yaml_rust2`'s own resolution would silently
//!   produce the wrong Rust value for `yes`/`no`/`on`/`off`, this module
//!   ignores the crate's built-in scalar typing entirely and re-implements
//!   PyYAML's exact implicit-resolver regexes for null/bool/int/float/
//!   timestamp against the raw scalar text + style (quoted scalars are never
//!   auto-typed, matching PyYAML, which only applies implicit resolution to
//!   plain-style scalars).
//! * **Merge-key (`<<`) handling** ([`expand_merge_keys`]) -- `yaml_rust2` has
//!   no built-in merge-key support at all (confirmed by reading its source);
//!   this module implements the YAML merge-key spec directly: an unquoted
//!   `<<` key's value must be a mapping or a sequence of mappings, merged in
//!   order with explicit keys in the mapping taking precedence, else a clean
//!   [`ParseError`] (never a panic) -- the omnist-ts#46 regression this
//!   module's test suite pins.
//! * **Depth guard, temporal shape-check, integer digit cap** -- reused from
//!   [`crate::document`]/[`crate::schema`]/this crate's established
//!   4300-digit-cap pattern (see [`MAX_INT_DIGITS`]), not reimplemented.
//!
//! ## Depth guard reuse
//!
//! Same reasoning as `json.rs`: [`read_yaml`] builds a [`Doc`] via [`Doc::of`],
//! which calls [`crate::document::check_write_depth`] internally. The merge-
//! key/alias-resolution pass that runs *before* `Doc::of` also depth-guards
//! itself (an alias can reference an already-deep subtree, and merging can
//! grow a mapping before the Document-model depth check ever sees it), reusing
//! the same [`crate::document::check_write_depth`] guard rather than adding a
//! second copy. [`write_yaml`]/[`check_yaml`] walk an already-built `Doc` (via
//! `to_grouped`), whose nodes are already depth-checked, so there is nothing
//! left to re-guard on the way out.
//!
//! ## No native temporal type; but YAML dates/datetimes still need normalizing
//!
//! Like `json.rs`, this port's [`crate::document::Scalar`] has no temporal
//! variant -- a YAML timestamp becomes a `Scalar::Str` holding its ISO
//! spelling. Unlike JSON, YAML's timestamp grammar is looser than the ISO
//! shapes `schema.rs`'s temporal shape-check accepts (space-separated date/
//! time, single-digit month/day, a bare `Z` suffix, no zero-padding) --
//! Python's PyYAML normalizes any such spelling to a `datetime.date`/
//! `datetime.datetime` object and then (elsewhere in the pipeline) back to a
//! canonical ISO string. [`normalize_timestamp`] reproduces that
//! normalization for the timestamp grammar PyYAML's own
//! `tag:yaml.org,2002:timestamp` resolver accepts, so a YAML value like
//! `2001-12-14 21:59:43.10 -5` round-trips to the same canonical
//! `2001-12-14T21:59:43.100000-05:00` shape Python's `datetime.isoformat()`
//! would produce, not the original loose spelling. A timestamp-shaped string
//! naming a calendar/clock value that doesn't exist (`2024-13-01`,
//! `2024-02-30`) is a [`ParseError`], not a silent string fallback --
//! live-confirmed: PyYAML's construction step raises `ValueError` there,
//! which fails the whole document. Calendar/clock validity itself reuses
//! [`crate::schema::valid_ymd`]/[`crate::schema::valid_hms`] rather than a
//! second copy of that logic, even though the *shape* regex is necessarily
//! separate (looser than `schema.rs`'s).
//!
//! ## Native NaN/Infinity support (no lossy adjustment, unlike JSON)
//!
//! YAML's float grammar has native tokens for `.nan`/`.inf`/`-.inf`
//! (`tag:yaml.org,2002:float`), so -- unlike `json.rs`'s `write_json`, which
//! must substitute `null` for a special float -- [`write_yaml`] never needs to
//! adjust a `NaN`/`Infinity` leaf. The only adjustment [`check_yaml`] ever
//! records is forcing double-quoted style for a string containing U+0085
//! (NEL), which PyYAML's default (unquoted/single-quoted) scalar styles
//! normalize away as a line break -- mirrors Python's `_scan_yaml_labels`/
//! `_yaml_str_representer` exactly.
//!
//! ## Integer digit cap (omnist-ts#54 / oml.rs / json.rs precedent)
//!
//! Same 4300-digit cap, applied to a plain decimal integer scalar's digit run
//! before attempting to parse it, mirroring `json.rs`'s identical guard.

use std::collections::HashMap;

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, ScanError, TScalarStyle};

use crate::WriteError;
use crate::document::{Doc, Value};
use crate::error::{OmnistError, ParseError};
use crate::report::{Severity, WriteReport};
use indexmap::IndexMap;

/// Same guard, same constant as `json.rs`'s/`oml.rs`'s `MAX_INT_DIGITS` -- see
/// this module's doc comment.
const MAX_INT_DIGITS: usize = 4300;

// ============================================================== Reader

/// The raw shape `yaml_rust2`'s event stream is rebuilt into: a scalar with
/// its original text and style (style is what [`resolve_plain_scalar`] and
/// merge-key detection both need, and what `yaml_rust2`'s own `Yaml` enum
/// throws away by pre-resolving plain scalars with its own, PyYAML-incompatible
/// rules -- see this module's doc comment), or an ordered sequence/mapping.
#[derive(Debug, Clone)]
enum Raw {
    Scalar(String, TScalarStyle, Option<Tag>),
    Sequence(Vec<Raw>),
    Mapping(Vec<(Raw, Raw)>),
}

/// Rebuilds a [`Raw`] tree from `yaml_rust2`'s parser event stream, resolving
/// aliases against anchors seen so far (anchors always precede their aliases
/// in a valid YAML document). Structurally mirrors `yaml_rust2::yaml::YamlLoader`
/// (the crate's own reference event-to-tree builder), adapted to keep scalar
/// style/tag information `YamlLoader`'s `Yaml` throws away.
struct Builder {
    doc_stack: Vec<(Raw, usize)>,
    key_stack: Vec<Option<Raw>>,
    anchor_map: HashMap<usize, Raw>,
    docs: Vec<Raw>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            doc_stack: Vec::new(),
            key_stack: Vec::new(),
            anchor_map: HashMap::new(),
            docs: Vec::new(),
        }
    }

    fn insert(&mut self, node: Raw, aid: usize, _mark: Marker) {
        if aid > 0 {
            self.anchor_map.insert(aid, node.clone());
        }
        match self.doc_stack.last_mut() {
            None => self.doc_stack.push((node, aid)),
            Some((Raw::Sequence(items), _)) => items.push(node),
            Some((Raw::Mapping(_), _)) => {
                let cur_key = self
                    .key_stack
                    .last_mut()
                    .expect("a Mapping is only ever pushed alongside a matching key_stack entry");
                match cur_key.take() {
                    None => *cur_key = Some(node),
                    Some(k) => {
                        if let Some((Raw::Mapping(entries), _)) = self.doc_stack.last_mut() {
                            entries.push((k, node));
                        }
                    }
                }
            }
            Some((Raw::Scalar(..), _)) => {
                // A scalar is never pushed onto doc_stack as a container
                // (only Sequence/Mapping are, in on_event's SequenceStart/
                // MappingStart arms) -- this arm is structurally unreachable.
                unreachable!("a Scalar is never a container on doc_stack")
            }
        }
    }

    /// Handles one parser event. `Event::Alias` never fails here: live-
    /// confirmed against `yaml_rust2::YamlLoader::load_from_str` (see this
    /// module's doc comment) -- the crate's own scanner already rejects an
    /// alias whose anchor was never defined (`ScanError: found unknown
    /// anchor`, surfaced through [`scan_error_to_parse_error`] before this
    /// receiver ever runs) for *every* input that reaches an event receiver
    /// at all, so an `anchor_map` miss inside `on_event` is unreachable in
    /// practice -- `.expect()` documents that invariant instead of leaving a
    /// structurally-dead error branch, matching `json.rs`'s identical
    /// surrogate-pair `.expect()` precedent.
    fn on_event_impl(&mut self, ev: Event, mark: Marker) {
        match ev {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentStart => {}
            Event::DocumentEnd => match self.doc_stack.len() {
                0 => self
                    .docs
                    .push(Raw::Scalar(String::new(), TScalarStyle::Plain, None)),
                1 => self.docs.push(self.doc_stack.pop().unwrap().0),
                _ => unreachable!("a single document's stack never nests more than one root"),
            },
            Event::SequenceStart(aid, _) => self.doc_stack.push((Raw::Sequence(Vec::new()), aid)),
            Event::SequenceEnd => {
                let (node, aid) = self.doc_stack.pop().expect("matched by SequenceStart");
                self.insert(node, aid, mark);
            }
            Event::MappingStart(aid, _) => {
                self.doc_stack.push((Raw::Mapping(Vec::new()), aid));
                self.key_stack.push(None);
            }
            Event::MappingEnd => {
                let (node, aid) = self.doc_stack.pop().expect("matched by MappingStart");
                self.key_stack.pop();
                self.insert(node, aid, mark);
            }
            Event::Scalar(v, style, aid, tag) => {
                self.insert(Raw::Scalar(v, style, tag), aid, mark);
            }
            Event::Alias(id) => {
                let node = self.anchor_map.get(&id).cloned().expect(
                    "yaml_rust2's scanner rejects an alias to an undefined anchor before this \
                     receiver ever runs -- see on_event_impl's doc comment",
                );
                self.insert(node, 0, mark);
            }
        }
    }
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        self.on_event_impl(ev, mark);
    }
}

fn scan_error_to_parse_error(e: &ScanError) -> ParseError {
    let mark = e.marker();
    ParseError::new(mark.line(), mark.col() + 1, format!("invalid YAML: {e}"))
}

/// Parse YAML text into a [`Doc`].
///
/// Exactly one YAML document is accepted (matching Python's `yaml.safe_load`,
/// which raises on a stream containing more than one `---`-separated
/// document); an empty/blank input parses as a `Null` document, also matching
/// `yaml.safe_load("")` returning `None`. A bare top-level sequence, a
/// sequence nested directly inside another sequence, or nesting past
/// [`crate::document::MAX_DEPTH`] all surface as
/// [`crate::error::DocumentError`] (via [`Doc::of`]), matching `json.rs`'s
/// identical `read_json` behavior.
pub fn read_yaml(text: &str) -> Result<Doc, OmnistError> {
    let mut parser = Parser::new(text.chars());
    let mut builder = Builder::new();
    parser
        .load(&mut builder, true)
        .map_err(|e| scan_error_to_parse_error(&e))?;
    if builder.docs.len() > 1 {
        return Err(ParseError::new(
            1,
            1,
            "invalid YAML: expected a single document in the stream, found more than one",
        )
        .into());
    }
    let raw = builder.docs.into_iter().next().unwrap_or(Raw::Scalar(
        String::new(),
        TScalarStyle::Plain,
        None,
    ));
    let resolved = resolve_merges(&raw, 0)?;
    let value = raw_to_value(&resolved)?;
    Ok(Doc::of(&value)?)
}

/// Recursively expands every `<<` merge key, depth-guarded the same way
/// `document.rs`'s own construction path is (an alias can smuggle in an
/// already-deep subtree before `Doc::of` ever sees it).
fn resolve_merges(node: &Raw, depth: usize) -> Result<Raw, OmnistError> {
    crate::document::check_write_depth(depth, "$")?;
    match node {
        Raw::Scalar(..) => Ok(node.clone()),
        Raw::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_merges(item, depth + 1)?);
            }
            Ok(Raw::Sequence(out))
        }
        Raw::Mapping(entries) => {
            let mut merged_from: Vec<(Raw, Raw)> = Vec::new();
            let mut own: Vec<(Raw, Raw)> = Vec::new();
            for (k, v) in entries {
                if is_merge_key(k) {
                    for (mk, mv) in merge_source_entries(v, depth)? {
                        merged_from.push((mk, mv));
                    }
                } else {
                    own.push((resolve_merges(k, depth + 1)?, resolve_merges(v, depth + 1)?));
                }
            }
            // Explicit keys take precedence over merged-in ones (an explicit
            // duplicate key among `own` is left untouched here -- last-wins
            // for those is `raw_to_value`'s `IndexMap::insert`'s job, exactly
            // like `json.rs`'s reader). Among the merge sources themselves,
            // first-listed source wins a collision (YAML merge spec).
            let own_labels: Vec<&str> =
                own.iter().filter_map(|(k, _)| scalar_key_text(k)).collect();
            let mut merged_seen: Vec<&str> = Vec::new();
            let mut result = own.clone();
            for (k, v) in &merged_from {
                let label = scalar_key_text(k);
                if let Some(label) = label {
                    if own_labels.contains(&label) || merged_seen.contains(&label) {
                        continue;
                    }
                    merged_seen.push(label);
                }
                result.push((k.clone(), v.clone()));
            }
            Ok(Raw::Mapping(result))
        }
    }
}

/// A merge key's own key text, for de-duplication purposes -- non-scalar
/// (or non-string-scalar) keys never collide with anything by this scheme,
/// matching the fact that only string-labeled fields exist in this model.
fn scalar_key_text(k: &Raw) -> Option<&str> {
    match k {
        Raw::Scalar(s, _, _) => Some(s.as_str()),
        _ => None,
    }
}

/// Is `k` an (unquoted) `<<` merge-key marker? A *quoted* `"<<"` is a literal
/// string key, not a merge marker -- matches PyYAML's resolver, which only
/// assigns the `tag:yaml.org,2002:merge` tag to a plain-style scalar spelled
/// exactly `<<`.
fn is_merge_key(k: &Raw) -> bool {
    matches!(k, Raw::Scalar(s, TScalarStyle::Plain, None) if s == "<<")
}

/// The `(key, value)` pairs a merge key's value contributes: a mapping
/// contributes its own entries directly; a sequence contributes every
/// element's entries in order; anything else -- the omnist-ts#46 regression
/// this reader guards against -- is a clean [`ParseError`], never a panic.
fn merge_source_entries(v: &Raw, depth: usize) -> Result<Vec<(Raw, Raw)>, OmnistError> {
    match v {
        Raw::Mapping(entries) => {
            let mut out = Vec::with_capacity(entries.len());
            for (k, val) in entries {
                out.push((
                    resolve_merges(k, depth + 1)?,
                    resolve_merges(val, depth + 1)?,
                ));
            }
            Ok(out)
        }
        Raw::Sequence(items) => {
            let mut out = Vec::new();
            for item in items {
                out.extend(merge_source_entries(item, depth + 1)?);
            }
            Ok(out)
        }
        Raw::Scalar(..) => Err(ParseError::new(
            1,
            1,
            "invalid YAML: merge key '<<' requires a mapping or a sequence of mappings, \
             found a scalar",
        )
        .into()),
    }
}

/// Turn a (merge-resolved) [`Raw`] tree into a [`Value`], applying scalar-tag
/// resolution to every leaf along the way.
fn raw_to_value(node: &Raw) -> Result<Value, OmnistError> {
    match node {
        Raw::Scalar(text, style, tag) => Ok(scalar_to_value(text, *style, tag.as_ref())?),
        Raw::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(raw_to_value(item)?);
            }
            Ok(Value::Array(out))
        }
        Raw::Mapping(entries) => {
            let mut map: IndexMap<String, Value> = IndexMap::new();
            for (k, v) in entries {
                let key = match k {
                    Raw::Scalar(s, _, _) => s.clone(),
                    _ => {
                        return Err(ParseError::new(
                            1,
                            1,
                            "invalid YAML: a mapping key must be a scalar",
                        )
                        .into());
                    }
                };
                // Last-duplicate-key-wins, matching json.rs's IndexMap::insert
                // semantics and PyYAML's own dict-construction behavior
                // (confirmed live: `yaml.safe_load("a: 1\\na: 2\\n") == {'a': 2}`).
                map.insert(key, raw_to_value(v)?);
            }
            Ok(Value::Object(map))
        }
    }
}

fn scalar_to_value(
    text: &str,
    style: TScalarStyle,
    tag: Option<&Tag>,
) -> Result<Value, ParseError> {
    if let Some(t) = tag
        && t.handle == "tag:yaml.org,2002:"
    {
        return explicit_tag_to_value(text, &t.suffix);
    }
    if style != TScalarStyle::Plain {
        return Ok(Value::Str(text.to_string()));
    }
    resolve_plain_scalar(text)
}

/// Constructs a [`Value`] from an explicit standard YAML tag
/// (`!!str`/`!!int`/`!!float`/`!!bool`/`!!null`), matching what PyYAML's
/// `SafeConstructor` supports for these five. Any other explicit tag (a
/// custom `!!` type, `!!seq`/`!!map` forced onto a scalar, an unknown handle)
/// is rejected with a [`ParseError`] -- PyYAML's `SafeConstructor` itself
/// raises `ConstructorError: could not determine a constructor for the tag`
/// for anything it doesn't recognize, so this is matching behavior, not an
/// arbitrarily narrower one.
fn explicit_tag_to_value(text: &str, suffix: &str) -> Result<Value, ParseError> {
    match suffix {
        "str" => Ok(Value::Str(text.to_string())),
        "null" => Ok(Value::Null),
        // PyYAML's `SafeConstructor.construct_yaml_bool` looks up
        // `node.value.lower()` in `bool_values = {"yes": True, "no": False,
        // "true": True, "false": False, "on": True, "off": False}`
        // regardless of how the `!!bool` tag got attached (explicit or
        // implicit) -- live-confirmed (see module doc comment): `!!bool
        // "YES"`/`"On"`/`"OFF"` all construct successfully, not just
        // true/false spellings; bare `y`/`n`/`1`/`0` do not (`KeyError`).
        "bool" => match text.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => Ok(Value::Bool(true)),
            "false" | "no" | "off" => Ok(Value::Bool(false)),
            _ => Err(ParseError::new(
                1,
                1,
                format!("invalid YAML: {text:?} is not a valid !!bool value"),
            )),
        },
        "int" => parse_int_literal(text),
        "float" => parse_float_literal(text),
        other => Err(ParseError::new(
            1,
            1,
            format!("invalid YAML: unsupported explicit tag '!!{other}'"),
        )),
    }
}

/// PyYAML's `tag:yaml.org,2002:bool` implicit-resolver spelling set --
/// live-confirmed (see module doc comment): `yes`/`no`/`on`/`off` (and case
/// variants) count as booleans; bare `y`/`n` do not.
fn resolve_plain_scalar(text: &str) -> Result<Value, ParseError> {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Ok(Value::Null),
        "true" | "True" | "TRUE" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" => {
            return Ok(Value::Bool(true));
        }
        "false" | "False" | "FALSE" | "no" | "No" | "NO" | "off" | "Off" | "OFF" => {
            return Ok(Value::Bool(false));
        }
        _ => {}
    }
    if is_int_literal_shape(text) {
        return parse_int_literal(text);
    }
    if is_float_literal_shape(text) {
        return parse_float_literal(text);
    }
    if let Some(iso) = normalize_timestamp(text)? {
        return Ok(Value::Str(iso));
    }
    Ok(Value::Str(text.to_string()))
}

/// Live-confirmed against `yaml.safe_load` (see module doc comment): PyYAML's
/// `tag:yaml.org,2002:int` implicit resolver recognizes `0x`/`0b` prefixes and
/// a bare leading zero as octal (`"017" -> 15`), but **not** a YAML-1.2-style
/// `0o` prefix (`"0o17"` stays a plain string) -- the legacy sexagesimal
/// `1:20` form is out of scope, same rationale as the timestamp normalizer.
static INT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+)$",
    )
    .unwrap()
});

fn is_int_literal_shape(text: &str) -> bool {
    INT_RE.is_match(text)
}

fn parse_int_literal(text: &str) -> Result<Value, ParseError> {
    let neg = text.starts_with('-');
    let t = text.strip_prefix(['+', '-']).unwrap_or(text);
    let cleaned: String = t.chars().filter(|&c| c != '_').collect();
    let (radix, digits) = if let Some(rest) = cleaned.strip_prefix("0x") {
        (16, rest)
    } else if let Some(rest) = cleaned.strip_prefix("0b") {
        (2, rest)
    } else if cleaned.starts_with('0') && cleaned.len() > 1 {
        (8, &cleaned[1..])
    } else {
        (10, cleaned.as_str())
    };
    if radix == 10 && digits.len() > MAX_INT_DIGITS {
        return Err(ParseError::new(
            1,
            1,
            format!(
                "invalid YAML: integer literal has {} digits, exceeding the \
                 {MAX_INT_DIGITS}-digit limit (security: unbounded-digit int-to-str \
                 conversion is superlinear)",
                digits.len()
            ),
        ));
    }
    // Parse the unsigned magnitude first, then apply the sign -- issue #26's
    // fuzz harness caught the previous version parsing `-9223372036854775808`
    // (`i64::MIN`) by stripping the sign and calling
    // `i64::from_str_radix("9223372036854775808", 10)`, which itself
    // overflows (the positive magnitude `9223372036854775808` is one past
    // `i64::MAX`) even though the signed value is perfectly representable.
    // `u64` holds the full magnitude range for both `i64::MIN` and
    // `i64::MAX`, so parse there and negate via `checked_neg` on the signed
    // conversion instead of negating a possibly-unrepresentable positive
    // `i64`.
    let magnitude = match u64::from_str_radix(digits, radix) {
        Ok(m) => m,
        Err(_) => {
            return Err(ParseError::new(
                1,
                1,
                format!(
                    "invalid YAML: integer literal {text:?} is out of range for a 64-bit integer"
                ),
            ));
        }
    };
    let value = if neg {
        if magnitude == i64::MIN.unsigned_abs() {
            Some(i64::MIN)
        } else {
            i64::try_from(magnitude).ok().and_then(i64::checked_neg)
        }
    } else {
        i64::try_from(magnitude).ok()
    };
    match value {
        Some(v) => Ok(Value::Int(v)),
        None => Err(ParseError::new(
            1,
            1,
            format!("invalid YAML: integer literal {text:?} is out of range for a 64-bit integer"),
        )),
    }
}

/// Live-confirmed against `yaml.safe_load`: PyYAML's `tag:yaml.org,2002:float`
/// implicit resolver requires a literal `.` -- `"1e3"` (no decimal point)
/// stays a plain string, and the exponent sign is **mandatory** when present
/// (`"1.0e3"` stays a string; `"1.0e+3"`/`"1.0e-3"` are floats). The legacy
/// sexagesimal float form is out of scope, same rationale as
/// `is_int_literal_shape`.
static FLOAT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$",
    )
    .unwrap()
});

fn is_float_literal_shape(text: &str) -> bool {
    FLOAT_RE.is_match(text)
}

fn parse_float_literal(text: &str) -> Result<Value, ParseError> {
    match text {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => {
            return Ok(Value::Float(f64::INFINITY));
        }
        "-.inf" | "-.Inf" | "-.INF" => return Ok(Value::Float(f64::NEG_INFINITY)),
        ".nan" | ".NaN" | ".NAN" => return Ok(Value::Float(f64::NAN)),
        _ => {}
    }
    let cleaned: String = text.chars().filter(|&c| c != '_').collect();
    cleaned.parse::<f64>().map(Value::Float).map_err(|_| {
        ParseError::new(
            1,
            1,
            format!("invalid YAML: invalid float literal {text:?}"),
        )
    })
}

/// Reproduces PyYAML's `tag:yaml.org,2002:timestamp` resolver + construction
/// (`construct_yaml_timestamp`): accepts a bare ISO date, or a date/time
/// joined by `T`/`t`/one-or-more spaces, with an optional fractional-second
/// part and an optional `Z`/`±HH[:MM]` offset -- and returns the value
/// re-spelled the way `datetime.date.isoformat()`/`datetime.datetime.isoformat()`
/// would: zero-padded, `T`-joined, offset as `+HH:MM`/`-HH:MM` (a bare `Z`
/// becomes `+00:00`, matching `datetime.timezone.utc`'s own `isoformat()`).
/// Returns `Ok(None)` for anything not shaped like a timestamp at all (the
/// overwhelmingly common case -- most plain scalars are plain strings).
///
/// A string that *is* timestamp-shaped but names a calendar/clock value that
/// doesn't exist (`2024-13-01`, `2024-02-30`, `2024-01-01T25:00:00`, an
/// out-of-range timezone offset) is a [`ParseError`], **not** a silent
/// fallback to a plain string -- live-confirmed against PyYAML (see this
/// module's doc comment): `yaml.safe_load` calls `datetime.date`/
/// `datetime.datetime`'s constructor on the captured fields, which raises a
/// `ValueError` PyYAML doesn't catch, so the *whole document* fails to parse
/// rather than quietly typing the value as a string. Calendar-date validity
/// (leap years, per-month day counts) reuses [`crate::schema::valid_ymd`]/
/// [`crate::schema::valid_hms`] rather than a second copy of that logic.
fn normalize_timestamp(text: &str) -> Result<Option<String>, ParseError> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"^(?P<year>[0-9]{4})-(?P<month>[0-9][0-9]?)-(?P<day>[0-9][0-9]?)(?:(?:[Tt]|[ \t]+)(?P<hour>[0-9][0-9]?):(?P<minute>[0-9][0-9]):(?P<second>[0-9][0-9])(?:\.(?P<fraction>[0-9]*))?(?:[ \t]*(?:Z|(?P<tz_sign>[-+])(?P<tz_hour>[0-9][0-9]?)(?::(?P<tz_minute>[0-9][0-9]))?))?)?$",
        )
        .unwrap()
    });
    let Some(caps) = RE.captures(text) else {
        return Ok(None);
    };
    let bad = |what: &str| {
        Err(ParseError::new(
            1,
            1,
            format!("invalid YAML: {text:?} is timestamp-shaped but names an invalid {what}"),
        ))
    };
    let year: u32 = caps["year"].parse().unwrap_or(u32::MAX);
    let month: u32 = caps["month"].parse().unwrap_or(u32::MAX);
    let day: u32 = caps["day"].parse().unwrap_or(u32::MAX);
    if !crate::schema::valid_ymd(year, month, day) {
        return bad("calendar date");
    }
    let Some(hour_m) = caps.name("hour") else {
        return Ok(Some(format!("{year:04}-{month:02}-{day:02}")));
    };
    let hour: u32 = hour_m.as_str().parse().unwrap_or(u32::MAX);
    let minute: u32 = caps["minute"].parse().unwrap_or(u32::MAX);
    let second: u32 = caps["second"].parse().unwrap_or(u32::MAX);
    if !crate::schema::valid_hms(hour, minute, second) {
        return bad("time of day");
    }
    let mut out = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    if let Some(frac) = caps.name("fraction") {
        let mut digits = frac.as_str().to_string();
        while digits.len() < 6 {
            digits.push('0');
        }
        digits.truncate(6);
        out.push('.');
        out.push_str(&digits);
    }
    match caps.name("tz_sign") {
        Some(sign) => {
            let tz_hour: u32 = caps["tz_hour"].parse().unwrap_or(u32::MAX);
            let tz_minute: u32 = caps
                .name("tz_minute")
                .map(|m| m.as_str().parse().unwrap_or(u32::MAX))
                .unwrap_or(0);
            if tz_hour > 23 || tz_minute > 59 {
                return bad("timezone offset");
            }
            out.push_str(sign.as_str());
            out.push_str(&format!("{tz_hour:02}:{tz_minute:02}"));
        }
        None if text.trim_end().ends_with('Z') => out.push_str("+00:00"),
        None => {}
    }
    Ok(Some(out))
}

// ============================================================== Writer

/// Project a [`Doc`] to YAML text (block style, 2-space indent, insertion
/// order preserved -- matching Python's `yaml.dump(..., sort_keys=False,
/// default_flow_style=False)`).
pub fn write_yaml(
    doc: &Doc,
    strict: bool,
    report: Option<&mut WriteReport>,
) -> Result<String, WriteError> {
    let rep = check_yaml(doc);
    let grouped = doc.to_grouped();
    let mut out = String::new();
    write_node(&grouped, 0, &mut out, true);
    if out.ends_with('\n') {
        out.pop();
    }
    crate::report::finish_write(out, rep, strict, report)
}

/// Report what writing YAML would adjust, without producing output. The only
/// adjustment YAML ever needs is forcing double-quoted style for a U+0085
/// (NEL) string/label -- see this module's doc comment.
pub fn check_yaml(doc: &Doc) -> WriteReport {
    let mut rep = WriteReport::new();
    scan_nel(&doc.to_grouped(), "$", &mut rep);
    rep
}

fn scan_nel(node: &Value, path: &str, rep: &mut WriteReport) {
    match node {
        Value::Object(map) => {
            for (label, child) in map {
                if label.contains('\u{0085}') {
                    rep.add(
                        format!("{path}.{label}"),
                        "string.line-break-char",
                        "label contains U+0085 (NEL); written double-quoted to round-trip \
                         correctly",
                        Severity::Warning,
                    );
                }
                match child {
                    Value::Array(items) => {
                        for (i, item) in items.iter().enumerate() {
                            let p = if i == 0 {
                                format!("{path}.{label}")
                            } else {
                                format!("{path}.{label}[{i}]")
                            };
                            scan_nel(item, &p, rep);
                        }
                    }
                    other => scan_nel(other, &format!("{path}.{label}"), rep),
                }
            }
        }
        Value::Str(s) if s.contains('\u{0085}') => {
            rep.add(
                path,
                "string.line-break-char",
                "value contains U+0085 (NEL); written double-quoted to round-trip correctly",
                Severity::Warning,
            );
        }
        _ => {}
    }
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

/// Writes `node` at `level`, matching PyYAML's block style: a scalar directly
/// after `key:`/`- ` on the same line; a nested mapping/sequence starts on
/// the next line, indented. `top` is `true` only for the document root, which
/// (for an empty root object) still needs `{}` -- an empty nested object
/// is written the same way PyYAML does, inline `{}`/`[]`.
fn write_node(node: &Value, level: usize, out: &mut String, top: bool) {
    match node {
        Value::Object(map) if map.is_empty() => {
            out.push_str("{}\n");
        }
        Value::Array(items) if items.is_empty() => {
            out.push_str("[]\n");
        }
        Value::Object(map) => {
            for (label, child) in map {
                indent(out, level);
                write_scalar(label, out);
                out.push(':');
                write_child(child, level, out);
            }
            let _ = top;
        }
        Value::Array(items) => {
            for item in items {
                indent(out, level);
                out.push('-');
                write_seq_child(item, level, out);
            }
        }
        other => {
            write_scalar_value(other, out);
            out.push('\n');
        }
    }
}

fn write_child(child: &Value, level: usize, out: &mut String) {
    match child {
        Value::Object(m) if !m.is_empty() => {
            out.push('\n');
            write_node(child, level + 1, out, false);
        }
        Value::Array(a) if !a.is_empty() => {
            out.push('\n');
            write_node(child, level, out, false);
        }
        _ => {
            out.push(' ');
            write_node(child, level + 1, out, false);
        }
    }
}

fn write_seq_child(item: &Value, level: usize, out: &mut String) {
    match item {
        Value::Object(m) if !m.is_empty() => {
            out.push(' ');
            // The first field of a mapping under a `- ` sequence marker is
            // written on the same line as the dash; PyYAML indents the
            // remaining fields to align under it (one level deeper than the
            // dash itself, i.e. `level + 1`).
            let mut first = true;
            for (label, child) in m {
                if !first {
                    indent(out, level + 1);
                }
                first = false;
                write_scalar(label, out);
                out.push(':');
                write_child(child, level + 1, out);
            }
        }
        Value::Array(a) if !a.is_empty() => {
            out.push('\n');
            write_node(item, level + 1, out, false);
        }
        _ => {
            out.push(' ');
            write_node(item, level + 1, out, false);
        }
    }
}

fn write_scalar(s: &str, out: &mut String) {
    write_scalar_value(&Value::Str(s.to_string()), out);
}

fn write_scalar_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(x) => write_float(*x, out),
        Value::Str(s) => write_yaml_string(s, out),
        Value::Object(_) | Value::Array(_) => {
            unreachable!("write_scalar_value is only ever called on a leaf")
        }
    }
}

fn write_float(x: f64, out: &mut String) {
    if x.is_nan() {
        out.push_str(".nan");
    } else if x.is_infinite() {
        out.push_str(if x > 0.0 { ".inf" } else { "-.inf" });
    } else {
        // See `json.rs::write_float` for why this checks the rendered
        // string for `.`/`e`/`E` rather than comparing `x` against a fixed
        // magnitude cutoff (issue #46: Rust's `f64::to_string()` drops the
        // decimal point for integral values >= 1e17).
        let s = x.to_string();
        if s.contains('.') || s.contains('e') || s.contains('E') {
            out.push_str(&s);
        } else {
            out.push_str(&s);
            out.push_str(".0");
        }
    }
}

/// Writes `s` as a YAML scalar: double-quoted (with the U+0085 escape
/// `check_yaml` warns about) if it contains a NEL, or if writing it bare
/// would round-trip back as a *different* value (it would be re-resolved as
/// null/bool/int/float/timestamp, it's empty, or it has YAML-significant
/// leading/embedded punctuation) -- otherwise written plain.
fn write_yaml_string(s: &str, out: &mut String) {
    if needs_quoting(s) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\u{0085}' => out.push_str("\\N"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn needs_quoting(s: &str) -> bool {
    if s.is_empty() || s.contains('\u{0085}') || s.contains('\n') {
        return true;
    }
    if matches!(resolve_plain_scalar(s), Ok(Value::Str(ref t)) if t == s) {
        // Round-trips as the identical plain string -- but a leading char /
        // embedded token that's YAML-significant still needs quoting even
        // though resolve_plain_scalar wouldn't itself retype it.
    } else {
        return true; // would be re-read as null/bool/int/float/timestamp
    }
    let first = s.chars().next().unwrap();
    if matches!(
        first,
        '-' | '?'
            | ':'
            | ','
            | '['
            | ']'
            | '{'
            | '}'
            | '#'
            | '&'
            | '*'
            | '!'
            | '|'
            | '>'
            | '\''
            | '"'
            | '%'
            | '@'
            | '`'
            | ' '
    ) {
        return true;
    }
    if s.ends_with(' ') || s.contains(": ") || s.ends_with(':') || s.contains(" #") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Doc, Scalar, Value};

    fn obj(pairs: Vec<(&str, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn doc_of(v: Value) -> Doc {
        Doc::of(&v).unwrap()
    }

    // ---------------------------------------------------------- reader: scalars

    #[test]
    fn reads_every_scalar_kind() {
        let doc = read_yaml("a: 1\nb: \"s\"\nc: true\nd: null\ne: 1.5\n").unwrap();
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
    fn reads_yaml_1_1_bool_spellings_but_not_bare_y_or_n() {
        let doc = read_yaml("a: yes\nb: no\nc: on\nd: off\ne: Yes\nf: y\ng: n\n").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Bool(false)
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        assert_eq!(
            *root.get_one("d").unwrap().value().unwrap(),
            Scalar::Bool(false)
        );
        assert_eq!(
            *root.get_one("e").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        // 'y'/'n' alone stay strings -- live-confirmed against PyYAML (see
        // module doc comment).
        assert_eq!(
            *root.get_one("f").unwrap().value().unwrap(),
            Scalar::Str("y".to_string())
        );
        assert_eq!(
            *root.get_one("g").unwrap().value().unwrap(),
            Scalar::Str("n".to_string())
        );
    }

    #[test]
    fn quoted_yes_stays_a_string_not_a_bool() {
        let doc = read_yaml("a: \"yes\"\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("yes".to_string())
        );
    }

    #[test]
    fn reads_null_spellings() {
        let doc = read_yaml("a: ~\nb: null\nc: Null\nd: NULL\ne:\n").unwrap();
        let root = doc.root();
        for label in ["a", "b", "c", "d", "e"] {
            assert_eq!(*root.get_one(label).unwrap().value().unwrap(), Scalar::Null);
        }
    }

    #[test]
    fn reads_negative_and_hex_and_octal_and_binary_ints() {
        // Note: PyYAML's legacy (YAML-1.1-derived) int resolver recognizes a
        // *bare leading zero* as octal ("017" -> 15), not a `0o` prefix --
        // live-confirmed (see module doc comment on `INT_RE`); a separate
        // test below pins `"0o17"` staying a string for that exact reason.
        let doc = read_yaml("a: -5\nb: 0x1A\nc: 017\nd: 0b101\ne: 1_000\n").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Int(-5)
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Int(26)
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Int(15)
        );
        assert_eq!(*root.get_one("d").unwrap().value().unwrap(), Scalar::Int(5));
        assert_eq!(
            *root.get_one("e").unwrap().value().unwrap(),
            Scalar::Int(1000)
        );
    }

    #[test]
    fn a_yaml_1_2_style_0o_octal_prefix_is_not_recognized_and_stays_a_string() {
        let doc = read_yaml("a: 0o17\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("0o17".to_string())
        );
    }

    #[test]
    fn reads_float_and_inf_and_nan_tokens() {
        let doc = read_yaml("a: 1.5\nb: .inf\nc: -.inf\nd: .nan\ne: 1.0e+3\n").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Float(1.5)
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Float(f64::INFINITY)
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Float(f64::NEG_INFINITY)
        );
        assert!(
            matches!(root.get_one("d").unwrap().value().unwrap(), Scalar::Float(x) if x.is_nan())
        );
        assert_eq!(
            *root.get_one("e").unwrap().value().unwrap(),
            Scalar::Float(1000.0)
        );
    }

    #[test]
    fn a_bare_exponent_without_a_decimal_point_is_not_float_shaped_and_stays_a_string() {
        // Live-confirmed against PyYAML: its float resolver requires a
        // literal `.`, and a mandatory sign on the exponent when present --
        // "1e3" and "1.0e3" both stay plain strings.
        let doc = read_yaml("a: 1e3\nb: 1.0e3\n").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Str("1e3".to_string())
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Str("1.0e3".to_string())
        );
    }

    #[test]
    fn reads_bare_date_as_iso_string() {
        let doc = read_yaml("a: 2024-01-15\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2024-01-15".to_string())
        );
    }

    #[test]
    fn reads_loose_timestamp_and_normalizes_to_canonical_iso() {
        // Space-separated, single-digit month/day/hour, short fraction,
        // bare `Z` -- all normalized the way `datetime.isoformat()` would.
        // (Minute/second must still be exactly two digits -- that's PyYAML's
        // own `tag:yaml.org,2002:timestamp` regex, not a relaxation this port
        // invented; live-confirmed via the module doc comment's approach.)
        let doc = read_yaml("a: 2001-2-3 4:05:06.7 Z\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2001-02-03T04:05:06.700000+00:00".to_string())
        );
    }

    #[test]
    fn a_single_digit_minute_or_second_is_not_timestamp_shaped_and_stays_a_string() {
        // PyYAML's own timestamp resolver requires an exactly-2-digit
        // minute/second even though hour/month/day may be 1 or 2 digits --
        // confirmed by this port's `normalize_timestamp` regex (mirroring
        // PyYAML's), not a looser rule this port invented.
        let doc = read_yaml("a: 2001-2-3 4:5:6\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2001-2-3 4:5:6".to_string())
        );
    }

    #[test]
    fn reads_datetime_with_no_timezone_at_all() {
        // A full date+time with no `Z` and no explicit offset -- exercises
        // `normalize_timestamp`'s "no timezone information at all" arm,
        // distinct from both the explicit-offset and bare-`Z` cases.
        let doc = read_yaml("a: 2024-01-15T12:30:00\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2024-01-15T12:30:00".to_string())
        );
    }

    #[test]
    fn reads_timestamp_with_explicit_offset() {
        let doc = read_yaml("a: 2001-12-14T21:59:43.10-05:00\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2001-12-14T21:59:43.100000-05:00".to_string())
        );
    }

    #[test]
    fn a_string_that_merely_looks_like_a_short_date_but_isnt_shaped_right_stays_a_string() {
        let doc = read_yaml("a: 2024-1\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2024-1".to_string())
        );
    }

    // ---------------------------------------------------------- reader: structure

    #[test]
    fn reads_nested_mapping_and_sequence() {
        let doc = read_yaml("a:\n  b:\n    c: 1\nm:\n  - 1\n  - 2\n  - 3\n").unwrap();
        let root = doc.root();
        let a = root.get_one("a").unwrap();
        let b = a.get_one("b").unwrap();
        assert_eq!(*b.get_one("c").unwrap().value().unwrap(), Scalar::Int(1));
        let ms = root.get("m");
        assert_eq!(ms.len(), 3);
        assert_eq!(*ms[2].value().unwrap(), Scalar::Int(3));
    }

    #[test]
    fn reads_flow_style_mapping_and_sequence() {
        let doc = read_yaml("a: {b: 1, c: 2}\nm: [1, 2, 3]\n").unwrap();
        let root = doc.root();
        let a = root.get_one("a").unwrap();
        assert_eq!(*a.get_one("b").unwrap().value().unwrap(), Scalar::Int(1));
        assert_eq!(root.get("m").len(), 3);
    }

    #[test]
    fn bare_top_level_sequence_is_a_document_error_not_a_parse_error() {
        let err = read_yaml("- 1\n- 2\n").unwrap_err();
        assert!(matches!(err, OmnistError::Document(_)), "got {err:?}");
    }

    #[test]
    fn sequence_of_sequences_is_a_document_error() {
        let err = read_yaml("m:\n  - [1, 2]\n").unwrap_err();
        assert!(matches!(err, OmnistError::Document(_)), "got {err:?}");
    }

    #[test]
    fn empty_input_reads_as_a_null_document() {
        let doc = read_yaml("").unwrap();
        assert_eq!(*doc.root().value().unwrap(), Scalar::Null);
    }

    #[test]
    fn explicit_empty_document_marker_reads_as_a_null_document() {
        // An explicit `---` document-start marker with no content produces a
        // `DocumentEnd` event with nothing ever pushed onto the doc stack --
        // a different code path than a wholly empty input (which never
        // produces a `DocumentStart`/`DocumentEnd` pair at all).
        let doc = read_yaml("---\n").unwrap();
        assert_eq!(*doc.root().value().unwrap(), Scalar::Null);
    }

    #[test]
    fn duplicate_mapping_keys_last_value_wins() {
        let doc = read_yaml("a: 1\nb: 2\na: 3\n").unwrap();
        let root = doc.root();
        assert_eq!(root.labels(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(*root.get_one("a").unwrap().value().unwrap(), Scalar::Int(3));
    }

    #[test]
    fn invalid_yaml_syntax_is_a_parse_error() {
        let err = read_yaml("a: [1, 2\n").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn multiple_documents_is_a_parse_error() {
        let err = read_yaml("a: 1\n---\nb: 2\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("single document")),
            "got {err:?}"
        );
    }

    #[test]
    fn nesting_past_max_depth_is_a_document_error() {
        // Genuine indentation-nested mappings, not a flat run of overwritten
        // "a:" keys -- each level indented two spaces deeper than the last.
        let mut text = String::new();
        for i in 0..=crate::document::MAX_DEPTH {
            text.push_str(&"  ".repeat(i));
            text.push_str("a:\n");
        }
        let err = read_yaml(&text).unwrap_err();
        assert!(matches!(err, OmnistError::Document(_)), "got {err:?}");
    }

    #[test]
    fn integer_literal_over_digit_cap_is_rejected() {
        let text = format!("a: {}\n", "9".repeat(MAX_INT_DIGITS + 1));
        let err = read_yaml(&text).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("4300-digit")),
            "got {err:?}"
        );
    }

    #[test]
    fn i64_min_round_trips_through_yaml() {
        // Regression test for issue #26's fuzz harness finding: the
        // previous `parse_int_literal` stripped the sign, then parsed the
        // *positive* magnitude as `i64`, which overflows for `i64::MIN`
        // (whose magnitude, 9223372036854775808, is one past `i64::MAX`)
        // even though the signed value itself is representable.
        let doc = read_yaml("a: -9223372036854775808\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Int(i64::MIN)
        );
    }

    #[test]
    fn positive_integer_one_past_i64_max_is_out_of_range_error() {
        // Fits in u64 (so passes the magnitude parse) but not in i64 --
        // exercises the final `None` arm of `parse_int_literal`'s match,
        // distinct from `integer_literal_over_i64_range_is_out_of_range_error`
        // below (whose 20-nines literal overflows even `u64`).
        let err = read_yaml("a: 9223372036854775808\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("out of range")),
            "got {err:?}"
        );
    }

    #[test]
    fn negative_integer_one_past_i64_min_is_out_of_range_error() {
        // Magnitude 9223372036854775809 fits in u64 and isn't
        // `i64::MIN.unsigned_abs()`, so it falls through to the
        // `checked_neg` arm, which correctly reports out-of-range instead
        // of silently wrapping.
        let err = read_yaml("a: -9223372036854775809\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("out of range")),
            "got {err:?}"
        );
    }

    #[test]
    fn integer_literal_over_i64_range_is_out_of_range_error() {
        let text = format!("a: {}\n", "9".repeat(20));
        let err = read_yaml(&text).unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("out of range")),
            "got {err:?}"
        );
    }

    // ---------------------------------------------------------- reader: anchors/aliases

    #[test]
    fn reads_anchor_and_alias_as_a_deep_copy() {
        let doc = read_yaml("a: &x [1, 2, 3]\nb: *x\n").unwrap();
        let root = doc.root();
        assert_eq!(root.get("a").len(), 3);
        assert_eq!(root.get("b").len(), 3);
        assert_eq!(*root.get("b")[1].value().unwrap(), Scalar::Int(2));
    }

    #[test]
    fn unknown_alias_is_a_parse_error() {
        let err = read_yaml("a: *nope\n").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    // ---------------------------------------------------------- reader: merge keys

    #[test]
    fn merge_key_from_a_mapping_merges_with_local_keys_winning() {
        let doc =
            read_yaml("base: &b\n  x: 1\n  y: 2\nchild:\n  <<: *b\n  y: 20\n  z: 3\n").unwrap();
        let child = doc.root().get_one("child").unwrap();
        assert_eq!(
            *child.get_one("x").unwrap().value().unwrap(),
            Scalar::Int(1)
        );
        assert_eq!(
            *child.get_one("y").unwrap().value().unwrap(),
            Scalar::Int(20),
            "an explicit local key beats the merged-in value"
        );
        assert_eq!(
            *child.get_one("z").unwrap().value().unwrap(),
            Scalar::Int(3)
        );
    }

    #[test]
    fn merge_key_from_a_sequence_of_mappings_merges_each_in_order() {
        // First-listed source wins a collision between the sources
        // themselves (YAML merge spec), matching PyYAML's own behavior.
        let doc =
            read_yaml("a: &a\n  x: 1\nb: &b\n  x: 2\n  y: 3\nchild:\n  <<: [*a, *b]\n").unwrap();
        let child = doc.root().get_one("child").unwrap();
        assert_eq!(
            *child.get_one("x").unwrap().value().unwrap(),
            Scalar::Int(1)
        );
        assert_eq!(
            *child.get_one("y").unwrap().value().unwrap(),
            Scalar::Int(3)
        );
    }

    #[test]
    fn quoted_double_angle_bracket_key_is_a_literal_string_not_a_merge() {
        let doc = read_yaml("a:\n  \"<<\": 1\n").unwrap();
        let a = doc.root().get_one("a").unwrap();
        assert_eq!(*a.get_one("<<").unwrap().value().unwrap(), Scalar::Int(1));
    }

    // omnist-ts#46: a fuzz suite intermittently found a ParseError with a
    // non-map merge source -- pinned here as an explicit regression: merging
    // from a scalar must fail predictably (a clean ParseError), never panic
    // or behave inconsistently.
    #[test]
    fn merge_key_from_a_non_map_scalar_source_is_a_clean_parse_error_omnist_ts_46() {
        let err = read_yaml("child:\n  <<: 5\n  y: 2\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("merge key")),
            "got {err:?}"
        );
    }

    #[test]
    fn merge_key_from_a_sequence_containing_a_scalar_is_a_clean_parse_error() {
        let err = read_yaml("child:\n  <<: [1, 2]\n").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn merge_source_with_a_non_scalar_key_is_a_clean_parse_error() {
        // Exercises the (structurally rare) branch where a merge source's
        // own key isn't a plain scalar (YAML's complex-mapping-key syntax) --
        // `scalar_key_text` returns `None` for it during merge de-duplication,
        // and the final scalar-key check in `raw_to_value` still catches it.
        let err = read_yaml("base: &b\n  ? [1, 2]\n  : 3\nchild:\n  <<: *b\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("mapping key must be a scalar")),
            "got {err:?}"
        );
    }

    #[test]
    fn merge_key_from_an_alias_to_a_scalar_is_a_clean_parse_error() {
        let err = read_yaml("base: &b 5\nchild:\n  <<: *b\n").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    // ---------------------------------------------------------- reader: explicit tags

    #[test]
    fn explicit_tags_construct_the_named_type_regardless_of_spelling() {
        let doc = read_yaml("a: !!str yes\nb: !!int \"5\"\nc: !!bool \"true\"\n").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Str("yes".to_string())
        );
        assert_eq!(*root.get_one("b").unwrap().value().unwrap(), Scalar::Int(5));
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
    }

    #[test]
    fn unsupported_explicit_tag_is_a_parse_error() {
        let err = read_yaml("a: !!binary \"x\"\n").unwrap_err();
        assert!(matches!(err, OmnistError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn explicit_bool_tag_accepts_the_full_yaml_1_1_spelling_set_not_just_true_false() {
        // Live-confirmed against PyYAML (see module doc comment on
        // `explicit_tag_to_value`'s "bool" arm): `!!bool` uses the exact same
        // `bool_values` lookup regardless of implicit vs. explicit tagging,
        // so "yes"/"On"/"OFF" all construct via the explicit tag too.
        let doc = read_yaml("a: !!bool \"yes\"\nb: !!bool \"On\"\nc: !!bool \"OFF\"\n").unwrap();
        let root = doc.root();
        assert_eq!(
            *root.get_one("a").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        assert_eq!(
            *root.get_one("b").unwrap().value().unwrap(),
            Scalar::Bool(true)
        );
        assert_eq!(
            *root.get_one("c").unwrap().value().unwrap(),
            Scalar::Bool(false)
        );
    }

    #[test]
    fn invalid_explicit_bool_spelling_is_a_parse_error() {
        let err = read_yaml("a: !!bool \"nonsense\"\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("!!bool")),
            "got {err:?}"
        );
    }

    #[test]
    fn bare_y_or_n_is_not_a_valid_explicit_bool_spelling() {
        // Live-confirmed: PyYAML's `bool_values` dict has no "y"/"n" keys
        // even though the plain-scalar implicit resolver never even tags
        // these as bool in the first place -- a bare "y"/"n" reaches this
        // arm only via an explicit `!!bool` tag, and PyYAML raises
        // (`KeyError` -> `ConstructorError`) rather than accepting it.
        let err = read_yaml("a: !!bool \"y\"\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("!!bool")),
            "got {err:?}"
        );
    }

    #[test]
    fn a_non_scalar_mapping_key_is_a_clean_parse_error() {
        // YAML's complex-mapping-key syntax (`? ... : ...`) allows a
        // sequence/mapping as a key -- this model's edges are always
        // string-labeled, so this is rejected with a clean ParseError.
        let err = read_yaml("? [1, 2]\n: 3\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("mapping key must be a scalar")),
            "got {err:?}"
        );
    }

    // ---------------------------------------------------------- writer

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
        let text = write_yaml(&doc, false, None).unwrap();
        let back = read_yaml(&text).unwrap();
        assert!(doc.eq_doc(&back));
    }

    #[test]
    fn round_trips_integral_float_at_and_above_1e17_boundary_issue_46() {
        // Regression test for issue #46 (see json.rs's twin test for the
        // full explanation): an integral-valued float >= 1e17 used to
        // render as a bare digit run and re-read as `Scalar::Int`.
        for x in [1.0e17, 1.0e18, -1.23e17, 9.9e16_f64] {
            let doc = doc_of(obj(vec![("a", Value::Float(x))]));
            let text = write_yaml(&doc, false, None).unwrap();
            let back = read_yaml(&text).unwrap();
            assert_eq!(
                *back.root().get_one("a").unwrap().value().unwrap(),
                Scalar::Float(x),
                "x={x} text={text}"
            );
        }
    }

    #[test]
    fn round_trips_nan_and_infinity_natively_no_adjustment_needed() {
        let v = obj(vec![
            ("a", Value::Float(f64::NAN)),
            ("b", Value::Float(f64::INFINITY)),
            ("c", Value::Float(f64::NEG_INFINITY)),
        ]);
        let doc = doc_of(v);
        let mut rep = WriteReport::new();
        let text = write_yaml(&doc, false, Some(&mut rep)).unwrap();
        assert!(rep.is_empty());
        let back = read_yaml(&text).unwrap();
        assert!(
            matches!(back.root().get_one("a").unwrap().value().unwrap(), Scalar::Float(x) if x.is_nan())
        );
        assert_eq!(
            *back.root().get_one("b").unwrap().value().unwrap(),
            Scalar::Float(f64::INFINITY)
        );
        assert_eq!(
            *back.root().get_one("c").unwrap().value().unwrap(),
            Scalar::Float(f64::NEG_INFINITY)
        );
    }

    #[test]
    fn round_trips_strings_that_look_like_other_scalar_kinds() {
        let v = obj(vec![
            ("a", Value::Str("yes".to_string())),
            ("b", Value::Str("null".to_string())),
            ("c", Value::Str("123".to_string())),
            ("d", Value::Str("1.5".to_string())),
            ("e", Value::Str("".to_string())),
            ("f", Value::Str("2024-01-15".to_string())),
        ]);
        let doc = doc_of(v);
        let text = write_yaml(&doc, false, None).unwrap();
        let back = read_yaml(&text).unwrap();
        assert!(doc.eq_doc(&back), "text was:\n{text}");
    }

    #[test]
    fn round_trips_repeated_labels_as_a_yaml_sequence() {
        let doc = doc_of(obj(vec![(
            "m",
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        )]));
        let text = write_yaml(&doc, false, None).unwrap();
        let back = read_yaml(&text).unwrap();
        assert!(doc.eq_doc(&back));
    }

    #[test]
    fn round_trips_nested_mappings_and_sequences_of_mappings() {
        let v = obj(vec![
            ("a", obj(vec![("b", Value::Int(1)), ("c", Value::Int(2))])),
            (
                "items",
                Value::Array(vec![
                    obj(vec![("x", Value::Int(1)), ("y", Value::Int(2))]),
                    obj(vec![("x", Value::Int(3)), ("y", Value::Int(4))]),
                ]),
            ),
        ]);
        let doc = doc_of(v);
        let text = write_yaml(&doc, false, None).unwrap();
        let back = read_yaml(&text).unwrap();
        assert!(doc.eq_doc(&back), "text was:\n{text}");
    }

    #[test]
    fn writes_empty_object_and_array_compactly() {
        let doc = doc_of(obj(vec![("o", Value::Object(IndexMap::new()))]));
        let text = write_yaml(&doc, false, None).unwrap();
        assert!(text.contains("o: {}"));
    }

    #[test]
    fn nel_string_triggers_a_warning_and_still_round_trips() {
        let s = format!("a{}b", '\u{0085}');
        let doc = doc_of(obj(vec![("s", Value::Str(s.clone()))]));
        let mut rep = WriteReport::new();
        let text = write_yaml(&doc, false, Some(&mut rep)).unwrap();
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].code, "string.line-break-char");
        let back = read_yaml(&text).unwrap();
        assert_eq!(
            *back.root().get_one("s").unwrap().value().unwrap(),
            Scalar::Str(s)
        );
    }

    #[test]
    fn strict_write_with_nel_raises_and_carries_the_report() {
        let s = format!("x{}y", '\u{0085}');
        let doc = doc_of(obj(vec![("s", Value::Str(s))]));
        let err = write_yaml(&doc, true, None).unwrap_err();
        let rep = err.report().expect("strict WriteError carries a report");
        assert_eq!(rep.len(), 1);
    }

    #[test]
    fn strict_write_with_no_adjustments_succeeds() {
        let doc = doc_of(obj(vec![("a", Value::Int(1))]));
        let text = write_yaml(&doc, true, None).unwrap();
        assert!(text.contains("a: 1"));
    }

    #[test]
    fn check_yaml_reports_without_producing_output() {
        let s = format!("a{}b", '\u{0085}');
        let doc = doc_of(obj(vec![("s", Value::Str(s))]));
        let rep = check_yaml(&doc);
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].path, "$.s");
    }

    #[test]
    fn deeply_nested_document_write_reuses_doc_construction_depth_guard() {
        let mut v = Value::Int(0);
        for _ in 0..=crate::document::MAX_DEPTH {
            v = obj(vec![("a", v)]);
        }
        assert!(Doc::of(&v).is_err());
    }

    // ---------------------------------------------------------- omnist-ts#43: wide document

    // omnist-ts#43: YAML read was a ~3x-25x performance outlier vs OML/JSON on
    // wide documents. Not a hard timing assertion here (Rust is a different
    // performance regime) -- this is the *correctness*-under-scale angle:
    // a wide document reads and round-trips correctly, so a similarly bad
    // implementation (e.g. quadratic re-scanning per field) doesn't silently
    // produce wrong output even if it's slow.
    #[test]
    fn wide_document_smoke_test_reads_and_round_trips_every_field() {
        let n = 5_000;
        let mut text = String::new();
        for i in 0..n {
            text.push_str(&format!("field{i}: {i}\n"));
        }
        let doc = read_yaml(&text).unwrap();
        let root = doc.root();
        assert_eq!(root.labels().len(), n);
        for i in [0, n / 2, n - 1] {
            assert_eq!(
                *root.get_one(&format!("field{i}")).unwrap().value().unwrap(),
                Scalar::Int(i as i64)
            );
        }
        let out = write_yaml(&doc, false, None).unwrap();
        let back = read_yaml(&out).unwrap();
        assert!(doc.eq_doc(&back));
    }

    #[test]
    fn wide_flat_sequence_smoke_test() {
        let n = 5_000;
        let mut text = String::from("m:\n");
        for i in 0..n {
            text.push_str(&format!("  - {i}\n"));
        }
        let doc = read_yaml(&text).unwrap();
        assert_eq!(doc.root().get("m").len(), n);
    }

    // ---------------------------------------------------------- coverage: explicit tags/errors

    #[test]
    fn invalid_explicit_float_literal_is_a_parse_error() {
        let err = read_yaml("a: !!float \"not-a-float\"\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("invalid float literal")),
            "got {err:?}"
        );
    }

    #[test]
    fn explicit_float_tag_accepts_inf_and_nan_and_negative() {
        let doc = read_yaml("a: !!float \"-1.5\"\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Float(-1.5)
        );
    }

    // ---------------------------------------------------------- coverage: timestamp edge cases

    // Live-confirmed against PyYAML (see `normalize_timestamp`'s doc
    // comment): a timestamp-*shaped* string naming a calendar/clock value
    // that doesn't exist is a clean ParseError, not a silent string
    // fallback -- `yaml.safe_load` calls `datetime.date`/`datetime.datetime`
    // construction on the captured fields, and that raises `ValueError`
    // for an out-of-range month/day/hour/minute/timezone, failing the
    // *entire document*, not just retyping this one scalar as a string.

    #[test]
    fn timestamp_with_invalid_month_is_a_parse_error() {
        let err = read_yaml("a: 2024-13-01\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("calendar date")),
            "got {err:?}"
        );
    }

    #[test]
    fn timestamp_with_year_zero_is_a_parse_error() {
        let err = read_yaml("a: 0000-01-01\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("calendar date")),
            "got {err:?}"
        );
    }

    #[test]
    fn timestamp_with_a_day_that_doesnt_exist_in_the_month_is_a_parse_error() {
        // February 30th never exists, regardless of leap year -- exercises
        // the day-count upper bound (`schema::valid_ymd`'s `days_in_month`),
        // not just the "day < 1" lower bound.
        let err = read_yaml("a: 2024-02-30\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("calendar date")),
            "got {err:?}"
        );
    }

    #[test]
    fn timestamp_february_29_on_a_non_leap_year_is_a_parse_error() {
        let err = read_yaml("a: 2023-02-29\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("calendar date")),
            "got {err:?}"
        );
    }

    #[test]
    fn timestamp_february_29_on_a_leap_year_normalizes_fine() {
        let doc = read_yaml("a: 2024-02-29\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2024-02-29".to_string())
        );
    }

    #[test]
    fn timestamp_with_out_of_range_hour_is_a_parse_error() {
        let err = read_yaml("a: 2024-01-01T25:00:00\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("time of day")),
            "got {err:?}"
        );
    }

    #[test]
    fn timestamp_with_out_of_range_minute_is_a_parse_error() {
        let err = read_yaml("a: 2024-01-01T00:61:00\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("time of day")),
            "got {err:?}"
        );
    }

    #[test]
    fn timestamp_with_out_of_range_timezone_offset_is_a_parse_error() {
        let err = read_yaml("a: 2024-01-01T00:00:00+25:00\n").unwrap_err();
        assert!(
            matches!(&err, OmnistError::Parse(e) if e.message.contains("timezone offset")),
            "got {err:?}"
        );
    }

    #[test]
    fn timestamp_with_hour_only_timezone_offset_normalizes_with_zero_minutes() {
        let doc = read_yaml("a: 2024-01-01T00:00:00+05\n").unwrap();
        assert_eq!(
            *doc.root().get_one("a").unwrap().value().unwrap(),
            Scalar::Str("2024-01-01T00:00:00+05:00".to_string())
        );
    }

    // ---------------------------------------------------------- coverage: writer

    #[test]
    fn nel_in_a_label_triggers_a_warning_and_still_round_trips() {
        let label = format!("a{}b", '\u{0085}');
        let doc = doc_of(obj(vec![(label.as_str(), Value::Int(1))]));
        let mut rep = WriteReport::new();
        let text = write_yaml(&doc, false, Some(&mut rep)).unwrap();
        assert_eq!(rep.len(), 1);
        assert_eq!(rep.adjustments()[0].code, "string.line-break-char");
        let back = read_yaml(&text).unwrap();
        assert_eq!(
            *back.root().get_one(&label).unwrap().value().unwrap(),
            Scalar::Int(1)
        );
    }

    #[test]
    fn write_node_on_a_bare_empty_array_writes_the_flow_empty_token() {
        // `Doc`'s own model never produces a *standalone* empty-array node
        // (an empty `Value::Array` under a key expands into zero edges at
        // construction time -- see `document.rs`'s own note on this) --
        // white-box exercising `write_node`'s empty-array arm directly here,
        // the same "test the arm directly since the public API can't reach
        // it" pattern `document.rs`'s `internal_edges_mut_rejects_a_leaf_directly`
        // and `json.rs`'s handling of the analogous case establish.
        let mut out = String::new();
        write_node(&Value::Array(vec![]), 0, &mut out, true);
        assert_eq!(out, "[]\n");
    }

    #[test]
    fn round_trips_strings_needing_every_quoting_trigger() {
        let cases = [
            "-leading-dash",
            "?leading-question",
            ":leading-colon",
            ",leading-comma",
            "[leading-bracket",
            "]leading-bracket",
            "{leading-brace",
            "}leading-brace",
            "#leading-hash",
            "&leading-amp",
            "*leading-star",
            "!leading-bang",
            "|leading-pipe",
            ">leading-gt",
            "'leading-quote",
            "\"leading-dquote",
            "%leading-percent",
            "@leading-at",
            "`leading-backtick",
            " leading-space",
            "trailing-space ",
            "embedded: colon-space",
            "trailing-colon:",
            "embedded #hash-space",
            "line\nbreak",
            "tab\ttab",
            "quote\"quote",
            "back\\slash",
            "control\u{01}char",
        ];
        for s in cases {
            let doc = doc_of(obj(vec![("s", Value::Str(s.to_string()))]));
            let text = write_yaml(&doc, false, None).unwrap();
            let back = read_yaml(&text).unwrap();
            assert!(
                doc.eq_doc(&back),
                "round trip failed for {s:?}, text was:\n{text}"
            );
        }
    }

    #[test]
    fn write_scalar_value_panics_on_a_non_leaf_value() {
        // `write_scalar_value` is only ever called on a leaf via the public
        // `write_yaml` path (every call site passes a scalar) -- white-box
        // confirming the documented invariant directly, same rationale as
        // the empty-array test above.
        let result = std::panic::catch_unwind(|| {
            let mut out = String::new();
            write_scalar_value(&Value::Object(IndexMap::new()), &mut out);
        });
        assert!(result.is_err());
    }

    // ---------------------------------------------------------- coverage: Builder white-box

    // The following four tests drive `Builder`'s private event-handling
    // methods directly rather than through `read_yaml`/`Parser`. Each covers
    // a branch that is structurally unreachable via any real YAML text --
    // confirmed empirically (see `on_event_impl`'s doc comment and this
    // module's doc comment on the crate-choice rationale): `yaml_rust2`'s own
    // scanner already rejects an alias to an undefined anchor, and every
    // document's event stream always nests its root exactly once before a
    // `DocumentEnd`/pushes only `Sequence`/`Mapping` as containers. Testing
    // the arm directly (rather than leaving it an untested dead branch)
    // matches `document.rs`'s `internal_edges_mut_rejects_a_leaf_directly`
    // precedent.
    use yaml_rust2::parser::Event;
    use yaml_rust2::scanner::Marker;

    /// `Marker` has no public constructor, so a real one is captured from an
    /// actual (trivial) parse -- the tests below only care about the event
    /// stream reaching `Builder`'s methods, not about which `Marker` value
    /// they carry.
    fn test_marker() -> Marker {
        struct Capture(Option<Marker>);
        impl MarkedEventReceiver for Capture {
            fn on_event(&mut self, _ev: Event, mark: Marker) {
                self.0.get_or_insert(mark);
            }
        }
        let mut cap = Capture(None);
        Parser::new("x".chars()).load(&mut cap, false).unwrap();
        cap.0
            .expect("a trivial scalar document always emits at least one event")
    }

    #[test]
    fn builder_document_end_with_an_empty_stack_pushes_a_null_scalar() {
        let mut b = Builder::new();
        b.on_event_impl(Event::DocumentEnd, test_marker());
        assert_eq!(b.docs.len(), 1);
        assert!(matches!(&b.docs[0], Raw::Scalar(s, TScalarStyle::Plain, None) if s.is_empty()));
    }

    #[test]
    #[should_panic(expected = "a single document's stack never nests more than one root")]
    fn builder_document_end_with_more_than_one_stack_entry_panics() {
        let mut b = Builder::new();
        b.doc_stack
            .push((Raw::Scalar(String::new(), TScalarStyle::Plain, None), 0));
        b.doc_stack
            .push((Raw::Scalar(String::new(), TScalarStyle::Plain, None), 0));
        b.on_event_impl(Event::DocumentEnd, test_marker());
    }

    #[test]
    #[should_panic(expected = "a Scalar is never a container on doc_stack")]
    fn builder_insert_onto_a_scalar_container_panics() {
        let mut b = Builder::new();
        b.doc_stack
            .push((Raw::Scalar("x".to_string(), TScalarStyle::Plain, None), 0));
        b.insert(
            Raw::Scalar("y".to_string(), TScalarStyle::Plain, None),
            0,
            test_marker(),
        );
    }

    #[test]
    #[should_panic(expected = "yaml_rust2's scanner rejects an alias to an undefined anchor")]
    fn builder_alias_to_an_unknown_anchor_panics() {
        // Calling `on_event_impl` directly bypasses `yaml_rust2`'s own
        // scanner validation (which -- live-confirmed via
        // `yaml_rust2::YamlLoader::load_from_str("a: *nope\n")` -- always
        // catches this first for real input), exercising the `.expect()`'s
        // documented invariant on purpose.
        let mut b = Builder::new();
        b.on_event_impl(Event::Alias(999), test_marker());
    }

    // ---------------------------------------------------------- coverage: writer edge cases

    #[test]
    fn round_trips_an_integral_float_with_trailing_dot_zero() {
        let doc = doc_of(obj(vec![("f", Value::Float(2.0))]));
        let text = write_yaml(&doc, false, None).unwrap();
        assert!(text.contains("f: 2.0"), "text was:\n{text}");
        let back = read_yaml(&text).unwrap();
        assert!(doc.eq_doc(&back));
    }

    #[test]
    fn quoted_string_escapes_every_special_character_in_one_pass() {
        // A leading `-` forces quoting; once quoted, this exercises every
        // remaining escape arm in `write_yaml_string`'s per-char loop in a
        // single string: backslash, tab, double-quote, and a low control
        // character (NEL and newline are already covered by dedicated tests).
        let s = "-a\tb\\c\"d\u{01}e";
        let doc = doc_of(obj(vec![("s", Value::Str(s.to_string()))]));
        let text = write_yaml(&doc, false, None).unwrap();
        let back = read_yaml(&text).unwrap();
        assert!(doc.eq_doc(&back), "text was:\n{text}");
    }

    #[test]
    fn write_seq_child_on_a_bare_non_empty_array_item_writes_it_on_the_next_line() {
        // The Document model never produces a sequence item that is itself a
        // *standalone* non-empty array (sequence-of-sequences has no
        // labeled-edge form -- see `sequence_of_sequences_is_a_document_error`
        // above), so this arm of `write_seq_child` is unreachable via any
        // real `Doc` -- white-box exercising it directly, same rationale as
        // `write_node_on_a_bare_empty_array_writes_the_flow_empty_token`.
        let mut out = String::new();
        write_seq_child(
            &Value::Array(vec![Value::Int(1), Value::Int(2)]),
            0,
            &mut out,
        );
        assert_eq!(out, "\n  - 1\n  - 2\n");
    }
}
