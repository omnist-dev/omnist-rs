# XML

`omnist::formats::xml::{read_xml, read_xml_with_schema, write_xml, check_xml}`. Ported from
`~/dev/omnist/omnist/formats.py`'s `read_xml`/`write_xml`/`check_xml`; see
[`omnist/src/formats/xml.rs`](../../omnist/src/formats/xml.rs)'s module doc
for the full detail.

XML needs exactly one document element, so (unlike JSON/YAML/TOML) an
example document must be wrapped under a single top-level element.

```rust
use indexmap::IndexMap;
use omnist::document::{Doc, Value};
use omnist::formats::xml::{read_xml, write_xml};

// `age` is a `Value::Str`, not `Value::Int`: XML text carries no type
// information (see this doc's "Text stays untyped" note below) -- keeping
// it a string in the first place is what makes this round trip lossless.
let mut fields = IndexMap::new();
fields.insert("name".to_string(), Value::Str("Ada".to_string()));
fields.insert("age".to_string(), Value::Str("37".to_string()));
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
XML entities are recognized) -- XXE-safe by construction, not by
configuration (Python's `read_xml` needs `defusedxml` instead of the stdlib
`ElementTree` for the same protection).

## The data-XML profile: what the reader refuses

`read_xml` reads a deliberately narrow subset (omnist-spec's
[data-XML profile](https://spec.omnist.dev/formats/xml/#the-data-xml-profile))
and refuses the rest, rather than ignoring it. Each refusal is a
`DocumentError` at path `$` whose `code` is:

| Construct | `code` |
|---|---|
| any `DOCTYPE` declaration (refused on sight, even if nothing uses it) | `format.dtd-forbidden` |
| an entity reference other than the five predefined ones | `format.entity-forbidden` |
| text alongside child elements | `format.mixed-content` |

These are refusals, not syntax errors: the input is well-formed XML. The
refusal is raised only after the whole document has proved well-formed, so
malformed XML is always `parse.codec-syntax` (never misreported as a
refusal). Numeric character references and the five predefined entities stay
legal; comments, CDATA sections and processing instructions are inert.

## Illegal characters fail the write

XML 1.0 cannot represent a raw C0 control character other than tab, LF and CR
(nor U+FFFE/U+FFFF). A string holding one fails `write_xml` with
`write.unsupported-value` at that string's path, strict or not; it is no
longer replaced with U+FFFD.

## Namespaces: a disclosed simplification

`quick_xml` runs in non-namespace-aware mode here; a namespaced tag's local
name is taken by stripping a lexical `prefix:` up to the last `:`, which
coincides with Python's `ElementTree`-based behavior for the common case
(a declared, in-scope prefix) but does not resolve prefixes through
`xmlns` declarations. Namespaces are outside this issue's spec.

## Text stays untyped until materialize

XML's grammar carries no type information -- `<m>1</m>` and `<m>hi</m>` are
syntactically identical, a bare text node. `read_xml` builds every leaf as
a plain string unconditionally; no int/float/bool inference happens at
parse time.

Writing a non-string scalar (`bool`/`int`/`float`) to XML now honestly
reports it: XML has no native typed literals, so it reads back as a
string, not its original type (`check_xml`'s `value.stringified`
adjustment).

## Schema-guided pretyping (spec §2.2 / issue #114)

Because XML text is untyped and `materialize` strictly rejects coercing plain strings to `boolean`/`integer`/`number`, [`read_xml_with_schema`] performs schema-guided pretyping on the raw XML tree before materialization (spec §2.2 / issue #114).

```rust
use omnist::formats::xml::read_xml_with_schema;
use omnist::osd::parse_schema;

let schema_osd = r#"
record Address  { "street": string, "city": string }
record LineItem { "sku": string, "qty": integer, "price": number }

record Order {
    "id":           string,
    "status":       string,
    "total":        number,
    "address":      Address,
    "items" [1,]:   LineItem,
    "coupon" [0,1]: string,
}

record Root { "order": Order }
root Root
"#;
let schema = parse_schema(schema_osd).unwrap();

let xml = r#"<order>
  <id>A1</id>
  <status>shipped</status>
  <total>29.97</total>
  <address><street>1 Main</street><city>London</city></address>
  <items><sku>W</sku><qty>3</qty><price>9.99</price></items>
  <items><sku>G</sku><qty>1</qty><price>9.99</price></items>
</order>"#;

let doc = read_xml_with_schema(xml, &schema).unwrap();
assert!(schema.validate(&doc.root()).ok());
```
<!-- doc-illustrative -->
