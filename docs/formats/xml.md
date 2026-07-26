# XML

`omnist::formats::xml::{read_xml, write_xml, check_xml}`. Ported from
`~/dev/omnist/omnist/formats.py`'s `read_xml`/`write_xml`/`check_xml`; see
[`omnist/src/formats/xml.rs`](../../omnist/src/formats/xml.rs)'s module doc
for the full detail.

XML needs exactly one document element, so (unlike JSON/YAML/TOML) an
example document must be wrapped under a single top-level element.

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::xml::{read_xml, write_xml};

let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ada".to_string()));
fields.insert("age".to_string(), Value::Int(37));
let mut root = IndexMap::new();
root.insert("person".to_string(), Value::Object(fields));
let doc = Doc::of(&Value::Object(root)).unwrap();

let text = write_xml(&doc, true, None).unwrap();
let doc2 = read_xml(&text).unwrap();
assert!(doc.eq_doc(&doc2));
```
<!-- verified-by: omnist/tests/examples.rs::xml_roundtrip -->

## Structural difference: interleaving-preserving, not grouped

Unlike the other three codecs, this module goes through `Doc::from_raw`/
`Doc::to_raw` (`RawNode`), not `Doc::to_grouped` -- XML element order can
interleave distinct labels arbitrarily (`<b/><c/><b/>`), which a JSON-shaped
`IndexMap` can't represent.

## `quick-xml`: no advisory, no DTD/entity-expansion at all

`cargo audit` against this crate's full dependency tree (29 crates,
including `quick-xml` 0.41.0) on 2026-07-26 against the RustSec advisory
database found **zero** matches -- unlike TS's port, which carried an
unfixable `fast-xml-parser` advisory (omnist-ts#38). `quick-xml` also has
no DTD/external-entity expansion support at all (only the five predefined
XML entities are recognized; an undefined entity is a parse error) -- XXE
-safe by construction, not by configuration (Python's `read_xml` needs
`defusedxml` instead of the stdlib `ElementTree` for the same protection).

## Namespaces: a disclosed simplification

`quick_xml` runs in non-namespace-aware mode here; a namespaced tag's local
name is taken by stripping a lexical `prefix:` up to the last `:`, which
coincides with Python's `ElementTree`-based behavior for the common case
(a declared, in-scope prefix) but does not resolve prefixes through
`xmlns` declarations. Namespaces are outside this issue's spec.

## Scalar coercion, live-checked against Python's `_coerce`

Confirmed against a live Python interpreter, not assumed:

- The empty string reads as `""`, never coerced.
- A case-insensitive, **whole-string, untrimmed** match against
  `"true"`/`"false"` reads as a bool -- `" true "` (with surrounding
  whitespace) stays a string.
- Otherwise `int(text)` is tried, then `float(text)`, both after
  Python-style trimming and accepting a single `_` between two digits as a
  group separator (`"1_0"` -> `10`); text that matches neither stays a
  string.
- `float(text)` also accepts `inf`/`infinity`/`nan` (any case, optionally
  signed).
- **Disclosed representational gap**: this port's `Scalar::Int` is
  `i64`-backed (max ~19 digits); Python's `int` is arbitrary-precision.
  Text between 20 and 4300 digits that Python holds as an exact `int`
  falls through here to `Scalar::Float` instead (the same int-then-float
  fallback order Python's `_coerce` itself uses, not a new divergence).
- **Not replicated**: Python's `int()`/`float()` accept any Unicode decimal
  digit (e.g. Arabic-Indic digits); this module's parsers are
  ASCII-digit-only -- a disclosed, narrower-than-Python simplification.
